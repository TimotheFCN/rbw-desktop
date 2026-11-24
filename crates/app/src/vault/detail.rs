//! The selected item: every field copies on click, hidden ones reveal on
//! request, the verification code counts down live.

use std::time::{Duration, SystemTime};

use gpui::{
    AnyElement, AppContext as _, Context, ElementId, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, SharedString, Styled as _, Subscription, Task,
    WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use rbw_desktop_core::{
    AccountId, Error, FieldValue, Item, ItemContent, ItemId, ItemKind, ItemSummary, Secret,
    VerificationCode,
};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Look, button, card, chrome, field_row, icon, motion,
    password_text, progress_ring, scroll_area, tile,
};

use super::{EditItem, Part, VaultView};
use crate::backend::{self, Load};
use crate::clipboard;
use crate::icons::{self, Icons};
use crate::recent;
use crate::reprompt::{Reprompt, Unlocked};
use crate::secondary;

/// How long a selection moved with the keys rests before its item is
/// fetched.
const SETTLE: Duration = Duration::from_millis(100);
/// How often a verification code's countdown moves.
const COUNTDOWN_STEP: Duration = Duration::from_secs(1);
/// How a hidden value shows: always as long, so that its length says
/// nothing and nothing moves when it arrives.
const MASK: &str = "••••••••••";
/// The narrowest the item gets, beside the list.
const MIN_WIDTH: Pixels = px(220.);
/// The widest the item's fields get: in a wide window they stay in the
/// middle rather than stretch across it.
const MAX_CONTENT_WIDTH: Pixels = px(620.);

pub struct ItemDetail {
    account: AccountId,
    vault: WeakEntity<VaultView>,
    /// The item chosen in the list, as the list shows it.
    selected: Option<ItemSummary>,
    /// The last answer for an item, with the item it answers for. Until
    /// the selected one's arrives, the pane shows what the list knows of it.
    loaded: Option<(ItemId, Load<Item>)>,
    revealed: Vec<SharedString>,
    /// Something asked to be copied before the item arrived, copied once it
    /// does.
    pending_copy: Option<Part>,
    copied: Option<SharedString>,
    code: Option<VerificationCode>,
    /// The code shown expired: the next one is fetched once the pane is
    /// drawn again, so that a hidden window asks rbw for nothing.
    code_expired: bool,
    /// Asks for the master password, for an item that wants it again.
    reprompt: Option<Entity<Reprompt>>,
    /// Whether the password field takes the keyboard when it appears: when
    /// the item was clicked rather than stepped to with the arrow keys.
    focus_reprompt: bool,
    _reprompt_events: Option<Subscription>,
    load_task: Task<()>,
    copied_timer: Task<()>,
    code_task: Task<()>,
    _icons: Subscription,
}

/// How a field's value is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shown {
    Text,
    Hidden,
    /// Hidden until revealed, then shown with its digits and symbols
    /// coloured.
    Password,
    Link,
    Mono,
}

struct Field {
    key: SharedString,
    label: SharedString,
    value: Secret,
    shown: Shown,
    /// For a value not yet fetched: what a click copies once it arrives.
    pending: Option<Part>,
}

impl Field {
    fn new(
        key: impl Into<SharedString>,
        label: impl Into<SharedString>,
        value: impl Into<Secret>,
        shown: Shown,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value: value.into(),
            shown,
            pending: None,
        }
    }

    /// A field whose value comes with the item, such as a login's password
    /// while the list only knows the login's name.
    fn pending(key: &'static str, label: &'static str, shown: Shown, part: Part) -> Self {
        Self {
            pending: Some(part),
            ..Self::new(key, label, Secret::default(), shown)
        }
    }
}

impl ItemDetail {
    pub fn new(account: AccountId, vault: WeakEntity<VaultView>, cx: &mut Context<Self>) -> Self {
        Self {
            account,
            vault,
            selected: None,
            loaded: None,
            revealed: vec![],
            pending_copy: None,
            copied: None,
            code: None,
            code_expired: false,
            reprompt: None,
            focus_reprompt: false,
            _reprompt_events: None,
            load_task: Task::ready(()),
            copied_timer: Task::ready(()),
            code_task: Task::ready(()),
            _icons: cx.observe(&Icons::global(cx), |_, _, cx| cx.notify()),
        }
    }

