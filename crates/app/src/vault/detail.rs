//! The selected item: every field copies on click, hidden ones reveal on
//! request, the verification code counts down live ([`TotpRow`]).
//!
//! The pane draws a chosen item at once, complete when it can: from what
//! it showed of the item before, kept while the vault is unlocked without
//! the item's hidden values or notes ([`Shape`]), or else from what the
//! list knows of it. The item itself, which reveals and copies, arrives a
//! moment later and changes nothing on screen; only what arrives after a
//! visible wait fades in.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    password_text, scroll_area, tile,
};

use super::totp::{TotpEvent, TotpRow, code_row};
use super::{EditItem, Part, VaultView};
use crate::backend::{self, Load};
use crate::clipboard;
use crate::copied::CopiedSlot;
use crate::icons::{self, Icons};
use crate::recent;
use crate::reprompt::{Reprompt, Unlocked};
use crate::secondary;
use crate::vault_cache;

/// How long a selection moved with the keys rests before its item is
/// fetched.
const SETTLE: Duration = Duration::from_millis(100);
/// What arrives this soon after the item was chosen, a couple of frames,
/// shows as if it had been there: it doesn't fade in.
const AT_ONCE: Duration = Duration::from_millis(50);
/// How long an item that asks for the master password again stays open
/// here without being used, as long as rbw's agent keeps it open: then it
/// asks again. A verification code counting down isn't a use.
const REPROMPT_IDLE: Duration = Duration::from_secs(120);
/// How many items' shapes are kept: those shown last, and those fetched
/// ahead, without one kept for each row the pointer passed.
const MAX_SHAPES: usize = 24;
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
    selected: Option<Arc<ItemSummary>>,
    /// The last answer for the selected item, with the item it answers
    /// for. Until it arrives, the pane shows its shape, or what the list
    /// knows of it.
    loaded: Option<(ItemId, Load<Item>)>,
    /// What the pane showed of the items it showed last, while the vault
    /// is unlocked and unsynced, and of those fetched ahead.
    shapes: Shapes,
    /// When the selected item was chosen.
    chosen_at: Instant,
    /// Whether the fields the item brought fade in: it came after a wait.
    fields_fade: bool,
    revealed: Vec<SharedString>,
    /// The fields shown or hidden again since the item showed, whose
    /// values fade up in place when they change.
    toggled: Vec<SharedString>,
    /// The field something was asked to be copied of before the item
    /// arrived, copied once it does.
    pending_copy: Option<SharedString>,
    copied: CopiedSlot<SharedString>,
    /// The item's verification code, counting down in a view of its own.
    totp: Option<Entity<TotpRow>>,
    _totp_events: Option<Subscription>,
    /// Asks for the master password, for an item that wants it again.
    reprompt: Option<Entity<Reprompt>>,
    /// Whether the password field takes the keyboard when it appears: when
    /// the item was clicked rather than stepped to with the arrow keys.
    focus_reprompt: bool,
    _reprompt_events: Option<Subscription>,
    /// Asks for the master password again once an item that wants it has
    /// gone unused for [`REPROMPT_IDLE`].
    expiry_task: Task<()>,
    /// Whether a change of favourite is on its way, which another waits
    /// for.
    favoriting: bool,
    favorite_task: Task<()>,
    load_task: Task<()>,
    prefetch_task: Task<()>,
    _icons: Subscription,
}

/// What the pane shows of an item without revealing anything: the item
/// with each hidden value and its notes replaced by a stand-in. Enough to
/// draw it again as it was, with no secret the pane doesn't show. Its
/// verification code isn't kept: it changes, and is one.
struct Shape {
    item: Item,
}

