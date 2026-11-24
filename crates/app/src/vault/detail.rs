//! The selected item: every field copies on click, hidden ones reveal on
//! request, the verification code counts down live.

use std::time::{Duration, SystemTime};

use gpui::{
    AnyElement, Context, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Task,
    WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use rbw_desktop_core::{
    AccountId, FieldValue, Item, ItemContent, ItemId, ItemKind, Secret, VerificationCode,
};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Look, button, card, chrome, field_row, icon, motion,
    progress_ring, tile,
};

use super::VaultView;
use crate::backend::{self, Load};
use crate::clipboard;

pub struct ItemDetail {
    account: AccountId,
    vault: WeakEntity<VaultView>,
    shown: Option<(ItemId, Load<Item>)>,
    revealed: Vec<SharedString>,
    copied: Option<SharedString>,
    code: Option<VerificationCode>,
    load_task: Task<()>,
    copied_timer: Task<()>,
    code_task: Task<()>,
}

/// How a field's value is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shown {
    Text,
    Hidden,
    Link,
    Mono,
}

struct Field {
    key: SharedString,
    label: SharedString,
    value: Secret,
    shown: Shown,
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
        }
    }
}

impl ItemDetail {
    pub fn new(account: AccountId, vault: WeakEntity<VaultView>) -> Self {
        Self {
            account,
            vault,
            shown: None,
            revealed: vec![],
            copied: None,
            code: None,
            load_task: Task::ready(()),
            copied_timer: Task::ready(()),
            code_task: Task::ready(()),
        }
    }

    pub fn item(&self) -> Option<&Item> {
        self.shown.as_ref()?.1.ready()
    }

    pub fn show(&mut self, item: Option<ItemId>, cx: &mut Context<Self>) {
        self.revealed.clear();
        self.copied = None;
        self.code = None;
        self.code_task = Task::ready(());
        self.shown = item.map(|id| (id, Load::Loading));
        self.reload(cx);
    }

    /// Fetches the shown item again, after it may have changed.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some((id, _)) = &self.shown else {
            cx.notify();
            return;
        };
        let (account, id) = (self.account.clone(), id.clone());
        self.load_task = backend::request(
            cx,
            move |backend| backend.item(&account, &id),
            |this, result, cx| {
                let has_code = matches!(&result, Ok(item) if has_code(item));
                if let Some((_, load)) = &mut this.shown {
                    *load = result.into();
                }
                if has_code {
                    this.fetch_code(cx);
                }
                cx.notify();
            },
        );
    }

    fn fetch_code(&mut self, cx: &mut Context<Self>) {
        let Some((id, _)) = &self.shown else { return };
        let (account, id) = (self.account.clone(), id.clone());
        self.code_task = backend::request(
            cx,
            move |backend| backend.verification_code(&account, &id),
            |this, result, cx| {
                let Ok(code) = result else { return };
                let expires_at = code.expires_at;
                this.code = Some(code);
                // Redraw each second for the countdown, then fetch the next code.
                this.code_task = cx.spawn(async move |this, cx| {
                    while SystemTime::now() < expires_at {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            return;
                        }
                    }
                    this.update(cx, |this, cx| this.fetch_code(cx)).ok();
                });
                cx.notify();
            },
        );
    }

    fn copy(&mut self, key: SharedString, value: &Secret, secret: bool, cx: &mut Context<Self>) {
        clipboard::copy(value, secret, cx);
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

    fn toggle_reveal(&mut self, key: &SharedString, cx: &mut Context<Self>) {
        if let Some(index) = self.revealed.iter().position(|k| k == key) {
            self.revealed.remove(index);
        } else {
            self.revealed.push(key.clone());
        }
        cx.notify();
    }

    fn toggle_favorite(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.item() else { return };
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
        let item = self.item();
        let editable = item.is_some_and(|item| item.editable);
        let favorite = item.is_some_and(|item| item.favorite);
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
                .when(editable, |bar| {
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
                    .child(
                        button("edit")
                            .label("Edit")
                            .style(ButtonStyle::Normal)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.vault
                                    .update(cx, |vault, cx| vault.edit_selected(window, cx))
                                    .ok();
                            })),
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
        let hidden = field.shown == Shown::Hidden;
        let value: AnyElement = match field.shown {
            Shown::Hidden if !revealed => div()
                .text_size(px(15.))
                .line_height(px(18.))
                .child("•".repeat(field.value.expose().chars().count().clamp(8, 12)))
                .into_any_element(),
            Shown::Link => div()
                .truncate()
                .text_color(colors.accent_text)
                .child(field.value.expose().to_string())
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
        let mut row = field_row(ElementId::Name(field.key.clone()), field.label, value)
            .copied(copied)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.copy(key.clone(), &value_for_copy, hidden, cx)
            }));
        if hidden {
            let key = field.key.clone();
            row = row.action(
                div()
                    .id(ElementId::Name(format!("reveal-{}", field.key).into()))
                    .when(revealed, |d| d.text_color(colors.accent_text))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_reveal(&key, cx);
                    }))
                    .child(icon(IconName::Eye)),
            );
        }
        if field.shown == Shown::Link {
            let url = website_url(field.value.expose());
            row = row.action(
                div()
                    .id(ElementId::Name(format!("open-{}", field.key).into()))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        cx.open_url(&url);
                    })
                    .child(icon(IconName::External)),
            );
        } else if hidden {
            row = row.action(icon(IconName::Copy));
        }
        row.into_any_element()
    }

    fn render_code(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let code = self.code.as_ref()?;
        let theme = cx.theme();
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
        let key: SharedString = "code".into();
        let copied = self.copied.as_ref() == Some(&key);
        Some(
            field_row(
                "code",
                "Verification code",
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(spaced)
                    .child(progress_ring(fraction, cx)),
            )
            .copied(copied)
            .action(icon(IconName::Copy).text_color(theme.colors.text_tertiary))
            .on_click(cx.listener(move |this, _, _, cx| this.copy(key.clone(), &value, true, cx)))
            .into_any_element(),
        )
    }
}