    fn selected_id(&self) -> Option<&ItemId> {
        self.selected.as_ref().map(|item| &item.id)
    }

    /// The item chosen in the list, as the list shows it.
    pub fn selected(&self) -> Option<&ItemSummary> {
        self.selected.as_ref()
    }

    /// The answer for the selected item, once it has come.
    fn loaded(&self) -> Option<&Load<Item>> {
        let (id, load) = self.loaded.as_ref()?;
        (Some(id) == self.selected_id()).then_some(load)
    }

    /// The selected item, once it has arrived.
    pub fn item(&self) -> Option<&Item> {
        self.loaded()?.ready()
    }

    /// Takes another item, forgetting what was revealed or copied of the
    /// one before.
    fn select(&mut self, item: Option<ItemSummary>) {
        if item.as_ref().map(|item| &item.id) != self.selected_id() {
            self.forget_item_state();
        }
        self.selected = item;
    }

    /// Shows another item, or none. `by_pointer` says it was clicked, so
    /// that a master password it asks for takes the keyboard at once.
    pub fn show(&mut self, item: Option<ItemSummary>, by_pointer: bool, cx: &mut Context<Self>) {
        self.select(item);
        self.focus_reprompt = by_pointer;
        self.reload(cx);
    }

    /// Shows another item once the selection rests: one only passed over,
    /// with the arrow keys or while a search is typed, isn't fetched.
    pub fn show_when_settled(&mut self, item: Option<ItemSummary>, cx: &mut Context<Self>) {
        if item.is_none() {
            self.show(None, false, cx);
            return;
        }
        self.select(item);
        self.focus_reprompt = false;
        self.load_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            this.update(cx, |this, cx| this.reload(cx)).ok();
        });
        cx.notify();
    }

    /// Gives the keyboard to the master password prompt, if one shows.
    pub fn focus_reprompt(&mut self, cx: &mut Context<Self>) {
        self.focus_reprompt = true;
        cx.notify();
    }

    /// The selected item's latest summary, after the list reloaded. An
    /// item selected before the list knew it, such as one just created,
    /// shows now.
    pub fn set_summary(&mut self, item: ItemSummary, cx: &mut Context<Self>) {
        if self.selected_id() != Some(&item.id) {
            self.show(Some(item), false, cx);
        } else if self.selected.as_ref() != Some(&item) {
            self.selected = Some(item);
            cx.notify();
        }
    }

    fn forget_item_state(&mut self) {
        self.revealed.clear();
        self.pending_copy = None;
        self.copied = None;
        self.code = None;
        self.code_expired = false;
        self.code_task = Task::ready(());
    }

    /// Fetches the selected item again, after it may have changed.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_id().cloned() else {
            cx.notify();
            return;
        };
        let (account, requested) = (self.account.clone(), id.clone());
        self.load_task = backend::request(
            cx,
            move |backend| backend.item(&account, &requested),
            move |this, result, cx| {
                let has_code = matches!(&result, Ok(item) if has_code(item));
                this.loaded = Some((id.clone(), result.into()));
                if has_code {
                    this.fetch_code(id, cx);
                }
                // A code waits for its own answer.
                if let Some(part) = this.pending_copy.filter(|part| *part != Part::Code) {
                    this.pending_copy = None;
                    this.copy_part(part, cx);
                }
                cx.notify();
            },
        );
    }

    fn fetch_code(&mut self, id: ItemId, cx: &mut Context<Self>) {
        let (account, requested) = (self.account.clone(), id.clone());
        self.code_expired = false;
        self.code_task = backend::request(
            cx,
            move |backend| backend.verification_code(&account, &requested),
            move |this, result, cx| {
                // Another item took its place meanwhile.
                if this.selected_id() != Some(&id) {
                    return;
                }
                let code = match result {
                    Ok(code) => code,
                    // The item went unread long enough to lock again.
                    Err(Error::RepromptRequired) => {
                        this.loaded = Some((id, Load::Failed(Error::RepromptRequired)));
                        this.forget_item_state();
                        cx.notify();
                        return;
                    }
                    Err(_) => return,
                };
                let expires_at = code.expires_at;
                this.code = Some(code);
                // Redraw each second for the countdown, which a hidden window
                // doesn't draw, then ask for the next code when drawn.
                this.code_task = cx.spawn(async move |this, cx| {
                    while SystemTime::now() < expires_at {
                        cx.background_executor().timer(COUNTDOWN_STEP).await;
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            return;
                        }
                    }
                    this.update(cx, |this, cx| {
                        this.code_expired = true;
                        cx.notify();
                    })
                    .ok();
                });
                if this.pending_copy == Some(Part::Code) {
                    this.pending_copy = None;
                    this.copy_part(Part::Code, cx);
                }
                cx.notify();
            },
        );
    }

    fn copy(&mut self, key: SharedString, value: &Secret, secret: bool, cx: &mut Context<Self>) {
        if secret {
            clipboard::copy_secret(value, Some(&self.account), cx);
        } else {
            clipboard::copy_text(value.expose(), cx);
        }
        if let Some(item) = self.selected_id() {
            recent::used(&self.account, item, cx);
        }
        self.copied = Some(key);
        self.copied_timer = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(motion::CONFIRMATION_HOLD)
                .await;
            this.update(cx, |this, cx| {
                this.copied = None;
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    /// Copies the password, username or code of the login shown, as if its
    /// row was clicked: once it has arrived, if it is on the way; for a
    /// protected one, the master password field takes the keyboard.
    pub fn copy_part(&mut self, part: Part, cx: &mut Context<Self>) {
        let has_code = self.selected.as_ref().is_some_and(|item| item.has_code);
        let item = match self.loaded() {
            Some(Load::Ready(item)) => item,
            Some(Load::Failed(Error::RepromptRequired)) => {
                self.focus_reprompt(cx);
                return;
            }
            Some(Load::Failed(_)) => return,
            Some(Load::Loading) | None => {
                self.pending_copy = Some(part);
                self.reload(cx);
                return;
            }
        };
        let ItemContent::Login(login) = &item.data.content else {
            return;
        };
        let (key, value, secret) = match part {
            Part::Username => ("username", Secret::new(login.username.clone()), false),
            Part::Password => ("password", login.password.clone(), true),
            Part::Code => match &self.code {
                Some(code) => ("code", code.code.clone(), true),
                None => {
                    if has_code {
                        self.pending_copy = Some(Part::Code);
                    }
                    return;
                }
            },
        };
        if !value.is_empty() {
            self.copy(key.into(), &value, secret, cx);
        }
    }

    fn toggle_reveal(&mut self, key: &SharedString, cx: &mut Context<Self>) {
        if let Some(index) = self.revealed.iter().position(|k| k == key) {
            self.revealed.remove(index);
        } else {
            self.revealed.push(key.clone());
        }
        cx.notify();
    }

    /// The prompt for the master password, for the selected item, made when
    /// it is first needed.
    fn reprompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Reprompt>> {
        let id = self.selected_id()?.clone();
        let reprompt = match &self.reprompt {
            Some(reprompt) if *reprompt.read(cx).item() == id => reprompt.clone(),
            _ => {
                let account = self.account.clone();
                let reprompt = cx.new(|cx| Reprompt::new(account, id, window, cx));
                self._reprompt_events = Some(cx.subscribe_in(
                    &reprompt,
                    window,
                    |this, _, _: &Unlocked, window, cx| {
                        this.reprompt = None;
                        this.reload(cx);
                        // The field that had the keyboard goes with the prompt.
                        this.vault
                            .update(cx, |vault, cx| vault.focus_list(window, cx))
                            .ok();
                    },
                ));
                self.reprompt = Some(reprompt.clone());
                reprompt
            }
        };
        if std::mem::take(&mut self.focus_reprompt) {
            reprompt.update(cx, |reprompt, cx| reprompt.focus(window, cx));
        }
        Some(reprompt)
    }

    fn toggle_favorite(&mut self, cx: &mut Context<Self>) {
        let Some(item) = &self.selected else { return };
        let (account, id, favorite) = (self.account.clone(), item.id.clone(), !item.favorite);
        let vault = self.vault.clone();
        backend::request(
            cx,
            move |backend| backend.set_favorite(&account, &id, favorite),
            move |this, result, cx| match result {
                Ok(()) => {
                    this.reload(cx);
                    vault.update(cx, |vault, cx| vault.reload(cx)).ok();
                }
                Err(error) => {
                    vault
                        .update(cx, |vault, cx| vault.show_error(&error, cx))
                        .ok();
                }
            },
        )
        .detach();
    }

    fn render_toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let style = match theme.look {
            Look::MacOs => ButtonStyle::Normal,
            Look::Gnome => ButtonStyle::Flat,
        };
        // From the list's summary, which changes with the selection: the
        // buttons never wait for the item.
        let editable = self.selected.as_ref().is_some_and(|item| item.editable);
        // Favourites belong to the account: any item can be one, even
        // before it is unlocked.
        let favorite = self.selected.as_ref().map(|item| item.favorite);
        let accent = theme.colors.accent_text;
        let height = theme.metrics.toolbar_height;
        chrome::drag_region(
            "detail-toolbar",
            div()
                .h(height)
                .flex_none()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .px(px(12.))
                .when_some(favorite, |bar, favorite| {
                    bar.child(
                        div().when(favorite, |d| d.text_color(accent)).child(
                            button("favorite")
                                .icon(IconName::Star)
                                .style(style)
                                .accessibility_label(if favorite {
                                    "Remove from Favorites"
                                } else {
                                    "Add to Favorites"
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_favorite(cx))),
                        ),
                    )
                })
                .when(editable, |bar| {
                    bar.child(
                        button("edit")
                            .label("Edit")
                            .style(ButtonStyle::Normal)
                            .shortcut(secondary!("e"))
                            // An action, not a call: the vault reads this view
                            // to edit the item, and this view is mid-update.
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(EditItem), cx)
                            }),
                    )
                })
                .children(chrome::window_buttons(chrome::Side::End, window, cx)),
            window,
            cx,
        )
        .into_any_element()
    }

    fn render_field(&self, field: Field, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let revealed = self.revealed.contains(&field.key);
        let hidden = matches!(field.shown, Shown::Hidden | Shown::Password);
        let value: AnyElement = match field.shown {
            Shown::Hidden | Shown::Password if !revealed => div()
                .text_size(px(15.))
                .line_height(px(18.))
                .whitespace_nowrap()
                .overflow_hidden()
                .child(MASK)
                .into_any_element(),
            Shown::Link => div()
                .truncate()
                .text_color(colors.accent_text)
                .child(website_host(field.value.expose()))
                .into_any_element(),
            Shown::Password => div()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(12.))
                .truncate()
                .child(password_text(field.value.expose(), cx))
                .into_any_element(),
            Shown::Mono | Shown::Hidden => div()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(12.))
                .truncate()
                .child(field.value.expose().to_string())
                .into_any_element(),
            Shown::Text => div()
                .child(field.value.expose().to_string())
                .into_any_element(),
        };
        let key = field.key.clone();
        let copied = self.copied.as_ref() == Some(&field.key);
        let value_for_copy = field.value.clone();
        let spoken = match field.shown {
            Shown::Hidden | Shown::Password if !revealed => None,
            Shown::Link => Some(website_host(field.value.expose())),
            _ => Some(field.value.expose().to_string()),
        };
        let mut row = field_row(
            ElementId::Name(field.key.clone()),
            field.label.clone(),
            value,
        )
        .copied(copied)
        .when_some(spoken, |row, spoken| row.spoken_value(spoken))
        .on_click(cx.listener(move |this, _, _, cx| match field.pending {
            Some(part) => this.copy_part(part, cx),
            None => this.copy(key.clone(), &value_for_copy, hidden, cx),
        }));
        if hidden {
            let key = field.key.clone();
            let label = if revealed {
                format!("Hide {}", field.label)
            } else {
                format!("Show {}", field.label)
            };
            row = row.action(
                div()
                    .when(revealed, |d| d.text_color(colors.accent_text))
                    .child(
                        row_button(format!("reveal-{}", field.key), IconName::Eye, label).on_click(
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_reveal(&key, cx);
                            }),
                        ),
                    ),
            );
        }
        let url = (field.shown == Shown::Link)
            .then(|| website_url(field.value.expose()))
            .flatten();
        if let Some(url) = url {
            row = row.action(
                row_button(
                    format!("open-{}", field.key),
                    IconName::External,
                    "Open Website",
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    cx.open_url(&url);
                    if let Some(item) = this.selected_id() {
                        recent::used(&this.account, item, cx);
                    }
                })),
            );
        } else if hidden {
            row = row.action(icon(IconName::Copy));
        }
        row.into_any_element()
    }

    /// In place of an item that asks for the master password again: what it
    /// is, and the field that unlocks it.
    fn render_reprompt(
        &self,
        item: &ItemSummary,
        reprompt: Entity<Reprompt>,
        icon: rbw_desktop_ui::TileIcon,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let dim = cx.theme().colors.text_secondary;
        motion::content_enter(
            ElementId::Name(format!("reprompt-{}", item.id.0).into()),
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .pb(px(48.))
                .child(
                    div()
                        .w(px(320.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .text_center()
                        .child(
                            tile(&item.name)
                                .size(px(56.))
                                .icon(ElementId::Name(format!("tile-{}", item.id.0).into()), icon),
                        )
                        .child(
                            div()
                                .mt(px(12.))
                                .w_full()
                                .truncate()
                                .text_size(px(17.))
                                .font_weight(FontWeight::BOLD)
                                .child(item.name.clone()),
                        )
                        .child(
                            div()
                                .mt(px(3.))
                                .mb(px(16.))
                                .text_color(dim)
                                .child("Enter your master password to see this item."),
                        )
                        .child(div().w(px(270.)).child(reprompt)),
                ),
        )
        .into_any_element()
    }

    /// The verification code's row, counting down; while the code is on
    /// the way, its place.
    fn render_code(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let has_code = self.selected.as_ref()?.has_code;
        let theme = cx.theme();
        let tertiary = theme.colors.text_tertiary;
        let key: SharedString = "code".into();
        let copied = self.copied.as_ref() == Some(&key);
        let row = |digits: AnyElement, fraction: f32, cx: &mut Context<Self>| {
            field_row(
                "code",
                "Verification code",
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(digits)
                    .child(progress_ring(fraction, cx)),
            )
            .copied(copied)
            .action(icon(IconName::Copy).text_color(tertiary))
        };
        let Some(code) = &self.code else {
            return has_code.then(|| {
                row(
                    div()
                        .text_color(tertiary)
                        .child("––– –––")
                        .into_any_element(),
                    0.,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.copy_part(Part::Code, cx)))
                .into_any_element()
            });
        };
        let remaining = code
            .expires_at
            .duration_since(SystemTime::now())
            .unwrap_or_default();
        let fraction = remaining.as_secs_f32() / code.period.as_secs_f32().max(1.);
        let digits = code.code.expose();
        let spaced = if digits.len() == 6 {
            format!("{} {}", &digits[..3], &digits[3..])
        } else {
            digits.to_string()
        };
        let value = code.code.clone();
        // Each new code fades up in place.
        let period = code
            .expires_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let shown = motion::refresh(("code", period as usize), div().child(spaced.clone()))
            .into_any_element();
        Some(
            row(shown, fraction, cx)
                .spoken_value(spaced)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.copy(key.clone(), &value, true, cx)),
                )
                .into_any_element(),
        )
    }
}