impl Shape {
    fn of(item: &Item) -> Self {
        let hide = |secret: &mut Secret| {
            if !secret.is_empty() {
                *secret = Secret::new(MASK);
            }
        };
        let mut item = item.clone();
        match &mut item.data.content {
            ItemContent::Login(login) => {
                hide(&mut login.password);
                hide(&mut login.totp);
            }
            ItemContent::Card(card) => {
                hide(&mut card.number);
                hide(&mut card.code);
            }
            ItemContent::Identity(identity) => {
                hide(&mut identity.ssn);
                hide(&mut identity.passport_number);
                hide(&mut identity.license_number);
            }
            ItemContent::SshKey(key) => hide(&mut key.private_key),
            ItemContent::SecureNote => {}
        }
        // A secure note is all notes; any item's may hold secrets.
        hide(&mut item.data.notes);
        for field in &mut item.data.fields {
            if let FieldValue::Hidden(secret) = &mut field.value {
                hide(secret);
            }
        }
        Self { item }
    }
}

/// The shapes kept, the oldest forgotten past [`MAX_SHAPES`].
#[derive(Default)]
struct Shapes {
    by_item: HashMap<ItemId, Shape>,
    /// Oldest first.
    order: VecDeque<ItemId>,
}

impl Shapes {
    fn get(&self, id: &ItemId) -> Option<&Shape> {
        self.by_item.get(id)
    }

    fn contains(&self, id: &ItemId) -> bool {
        self.by_item.contains_key(id)
    }

    /// Keeps `shape`, in place of the item's shape before when `replace`,
    /// as the item itself just showed; a shape fetched ahead doesn't.
    fn insert(&mut self, shape: Shape, replace: bool) {
        let id = shape.item.id.clone();
        if self.by_item.contains_key(&id) {
            if replace {
                self.by_item.insert(id.clone(), shape);
                self.order.retain(|kept| *kept != id);
                self.order.push_back(id);
            }
            return;
        }
        self.by_item.insert(id.clone(), shape);
        self.order.push_back(id);
        while self.order.len() > MAX_SHAPES {
            if let Some(oldest) = self.order.pop_front() {
                self.by_item.remove(&oldest);
            }
        }
    }