fn has_code(item: &Item) -> bool {
    matches!(&item.data.content, ItemContent::Login(login) if !login.totp.is_empty())
}

fn website_url(website: &str) -> String {
    if website.contains("://") {
        website.to_string()
    } else {
        format!("https://{website}")
    }
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
                    Shown::Hidden,
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

impl Render for ItemDetail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fill = theme.colors.window;
        let dim = theme.colors.text_secondary;
        let toolbar = self.render_toolbar(window, cx);
        let corner = chrome::corner_radius(window, cx);
        let body: Option<AnyElement> = match &self.shown {
            Some((_, Load::Ready(item))) => {
                let item = item.clone();
                let code = self.render_code(cx);
                let mut cards = cards(&item);
                let mut groups: Vec<AnyElement> = vec![];
                let first = cards.remove(0);
                let mut first_rows: Vec<AnyElement> = first
                    .into_iter()
                    .map(|f| self.render_field(f, cx))
                    .collect();
                first_rows.extend(code);
                for rows in std::iter::once(first_rows).chain(cards.into_iter().map(|fields| {
                    fields
                        .into_iter()
                        .map(|f| self.render_field(f, cx))
                        .collect()
                })) {
                    if !rows.is_empty() {
                        groups.push(card().rows(rows).into_any_element());
                    }
                }
                let header = div()
                    .flex()
                    .items_center()
                    .gap(px(13.))
                    .mb(px(4.))
                    .child(tile(&item.data.name).size(px(56.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(item.data.name.clone()),
                            )
                            .child(div().text_color(dim).child(kind_label(item.kind()))),
                    );
                Some(
                    motion::content_enter(
                        ElementId::Name(format!("detail-{}", item.id.0).into()),
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .px(px(16.))
                            .pt(px(2.))
                            .pb(px(16.))
                            .child(header)
                            .children(groups),
                    )
                    .into_any_element(),
                )
            }
            Some((_, Load::Failed(error))) => Some(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(dim)
                    .child(backend::describe(error))
                    .into_any_element(),
            ),
            Some((_, Load::Loading)) | None => None,
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(fill)
            .rounded_r(corner)
            .child(toolbar)
            .child(
                div()
                    .id("detail-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(body),
            )
    }
}