fn has_code(item: &Item) -> bool {
    matches!(&item.data.content, ItemContent::Login(login) if !login.totp.is_empty())
}

/// The web address to open for a website, `https://` when it has none.
/// `None` for an address the browser wouldn't open, such as `file:`,
/// `smb:` or another app's: the system would hand it to that program, and
/// an item shared by an organization can be changed by its other members.
fn website_url(website: &str) -> Option<String> {
    let website = website.trim();
    match website.split_once("://") {
        Some((scheme, rest))
            if !rest.is_empty()
                && (scheme.eq_ignore_ascii_case("https")
                    || scheme.eq_ignore_ascii_case("http")) =>
        {
            Some(website.to_string())
        }
        Some(_) => None,
        None if website.is_empty() || website.contains(char::is_whitespace) => None,
        None => Some(format!("https://{website}")),
    }
}

/// What the item view shows of a website: its host, as a browser's address
/// bar does, or the whole address when it isn't a web one.
fn website_host(website: &str) -> String {
    let Some(url) = website_url(website) else {
        return website.to_string();
    };
    let rest = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = authority.rsplit('@').next().unwrap_or(authority);
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

/// A small button at the end of a field's row, such as the eye.
fn row_button(
    id: String,
    name: IconName,
    label: impl Into<SharedString>,
) -> rbw_desktop_ui::Button {
    button(SharedString::from(id))
        .icon(name)
        .style(ButtonStyle::Flat)
        .size(px(24.))
        .accessibility_label(label)
}

fn kind_label(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Login => "Login",
        ItemKind::SecureNote => "Note",
        ItemKind::Card => "Card",
        ItemKind::Identity => "Identity",
        ItemKind::SshKey => "SSH Key",
    }
}