    fn clear(&mut self) {
        self.by_item.clear();
        self.order.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.by_item.len()
    }
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
    /// Copied as a secret, which the clipboard forgets after a while and
    /// when the vault locks: hidden values, and notes, which may hold
    /// anything, though they show as text.
    secret: bool,
    /// The value comes with the item, such as a login's password while
    /// the list only knows the login's name: a click copies it once it
    /// arrives, and it shows hidden until then.
    pending: bool,
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
            secret: matches!(shown, Shown::Hidden | Shown::Password),
            pending: false,
        }
    }

    /// A field copied as a secret, though it shows as text.
    fn secret(self) -> Self {
        Self {
            secret: true,
            ..self
        }
    }

    /// A field whose value comes with the item.
    fn pending(self) -> Self {
        Self {
            pending: true,
            ..self
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
            shapes: Shapes::default(),
            chosen_at: Instant::now(),
            fields_fade: false,
            revealed: vec![],
            toggled: vec![],
            pending_copy: None,
            copied: CopiedSlot::new(),
            totp: None,
            _totp_events: None,
            reprompt: None,
            focus_reprompt: false,
            _reprompt_events: None,
            expiry_task: Task::ready(()),
            favoriting: false,
            favorite_task: Task::ready(()),
            load_task: Task::ready(()),
            prefetch_task: Task::ready(()),
            _icons: cx.observe(&Icons::global(cx), |_, _, cx| cx.notify()),
        }
    }

    fn selected_id(&self) -> Option<&ItemId> {
        self.selected.as_ref().map(|item| &item.id)
    }

    /// The item chosen in the list, as the list shows it.
    pub fn selected(&self) -> Option<&ItemSummary> {
        self.selected.as_deref()
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
    /// one before, and the item itself.
    fn select(&mut self, item: Option<Arc<ItemSummary>>) {
        if item.as_ref().map(|item| &item.id) != self.selected_id() {
            self.forget_item_state();
            self.chosen_at = Instant::now();
            self.fields_fade = false;
        }
        self.selected = item;
    }

    /// What the pane showed of the selected item before, if anything.
    fn shape(&self) -> Option<&Shape> {
        self.shapes.get(self.selected_id()?)
    }

    /// Forgets what the pane showed of every item, as after a sync, which
    /// may have changed them; what was being fetched ahead from before is
    /// dropped too.
    pub fn forget_shapes(&mut self) {
        self.shapes.clear();
        self.prefetch_task = Task::ready(());
    }

    /// Fetches items about to be shown, such as the row under the pointer
    /// or the next one down, so that they show complete at once. Only
    /// their shapes are kept; an item that asks for the master password
    /// again isn't fetched.
    pub fn prefetch(&mut self, items: Vec<Arc<ItemSummary>>, cx: &mut Context<Self>) {
        let wanted: Vec<ItemId> = items
            .into_iter()
            .filter(|item| !item.reprompt && !self.shapes.contains(&item.id))
            .map(|item| item.id.clone())
            .collect();
        if wanted.is_empty() {
            return;
        }
        let account = self.account.clone();
        self.prefetch_task = backend::request(
            cx,
            // A shape has no verification code: the item alone.
            move |backend| {
                Ok(wanted
                    .iter()
                    .filter_map(|id| backend.item(&account, id).ok())
                    .filter(|item| !item.reprompt)
                    .map(|item| Shape::of(&item))
                    .collect::<Vec<_>>())
            },
            |this, result, _| {
                for shape in result.unwrap_or_default() {
                    this.shapes.insert(shape, false);
                }
            },
        );
    }

    /// Shows another item, or none. `by_pointer` says it was clicked, so
    /// that a master password it asks for takes the keyboard at once.
    pub fn show(
        &mut self,
        item: Option<Arc<ItemSummary>>,
        by_pointer: bool,
        cx: &mut Context<Self>,
    ) {
        self.select(item);
        self.focus_reprompt = by_pointer;
        self.reload(cx);
    }

    /// Shows another item once the selection rests: one only passed over,
    /// with the arrow keys or while a search is typed, isn't fetched.
    pub fn show_when_settled(&mut self, item: Option<Arc<ItemSummary>>, cx: &mut Context<Self>) {
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
    /// item selected before the list knew it, such as one just created or
    /// one shown from the menu bar panel as the vault opened, shows now,
    /// its master password taking the keyboard if it was clicked.
    pub fn set_summary(&mut self, item: Arc<ItemSummary>, cx: &mut Context<Self>) {
        if self.selected_id() != Some(&item.id) {
            self.show(Some(item), self.focus_reprompt, cx);
        } else if self.selected.as_deref() != Some(&*item) {
            self.selected = Some(item);
            cx.notify();
        }
    }

    /// Forgets everything of the item shown: what was revealed, copied or
    /// asked for, the item itself with its secrets, its code, and a master
    /// password being typed for it.
    fn forget_item_state(&mut self) {
        self.loaded = None;
        self.revealed.clear();
        self.toggled.clear();
        self.pending_copy = None;
        self.copied.clear();
        self.totp = None;
        self._totp_events = None;
        self.reprompt = None;
        self._reprompt_events = None;
        self.expiry_task = Task::ready(());
    }

    /// Fetches the selected item again, after it may have changed, with
    /// its verification code, in one call.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_id().cloned() else {
            self.load_task = Task::ready(());
            cx.notify();
            return;
        };
        let (account, requested) = (self.account.clone(), id.clone());
        self.load_task = backend::request(
            cx,
            move |backend| backend.item_with_code(&account, &requested),
            move |this, result, cx| {
                // Fields the pane had nothing in place of fade in, after a
                // wait that showed; an item shown again, as after a sync,
                // doesn't.
                let had_nothing = this.loaded().is_none() && this.shape().is_none();
                this.fields_fade = had_nothing && this.chosen_at.elapsed() > AT_ONCE;
                match result {
                    Ok((item, code)) => {
                        if !item.reprompt {
                            this.shapes.insert(Shape::of(&item), true);
                        }
                        let reprompt = item.reprompt;
                        this.loaded = Some((id.clone(), Load::Ready(item)));
                        match code {
                            Some(code) => this.show_code(id, code, cx),
                            None => {
                                this.totp = None;
                                this._totp_events = None;
                            }
                        }
                        if reprompt {
                            this.touched(cx);
                        }
                        // A code asked for was copied with it, if it came.
                        if let Some(key) = this.pending_copy.take()
                            && key != "code"
                        {
                            this.copy_field(key, cx);
                        }
                    }
                    Err(error) => {
                        // Nothing is copied of an item that asks for the
                        // master password first: once given, it is asked
                        // again. Its prompt stays as it is.
                        if error == Error::RepromptRequired && this.pending_copy.take().is_some() {
                            this.focus_reprompt = true;
                        }
                        this.pending_copy = None;
                        this.totp = None;
                        this._totp_events = None;
                        this.expiry_task = Task::ready(());
                        this.loaded = Some((id, Load::Failed(error)));
                    }
                }
                cx.notify();
            },
        );
    }

    /// The item asks for the master password again, as after it went
    /// unread for a while: what showed of it goes.
    fn relocked(&mut self, id: ItemId, cx: &mut Context<Self>) {
        self.forget_item_state();
        self.loaded = Some((id, Load::Failed(Error::RepromptRequired)));
        cx.notify();
    }

    /// The item shown was used, such as a field copied or revealed: one
    /// that asks for the master password stays open [`REPROMPT_IDLE`]
    /// longer.
    fn touched(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self
            .item()
            .filter(|item| item.reprompt)
            .map(|item| item.id.clone())
        else {
            self.expiry_task = Task::ready(());
            return;
        };
        self.expiry_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REPROMPT_IDLE).await;
            this.update(cx, |this, cx| {
                if this.item().is_some_and(|item| item.id == id) {
                    this.relocked(id, cx);
                }
            })
            .ok();
        });
    }

    /// Shows a verification code of the selected item, counting down, and
    /// copies it if it was asked for.
    fn show_code(&mut self, id: ItemId, code: VerificationCode, cx: &mut Context<Self>) {
        match &self.totp {
            Some(row) if *row.read(cx).item() == id => {
                row.update(cx, |row, cx| row.set_code(code, cx));
            }
            _ => {
                let account = self.account.clone();
                let row = cx.new(|cx| TotpRow::new(account, id, code, cx));
                self._totp_events = Some(cx.subscribe(&row, |this, row, event, cx| match event {
                    TotpEvent::Copied => {
                        this.copied.clear();
                        this.touched(cx);
                        cx.notify();
                    }
                    TotpEvent::Relocked => {
                        let id = row.read(cx).item().clone();
                        if this.selected_id() == Some(&id) {
                            this.relocked(id, cx);
                        }
                    }
                }));
                self.totp = Some(row);
            }
        }
        if self.pending_copy.as_deref() == Some("code") {
            self.pending_copy = None;
            self.copy_part(Part::Code, cx);
        }
        cx.notify();
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
        if let Some(row) = &self.totp {
            row.update(cx, |row, cx| row.clear_copied(cx));
        }
        self.touched(cx);
        self.copied.set(key, cx, |this| &mut this.copied);
    }

    /// Copies the password, username or code of the login shown, as if its
    /// row was clicked: once it has arrived, if it is on the way; for a
    /// protected one, the master password field takes the keyboard.
    pub fn copy_part(&mut self, part: Part, cx: &mut Context<Self>) {
        match part {
            Part::Username => self.copy_field("username".into(), cx),
            Part::Password => self.copy_field("password".into(), cx),
            Part::Code => match self.totp.clone() {
                Some(row) if self.item().is_some() => row.update(cx, |row, cx| row.copy(cx)),
                _ if self.selected.as_ref().is_some_and(|item| item.has_code) => {
                    self.copy_field("code".into(), cx)
                }
                _ => {}
            },
        }
    }

    /// Copies the field of the selected item with this key, once the item
    /// has arrived if it is on the way.
    fn copy_field(&mut self, key: SharedString, cx: &mut Context<Self>) {
        let item = match self.loaded() {
            Some(Load::Ready(item)) => item,
            Some(Load::Failed(Error::RepromptRequired)) => {
                self.focus_reprompt(cx);
                return;
            }
            Some(Load::Failed(_)) => return,
            Some(Load::Loading) | None => {
                self.pending_copy = Some(key);
                self.reload(cx);
                return;
            }
        };
        // The code came with the item, if it came: it isn't a field.
        if key == "code" {
            return;
        }
        let field = cards(item)
            .into_iter()
            .flatten()
            .find(|field| field.key == key);
        if let Some(field) = field.filter(|field| !field.value.is_empty()) {
            self.copy(key, &field.value, field.secret, cx);
        }
    }

    fn toggle_reveal(&mut self, key: &SharedString, cx: &mut Context<Self>) {
        if let Some(index) = self.revealed.iter().position(|k| k == key) {
            self.revealed.remove(index);
        } else {
            self.revealed.push(key.clone());
        }
        if !self.toggled.contains(key) {
            self.toggled.push(key.clone());
        }
        self.touched(cx);
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
                        this._reprompt_events = None;
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

    /// Makes the item a favourite or not; the star changes at once, and
    /// back if the change fails. One change at a time.
    fn toggle_favorite(&mut self, cx: &mut Context<Self>) {
        if self.favoriting {
            return;
        }
        let Some(item) = &mut self.selected else {
            return;
        };
        let item = Arc::make_mut(item);
        item.favorite = !item.favorite;
        let (account, id, favorite) = (self.account.clone(), item.id.clone(), item.favorite);
        self.favoriting = true;
        cx.notify();
        let vault = self.vault.clone();
        let changed = id.clone();
        self.favorite_task = backend::request_network(
            cx,
            move |backend| backend.set_favorite(&account, &changed, favorite),
            move |this, result, cx| {
                this.favoriting = false;
                match result {
                    // The item says so too, for the editor; the list
                    // reloads, which brings the summary.
                    Ok(()) => {
                        if let Some((loaded, Load::Ready(item))) = &mut this.loaded
                            && *loaded == id
                        {
                            item.favorite = favorite;
                        }
                        vault_cache::invalidate(&this.account, cx);
                        vault.update(cx, |vault, cx| vault.reload(cx)).ok();
                    }
                    // Back, if the star is still this item's.
                    Err(error) => {
                        if let Some(item) = this.selected.as_mut().filter(|item| item.id == id) {
                            Arc::make_mut(item).favorite = !favorite;
                        }
                        vault
                            .update(cx, |vault, cx| vault.show_error(&error, cx))
                            .ok();
                    }
                }
                cx.notify();
            },
        );
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
                        button("favorite")
                            .icon(IconName::Star)
                            .style(style)
                            .toggled(favorite)
                            .accessibility_label(if favorite {
                                "Remove from Favorites"
                            } else {
                                "Add to Favorites"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_favorite(cx))),
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
        let hidden = matches!(field.shown, Shown::Hidden | Shown::Password);
        // A value on its way shows hidden, revealed or not, until it comes.
        let revealed = self.revealed.contains(&field.key) && !field.pending;
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
        // Revealed or hidden again, it fades up in place; shown with its
        // item, it is there at once.
        let value = if self.toggled.contains(&field.key) {
            motion::replace(
                ElementId::Name(format!("{}-{revealed}", field.key).into()),
                div().min_w_0().child(value),
            )
            .into_any_element()
        } else {
            value
        };
        let key = field.key.clone();
        let copied = self.copied.is(&field.key);
        let value_for_copy = field.value.clone();
        let secret = field.secret;
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
        .on_click(cx.listener(move |this, _, _, cx| {
            if field.pending {
                this.copy_field(key.clone(), cx)
            } else {
                this.copy(key.clone(), &value_for_copy, secret, cx)
            }
        }));
        if hidden {
            let key = field.key.clone();
            let label = if self.revealed.contains(&field.key) {
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
                    this.touched(cx);
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
        let prompt = div()
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
            );
        // It takes the place of the fields only after a wait that showed.
        if self.fields_fade {
            motion::appear(
                ElementId::Name(format!("reprompt-{}", item.id.0).into()),
                prompt,
            )
            .into_any_element()
        } else {
            prompt.into_any_element()
        }
    }

    /// The verification code's row, counting down; while the code is on
    /// the way, its place.
    fn render_code(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let summary = self.selected.as_ref()?;
        if self.item().is_some()
            && let Some(row) = &self.totp
        {
            return Some(row.clone().into_any_element());
        }
        // One that may ask for the master password shows nothing of itself
        // until it is known not to.
        let has_code = summary.has_code && (!summary.reprompt || self.item().is_some());
        has_code.then(|| {
            let tertiary = cx.theme().colors.text_tertiary;
            let digits = div()
                .text_color(tertiary)
                .child("––– –––")
                .into_any_element();
            code_row(digits, 0., false, cx)
                .on_click(cx.listener(|this, _, _, cx| this.copy_part(Part::Code, cx)))
                .into_any_element()
        })
    }
}

#[cfg(test)]
fn has_code(item: &Item) -> bool {
    matches!(&item.data.content, ItemContent::Login(login) if !login.totp.is_empty())
}

/// The web address to open for a website, `https://` when it has none.
/// `None` for an address the browser wouldn't open, such as `file:`,
/// `smb:` or another app's: the system would hand it to that program, and
/// an item shared by an organization can be changed by its other members.
/// Nor for one with a space or a control character inside, such as a line
/// break, which could make it read as something else on its way there.
fn website_url(website: &str) -> Option<String> {
    let website = website.trim();
    if website.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    match website.split_once("://") {
        Some((scheme, rest))
            if !rest.is_empty()
                && (scheme.eq_ignore_ascii_case("https")
                    || scheme.eq_ignore_ascii_case("http")) =>
        {
            Some(website.to_string())
        }
        Some(_) => None,
        None if website.is_empty() => None,
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
    cards.push(non_empty(vec![
        Field::new("notes", "Notes", item.data.notes.clone(), Shown::Text).secret(),
    ]));
    cards
}

impl ItemDetail {
    /// The item's fields in their cards, the verification code in the
    /// first. The cards from `arriving` on came with the item, after what
    /// the list knew of it: they fade in if they came after a wait.
    fn render_cards(
        &self,
        mut cards: Vec<Vec<Field>>,
        arriving: usize,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
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
            if rows.is_empty() {
                continue;
            }
            let card = card().rows(rows);
            groups.push(if groups.len() < arriving || !self.fields_fade {
                card.into_any_element()
            } else {
                let id = self.selected_id().map_or("", |id| id.0.as_str());
                motion::appear(
                    ElementId::Name(format!("card-{id}-{}", groups.len()).into()),
                    div().child(card),
                )
                .into_any_element()
            });
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
            Some(Load::Loading) | None => Ok(match self.shape() {
                Some(shape) => cards(&shape.item)
                    .into_iter()
                    .map(|fields| fields.into_iter().map(Field::pending).collect())
                    .collect(),
                None => provisional_cards(summary),
            }),
        };
        // The cards shown while the item was on the way stay as they are;
        // they matter only when the others fade in.
        let known = if self.fields_fade {
            provisional_cards(summary)
                .iter()
                .filter(|fields| !fields.is_empty())
                .count()
        } else {
            0
        };
        let content: Vec<AnyElement> = match fields {
            Ok(cards) => self.render_cards(cards, known, cx),
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
        // Another item shows at once, without a fade: what changed is
        // plain, and a fade on every step through the list reads as a
        // flicker.
        div()
            .flex()
            .justify_center()
            .child(
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
            )
            .into_any_element()
    }
}

impl Render for ItemDetail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
/// show their fields once they arrive, as does a login that may ask for
/// the master password instead.
fn provisional_cards(summary: &ItemSummary) -> Vec<Vec<Field>> {
    if summary.kind != ItemKind::Login || summary.reprompt {
        return vec![];
    }
    let username = summary
        .subtitle
        .as_ref()
        .map(|name| Field::new("username", "Username", name.as_str(), Shown::Text));
    let password = Field::new("password", "Password", Secret::default(), Shown::Password).pending();
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
    fn a_shape_draws_the_item_without_its_hidden_values() {
        let mut login = item(ItemContent::Login(Login {
            username: "alex".into(),
            password: Secret::new("hunter2"),
            totp: Secret::new("JBSWY3DPEHPK3PXP"),
            ..Login::default()
        }));
        login.data.notes = Secret::new("the recovery codes");
        login.data.fields = vec![rbw_desktop_core::CustomField {
            name: "PIN".into(),
            value: FieldValue::Hidden(Secret::new("1234")),
        }];
        let shape = Shape::of(&login);
        let shown: Vec<String> = cards(&shape.item)
            .into_iter()
            .flatten()
            .map(|field| field.value.expose().to_string())
            .collect();
        assert_eq!(shown, ["alex", MASK, MASK, MASK]);
        // The same fields, so that nothing moves when the item comes.
        assert_eq!(keys(&cards(&shape.item)), keys(&cards(&login)));
        assert!(has_code(&shape.item));
        // Nothing of the secrets themselves, nor of the notes.
        let ItemContent::Login(masked) = &shape.item.data.content else {
            unreachable!()
        };
        assert_eq!(masked.totp.expose(), MASK);
        assert_eq!(shape.item.data.notes.expose(), MASK);
        // A secure note keeps nothing of itself either.
        let mut note = item(ItemContent::SecureNote);
        note.data.notes = Secret::new("the safe's combination");
        assert_eq!(Shape::of(&note).item.data.notes.expose(), MASK);
        // Empty notes stay empty: no notes show where there were none.
        assert!(
            Shape::of(&item(ItemContent::SecureNote))
                .item
                .data
                .notes
                .is_empty()
        );
    }

    #[test]
    fn only_the_last_shapes_are_kept() {
        let mut shapes = Shapes::default();
        let shape = |id: usize| {
            let mut note = item(ItemContent::SecureNote);
            note.id = ItemId(id.to_string());
            Shape::of(&note)
        };
        for id in 0..MAX_SHAPES + 5 {
            shapes.insert(shape(id), false);
        }
        assert_eq!(shapes.len(), MAX_SHAPES);
        // The oldest went first.
        assert!(!shapes.contains(&ItemId("0".into())));
        assert!(shapes.contains(&ItemId((MAX_SHAPES + 4).to_string())));
        // One shown again is kept as the newest.
        let kept = ItemId("5".into());
        shapes.insert(shape(5), true);
        shapes.insert(shape(1000), false);
        assert!(shapes.contains(&kept));
        assert!(!shapes.contains(&ItemId("6".into())));
        shapes.clear();
        assert_eq!(shapes.len(), 0);
    }

    #[test]
    fn notes_and_hidden_values_copy_as_secrets() {
        let mut login = item(ItemContent::Login(Login {
            username: "alex".into(),
            password: Secret::new("hunter2"),
            websites: vec![rbw_desktop_core::Website {
                uri: "bank.example".into(),
                matching: None,
            }],
            ..Login::default()
        }));
        login.data.notes = Secret::new("recovery codes");
        login.data.fields = vec![
            rbw_desktop_core::CustomField {
                name: "PIN".into(),
                value: FieldValue::Hidden(Secret::new("1234")),
            },
            rbw_desktop_core::CustomField {
                name: "Member".into(),
                value: FieldValue::Text("4471".into()),
            },
        ];
        let secret: Vec<(String, bool)> = cards(&login)
            .into_iter()
            .flatten()
            .map(|field| (field.key.to_string(), field.secret))
            .collect();
        assert_eq!(
            secret,
            [
                ("username".to_string(), false),
                ("password".to_string(), true),
                ("website-0".to_string(), false),
                ("field-0".to_string(), true),
                ("field-1".to_string(), false),
                ("notes".to_string(), true),
            ]
        );
        // A secure note's text is its notes: a secret, shown as text.
        let mut note = item(ItemContent::SecureNote);
        note.data.notes = Secret::new("the safe's combination");
        let notes = cards(&note).into_iter().flatten().next().unwrap();
        assert!(notes.secret && notes.shown == Shown::Text);
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
        assert!(cards[0][1].pending);
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
            "https://evil.example\nbank.example",
            "https://bank.example/\u{7}",
            "https://bank.example/a b",
            "bank.example\tother",
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