/// The item's fields, grouped into the cards they are shown in. The
/// verification code goes in the first card, after the fields listed here.
fn cards(item: &Item) -> Vec<Vec<Field>> {
    let non_empty = |fields: Vec<Field>| -> Vec<Field> {
        fields
            .into_iter()
            .filter(|field| !field.value.is_empty())
            .collect()
    };
    let mut cards = vec![];
    match &item.data.content {
        ItemContent::Login(login) => {
            cards.push(non_empty(vec![
                Field::new("username", "Username", login.username.as_str(), Shown::Text),
                Field::new(
                    "password",
                    "Password",
                    login.password.clone(),
                    Shown::Password,
                ),
            ]));
            cards.push(
                login
                    .websites
                    .iter()
                    .enumerate()
                    .map(|(i, w)| {
                        Field::new(
                            format!("website-{i}"),
                            "Website",
                            w.uri.as_str(),
                            Shown::Link,
                        )
                    })
                    .collect(),
            );
        }
        ItemContent::Card(card) => {
            let expiry = match (card.exp_month.as_str(), card.exp_year.as_str()) {
                ("", "") => String::new(),
                (month, year) => format!("{month:0>2} / {year}"),
            };
            cards.push(non_empty(vec![
                Field::new(
                    "cardholder",
                    "Cardholder",
                    card.cardholder_name.as_str(),
                    Shown::Text,
                ),
                Field::new("number", "Number", card.number.clone(), Shown::Hidden),
                Field::new("expiry", "Expires", expiry, Shown::Text),
                Field::new(
                    "security-code",
                    "Security code",
                    card.code.clone(),
                    Shown::Hidden,
                ),
            ]));
        }
        ItemContent::Identity(identity) => {
            let address = [
                &identity.address1,
                &identity.address2,
                &identity.address3,
                &format!("{} {}", identity.postal_code, identity.city)
                    .trim()
                    .to_string(),
                &identity.state,
                &identity.country,
            ]
            .into_iter()
            .filter(|line| !line.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
            cards.push(non_empty(vec![
                Field::new("name", "Name", identity.full_name(), Shown::Text),
                Field::new("email", "Email", identity.email.as_str(), Shown::Text),
                Field::new("phone", "Phone", identity.phone.as_str(), Shown::Text),
                Field::new("address", "Address", address, Shown::Text),
                Field::new(
                    "identity-username",
                    "Username",
                    identity.username.as_str(),
                    Shown::Text,
                ),
            ]));
            cards.push(non_empty(vec![
                Field::new(
                    "ssn",
                    "Social security number",
                    identity.ssn.clone(),
                    Shown::Hidden,
                ),
                Field::new(
                    "passport",
                    "Passport number",
                    identity.passport_number.clone(),
                    Shown::Hidden,
                ),
                Field::new(
                    "license",
                    "License number",
                    identity.license_number.clone(),
                    Shown::Hidden,
                ),
            ]));
        }
        ItemContent::SshKey(key) => {
            cards.push(non_empty(vec![
                Field::new(
                    "public-key",
                    "Public key",
                    key.public_key.as_str(),
                    Shown::Mono,
                ),
                Field::new(
                    "fingerprint",
                    "Fingerprint",
                    key.fingerprint.as_str(),
                    Shown::Mono,
                ),
                Field::new(
                    "private-key",
                    "Private key",
                    key.private_key.clone(),
                    Shown::Hidden,
                ),
            ]));
        }
        ItemContent::SecureNote => {}
    }
    cards.push(
        item.data
            .fields
            .iter()
            .enumerate()
            .filter_map(|(i, field)| {
                let key = format!("field-{i}");
                let label = field.name.clone();
                match &field.value {
                    FieldValue::Text(text) => {
                        Some(Field::new(key, label, text.as_str(), Shown::Text))
                    }
                    FieldValue::Hidden(secret) => {
                        Some(Field::new(key, label, secret.clone(), Shown::Hidden))
                    }
                    FieldValue::Boolean(on) => Some(Field::new(
                        key,
                        label,
                        if *on { "Yes" } else { "No" },
                        Shown::Text,
                    )),
                    FieldValue::Linked(_) => None,
                }
            })
            .collect(),
    );
    cards.push(non_empty(vec![Field::new(
        "notes",
        "Notes",
        item.data.notes.clone(),
        Shown::Text,
    )]));
    cards
}

impl ItemDetail {
    /// The item's fields in their cards, the verification code in the
    /// first.
    fn render_cards(&self, mut cards: Vec<Vec<Field>>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let code = self.render_code(cx);
        let mut groups = vec![];
        let mut first: Vec<AnyElement> = if cards.is_empty() {
            vec![]
        } else {
            cards
                .remove(0)
                .into_iter()
                .map(|field| self.render_field(field, cx))
                .collect()
        };
        first.extend(code);
        for rows in std::iter::once(first).chain(cards.into_iter().map(|fields| {
            fields
                .into_iter()
                .map(|field| self.render_field(field, cx))
                .collect()
        })) {
            if !rows.is_empty() {
                groups.push(card().rows(rows).into_any_element());
            }
        }
        groups
    }

    /// The selected item: its name from the list at once, then its fields,
    /// those the list knows of while the item is on the way.
    fn render_item(
        &mut self,
        summary: &ItemSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let dim = cx.theme().colors.text_secondary;
        let icon = icons::icon(&self.account, summary, cx);
        // Its cards, or why there are none.
        let fields = match self.loaded() {
            Some(Load::Ready(item)) => Ok(cards(item)),
            Some(Load::Failed(Error::RepromptRequired)) => Err(None),
            Some(Load::Failed(error)) => Err(Some(backend::describe(error))),
            Some(Load::Loading) | None => Ok(provisional_cards(summary)),
        };
        let content: Vec<AnyElement> = match fields {
            Ok(cards) => self.render_cards(cards, cx),
            Err(None) => match self.reprompt(window, cx) {
                Some(reprompt) => return self.render_reprompt(summary, reprompt, icon, cx),
                None => vec![],
            },
            Err(Some(problem)) => vec![
                div()
                    .pt(px(24.))
                    .text_center()
                    .text_color(dim)
                    .child(problem)
                    .into_any_element(),
            ],
        };
        let header = div()
            .flex()
            .items_center()
            .gap(px(13.))
            .mb(px(4.))
            .child(tile(&summary.name).size(px(56.)).icon(
                ElementId::Name(format!("tile-{}", summary.id.0).into()),
                icon,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(20.))
                            .font_weight(FontWeight::BOLD)
                            .child(summary.name.clone()),
                    )
                    .child(div().text_color(dim).child(kind_label(summary.kind))),
            );
        div()
            .flex()
            .justify_center()
            .child(motion::content_enter(
                ElementId::Name(format!("detail-{}", summary.id.0).into()),
                div()
                    .flex_1()
                    .min_w_0()
                    .max_w(MAX_CONTENT_WIDTH)
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .px(px(16.))
                    .pt(px(2.))
                    .pb(px(16.))
                    .child(header)
                    .children(content),
            ))
            .into_any_element()
    }
}

impl Render for ItemDetail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.code_expired
            && let Some(id) = self.item().map(|item| item.id.clone())
        {
            self.fetch_code(id, cx);
        }
        let fill = cx.theme().colors.window;
        let toolbar = self.render_toolbar(window, cx);
        let corner = chrome::corner_radius(window, cx);
        let body = self
            .selected
            .clone()
            .map(|summary| self.render_item(&summary, window, cx));
        div()
            .key_context("ItemDetail")
            .flex_1()
            .min_w(MIN_WIDTH)
            .h_full()
            .flex()
            .flex_col()
            .bg(fill)
            .rounded_r(corner)
            .child(toolbar)
            .child(
                scroll_area("detail-body")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .children(body),
            )
    }
}

/// What the list knows of a login's fields, shown while the item is on the
/// way: its username, a password to come, and its websites. Other kinds
/// show their fields once they arrive.
fn provisional_cards(summary: &ItemSummary) -> Vec<Vec<Field>> {
    if summary.kind != ItemKind::Login {
        return vec![];
    }
    let username = summary
        .subtitle
        .as_ref()
        .map(|name| Field::new("username", "Username", name.as_str(), Shown::Text));
    let password = Field::pending("password", "Password", Shown::Password, Part::Password);
    let websites = summary
        .websites
        .iter()
        .enumerate()
        .map(|(i, website)| {
            Field::new(
                format!("website-{i}"),
                "Website",
                website.as_str(),
                Shown::Link,
            )
        })
        .collect();
    vec![username.into_iter().chain([password]).collect(), websites]
}

#[cfg(test)]
mod tests {
    use rbw_desktop_core::{Identity, ItemData, Login};

    use super::*;

    fn item(content: ItemContent) -> Item {
        Item {
            id: ItemId("a".into()),
            data: ItemData {
                content,
                ..ItemData::new(ItemKind::SecureNote)
            },
            organization: None,
            favorite: false,
            editable: true,
            reprompt: false,
        }
    }

    fn keys(cards: &[Vec<Field>]) -> Vec<Vec<&str>> {
        cards
            .iter()
            .map(|fields| fields.iter().map(|field| field.key.as_ref()).collect())
            .collect()
    }

    #[test]
    fn empty_fields_are_left_out() {
        let login = item(ItemContent::Login(Login {
            password: Secret::new("hunter2"),
            ..Login::default()
        }));
        assert_eq!(
            keys(&cards(&login)),
            [vec!["password"], vec![], vec![], vec![]]
        );
        let identity = item(ItemContent::Identity(Identity {
            first_name: "Alex".into(),
            city: "Lyon".into(),
            country: "France".into(),
            ..Identity::default()
        }));
        let cards = cards(&identity);
        assert_eq!(keys(&cards)[0], ["name", "address"]);
        assert_eq!(cards[0][1].value.expose(), "Lyon, France");
    }

    #[test]
    fn a_login_shows_what_the_list_knows_while_it_loads() {
        let summary = ItemSummary {
            id: ItemId("a".into()),
            kind: ItemKind::Login,
            name: "Bank".into(),
            subtitle: Some("alex".into()),
            websites: vec!["bank.example".into()],
            folder: None,
            organization: None,
            favorite: false,
            has_code: false,
            reprompt: false,
            editable: true,
            modified: None,
        };
        let cards = provisional_cards(&summary);
        assert_eq!(
            keys(&cards),
            [vec!["username", "password"], vec!["website-0"]]
        );
        assert_eq!(cards[0][1].pending, Some(Part::Password));
        let card = ItemSummary {
            kind: ItemKind::Card,
            ..summary
        };
        assert!(provisional_cards(&card).is_empty());
    }

    #[test]
    fn only_web_addresses_open() {
        assert_eq!(
            website_url("https://bank.example/login").as_deref(),
            Some("https://bank.example/login")
        );
        assert_eq!(
            website_url(" HTTP://router.local ").as_deref(),
            Some("HTTP://router.local")
        );
        assert_eq!(
            website_url("bank.example").as_deref(),
            Some("https://bank.example")
        );
        for refused in [
            "file:///Applications/Calculator.app",
            "smb://files.example/share",
            "ssh://host.example",
            "vnc://host.example",
            "someapp://open?thing=1",
            "https://",
            "",
            "bank example",
        ] {
            assert_eq!(website_url(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_website_shows_as_its_host() {
        assert_eq!(
            website_host("https://www.amazon.com/ap/signin"),
            "amazon.com"
        );
        assert_eq!(website_host("http://192.168.1.1"), "192.168.1.1");
        assert_eq!(website_host("forum.example.net"), "forum.example.net");
        assert_eq!(
            website_host("https://alex@mail.example:8443/#inbox"),
            "mail.example:8443"
        );
        assert_eq!(
            website_host("smb://files.example/share"),
            "smb://files.example/share"
        );
    }
}
