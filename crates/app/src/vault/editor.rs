//! The sheet that creates an item or edits one. New logins get a generated
//! password.

use gpui::base::input::{Input, InputEvent, InputState};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, Window, actions, anchored,
    deferred, div, prelude::FluentBuilder as _, px,
};
use rbw_desktop_core::{
    AccountId, Card, Folder, FolderId, GeneratorOptions, Identity, Item, ItemContent, ItemData,
    ItemId, ItemKind, Secret, Website,
};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Material, button, card, form_row, icon,
    segmented_control, surface, switch, toggle_row,
};

use super::VaultView;
use crate::backend;
use crate::secondary;

actions!(item_editor, [Cancel, Save]);

const CONTEXT: &str = "ItemEditor";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new(secondary!("enter"), Save, Some(CONTEXT)),
    ]);
}

/// The kinds the sheet can create, in the order of its segmented control.
const KINDS: [(ItemKind, &str); 4] = [
    (ItemKind::Login, "Login"),
    (ItemKind::Card, "Card"),
    (ItemKind::Identity, "Identity"),
    (ItemKind::SecureNote, "Note"),
];

struct Inputs {
    name: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    website: Entity<InputState>,
    cardholder: Entity<InputState>,
    number: Entity<InputState>,
    expiry: Entity<InputState>,
    code: Entity<InputState>,
    first_name: Entity<InputState>,
    last_name: Entity<InputState>,
    email: Entity<InputState>,
    phone: Entity<InputState>,
    notes: Entity<InputState>,
}

pub struct ItemEditor {
    account: AccountId,
    /// The item being edited; `None` while creating one.
    editing: Option<Item>,
    kind: ItemKind,
    inputs: Inputs,
    folders: Vec<Folder>,
    folder: Option<FolderId>,
    folder_menu_open: bool,
    favorite: bool,
    /// Whether the password is still the generated one.
    generated: bool,
    problem: Option<&'static str>,
    saving: bool,
    vault: WeakEntity<VaultView>,
    focus_handle: FocusHandle,
    task: Task<()>,
}

impl ItemEditor {
    pub fn create(
        account: AccountId,
        kind: ItemKind,
        folder: Option<FolderId>,
        folders: Vec<Folder>,
        vault: WeakEntity<VaultView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let kind = if KINDS.iter().any(|(k, _)| *k == kind) {
            kind
        } else {
            ItemKind::Login
        };
        let mut this = Self::new(account, None, kind, folders, vault, window, cx);
        this.folder = folder;
        this.generate_password(window, cx);
        this
    }

    pub fn edit(
        account: AccountId,
        item: Item,
        folders: Vec<Folder>,
        vault: WeakEntity<VaultView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let kind = item.kind();
        let mut this = Self::new(account, None, kind, folders, vault, window, cx);
        let data = &item.data;
        let set = |input: &Entity<InputState>,
                   value: &str,
                   window: &mut Window,
                   cx: &mut Context<Self>| {
            input.update(cx, |input, cx| {
                input.set_value(value.to_string(), window, cx)
            });
        };
        set(&this.inputs.name, &data.name, window, cx);
        set(&this.inputs.notes, data.notes.expose(), window, cx);
        match &data.content {
            ItemContent::Login(login) => {
                set(&this.inputs.username, &login.username, window, cx);
                set(&this.inputs.password, login.password.expose(), window, cx);
                if let Some(website) = login.websites.first() {
                    set(&this.inputs.website, &website.uri, window, cx);
                }
            }
            ItemContent::Card(card) => {
                set(&this.inputs.cardholder, &card.cardholder_name, window, cx);
                set(&this.inputs.number, card.number.expose(), window, cx);
                if !card.exp_month.is_empty() || !card.exp_year.is_empty() {
                    let expiry = format!("{:0>2}/{}", card.exp_month, card.exp_year);
                    set(&this.inputs.expiry, &expiry, window, cx);
                }
                set(&this.inputs.code, card.code.expose(), window, cx);
            }
            ItemContent::Identity(identity) => {
                set(&this.inputs.first_name, &identity.first_name, window, cx);
                set(&this.inputs.last_name, &identity.last_name, window, cx);
                set(&this.inputs.email, &identity.email, window, cx);
                set(&this.inputs.phone, &identity.phone, window, cx);
            }
            ItemContent::SecureNote | ItemContent::SshKey(_) => {}
        }
        this.folder = data.folder.clone();
        this.favorite = item.favorite;
        this.editing = Some(item);
        this
    }

    fn new(
        account: AccountId,
        editing: Option<Item>,
        kind: ItemKind,
        folders: Vec<Folder>,
        vault: WeakEntity<VaultView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut input = |placeholder: &'static str| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            cx.subscribe_in(&input, window, Self::on_input).detach();
            input
        };
        let inputs = Inputs {
            name: input("Required"),
            username: input(""),
            password: input(""),
            website: input("example.com"),
            cardholder: input(""),
            number: input(""),
            expiry: input("MM/YYYY"),
            code: input(""),
            first_name: input(""),
            last_name: input(""),
            email: input(""),
            phone: input(""),
            notes: input(""),
        };
        Self {
            account,
            editing,
            kind,
            inputs,
            folders,
            folder: None,
            folder_menu_open: false,
            favorite: false,
            generated: false,
            problem: None,
            saving: false,
            vault,
            focus_handle: cx.focus_handle(),
            task: Task::ready(()),
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.inputs
            .name
            .update(cx, |name, cx| name.focus(window, cx));
    }

    fn on_input(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change if *input == self.inputs.password => {
                self.generated = false;
                cx.notify();
            }
            InputEvent::Change if *input == self.inputs.name => {
                self.problem = None;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => self.save(&Save, window, cx),
            InputEvent::Change | InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    fn generate_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.task = backend::request_in(
            window,
            cx,
            |backend| backend.generate(&GeneratorOptions::default()),
            |this, result, window, cx| {
                let Ok(secret) = result else { return };
                this.inputs.password.update(cx, |input, cx| {
                    input.set_value(secret.expose().to_string(), window, cx)
                });
                this.generated = true;
                cx.notify();
            },
        );
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.close(None, window, cx);
    }

    fn close(&mut self, saved: Option<ItemId>, window: &mut Window, cx: &mut Context<Self>) {
        self.vault
            .update(cx, |vault, cx| vault.dismiss_sheet(saved, window, cx))
            .ok();
    }

    fn value(&self, input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().trim().to_string()
    }

    fn item_data(&self, cx: &App) -> ItemData {
        let mut data = self
            .editing
            .as_ref()
            .map_or_else(|| ItemData::new(self.kind), |item| item.data.clone());
        let inputs = &self.inputs;
        data.name = self.value(&inputs.name, cx);
        data.folder = self.folder.clone();
        data.notes = Secret::new(self.value(&inputs.notes, cx));
        match &mut data.content {
            ItemContent::Login(login) => {
                login.username = self.value(&inputs.username, cx);
                login.password = Secret::new(self.value(&inputs.password, cx));
                let website = self.value(&inputs.website, cx);
                match (login.websites.first_mut(), website.is_empty()) {
                    (Some(first), false) => first.uri = website,
                    (Some(_), true) => {
                        login.websites.remove(0);
                    }
                    (None, false) => login.websites.push(Website {
                        uri: website,
                        matching: None,
                    }),
                    (None, true) => {}
                }
            }
            ItemContent::Card(card) => {
                let expiry = self.value(&inputs.expiry, cx);
                let (month, year) = expiry.split_once('/').unwrap_or((&expiry, ""));
                *card = Card {
                    cardholder_name: self.value(&inputs.cardholder, cx),
                    number: Secret::new(self.value(&inputs.number, cx)),
                    exp_month: month.trim().to_string(),
                    exp_year: year.trim().to_string(),
                    code: Secret::new(self.value(&inputs.code, cx)),
                    ..card.clone()
                };
            }
            ItemContent::Identity(identity) => {
                *identity = Identity {
                    first_name: self.value(&inputs.first_name, cx),
                    last_name: self.value(&inputs.last_name, cx),
                    email: self.value(&inputs.email, cx),
                    phone: self.value(&inputs.phone, cx),
                    ..identity.clone()
                };
            }
            ItemContent::SecureNote | ItemContent::SshKey(_) => {}
        }
        data
    }

    fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let data = self.item_data(cx);
        if data.name.is_empty() {
            self.problem = Some("Give the item a name.");
            self.focus(window, cx);
            cx.notify();
            return;
        }
        self.saving = true;
        cx.notify();
        let account = self.account.clone();
        let editing = self
            .editing
            .as_ref()
            .map(|item| (item.id.clone(), item.favorite));
        let favorite = self.favorite;
        self.task = backend::request_in(
            window,
            cx,
            move |backend| {
                let (id, was_favorite) = match editing {
                    Some((id, was_favorite)) => {
                        backend.edit_item(&account, &id, &data)?;
                        (id, was_favorite)
                    }
                    None => (backend.create_item(&account, &data)?.id, false),
                };
                if favorite != was_favorite {
                    backend.set_favorite(&account, &id, favorite)?;
                }
                Ok(id)
            },
            |this, result, window, cx| {
                this.saving = false;
                match result {
                    Ok(id) => this.close(Some(id), window, cx),
                    Err(error) => this.problem = Some(backend::describe(&error)),
                }
                cx.notify();
            },
        );
    }

    fn folder_name(&self) -> String {
        self.folder
            .as_ref()
            .and_then(|id| self.folders.iter().find(|folder| &folder.id == id))
            .map_or_else(|| "None".to_string(), |folder| folder.name.clone())
    }

    fn field(&self, label: &'static str, input: &Entity<InputState>, cx: &App) -> AnyElement {
        form_row(label, div().flex_1().child(Input::new(input)), cx)
    }

    fn render_fields(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let inputs = &self.inputs;
        let mut rows = vec![self.field("Name", &inputs.name, cx)];
        match self.kind {
            ItemKind::Login => {
                rows.push(self.field("Username", &inputs.username, cx));
                let mono = cx.theme().mono_font_family.clone();
                rows.push(form_row(
                    "Password",
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_family(mono)
                                .text_size(px(12.))
                                .child(Input::new(&inputs.password)),
                        )
                        .child(
                            button("generate")
                                .icon(IconName::Refresh)
                                .size(px(26.))
                                .accessibility_label("Generate Password")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.generate_password(window, cx)
                                })),
                        ),
                    cx,
                ));
                rows.push(self.field("Website", &inputs.website, cx));
            }
            ItemKind::Card => {
                rows.push(self.field("Cardholder", &inputs.cardholder, cx));
                rows.push(self.field("Number", &inputs.number, cx));
                rows.push(self.field("Expires", &inputs.expiry, cx));
                rows.push(self.field("Security code", &inputs.code, cx));
            }
            ItemKind::Identity => {
                rows.push(self.field("First name", &inputs.first_name, cx));
                rows.push(self.field("Last name", &inputs.last_name, cx));
                rows.push(self.field("Email", &inputs.email, cx));
                rows.push(self.field("Phone", &inputs.phone, cx));
            }
            ItemKind::SecureNote | ItemKind::SshKey => {
                rows.push(self.field("Note", &inputs.notes, cx));
            }
        }
        rows
    }

    fn render_folder_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let hover = theme.colors.hover;
        let options = std::iter::once((None, "None".to_string())).chain(
            self.folders
                .iter()
                .map(|folder| (Some(folder.id.clone()), folder.name.clone())),
        );
        deferred(
            anchored().snap_to_window_with_margin(px(8.)).child(
                div().mt(px(4.)).occlude().child(
                    surface("folder-menu", Material::Popover)
                        .min_w(px(180.))
                        .p(px(5.))
                        .rounded(px(10.))
                        .children(options.enumerate().map(|(index, (id, name))| {
                            let selected = id == self.folder;
                            div()
                                .id(index)
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .h(px(26.))
                                .px(px(8.))
                                .rounded(px(6.))
                                .hover(move |s| s.bg(hover))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.folder = id.clone();
                                    this.folder_menu_open = false;
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .w(px(15.))
                                        .when(selected, |d| d.child(icon(IconName::Check))),
                                )
                                .child(name)
                        })),
                ),
            ),
        )
        .with_priority(1)
        .into_any_element()
    }
}

impl Focusable for ItemEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ItemEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let (dim, faint, success, danger) = (
            colors.text_secondary,
            colors.text_tertiary,
            colors.success,
            colors.danger,
        );
        let title = if self.editing.is_some() {
            "Edit Item"
        } else {
            "New Item"
        };
        let selected_kind = KINDS
            .iter()
            .position(|(kind, _)| *kind == self.kind)
            .unwrap_or_default();
        let fields = self.render_fields(cx);
        let menu = self.folder_menu_open.then(|| self.render_folder_menu(cx));
        let folder_row = div()
            .id("folder")
            .on_click(cx.listener(|this, _, _, cx| {
                this.folder_menu_open = !this.folder_menu_open;
                cx.notify();
            }))
            .child(form_row(
                "Folder",
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    // The menu opens under the value, like a pop-up button's.
                    .child(div().flex_1().child(self.folder_name()).children(menu))
                    .child(icon(IconName::ChevronsUpDown).text_color(faint)),
                cx,
            ));

        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::save))
            .flex()
            .flex_col()
            .child(
                div()
                    .mb(px(12.))
                    .text_size(px(15.))
                    .font_weight(FontWeight::BOLD)
                    .child(title),
            )
            .when(self.editing.is_none(), |d| {
                d.child(
                    segmented_control("kind", KINDS.map(|(_, label)| label), selected_kind)
                        .on_select({
                            let this = cx.entity().downgrade();
                            move |index, _, cx| {
                                this.update(cx, |this, cx| {
                                    this.kind = KINDS[index].0;
                                    cx.notify();
                                })
                                .ok();
                            }
                        }),
                )
                .child(div().h(px(12.)))
            })
            .child(card().on_surface().rows(fields))
            .when(self.kind == ItemKind::Login && self.generated, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .mt(px(8.))
                        .mx(px(13.))
                        .text_size(px(12.))
                        .text_color(dim)
                        .child(div().size(px(7.)).rounded_full().bg(success))
                        .child("Strong password, generated for you"),
                )
            })
            .child(
                div()
                    .mt(px(12.))
                    .child(card().on_surface().row(folder_row).row(toggle_row(
                        "Add to Favorites",
                        switch("favorite", self.favorite, "Add to Favorites").on_change({
                            let this = cx.entity().downgrade();
                            move |on, _, cx| {
                                this.update(cx, |this, cx| {
                                    this.favorite = on;
                                    cx.notify();
                                })
                                .ok();
                            }
                        }),
                    ))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .mt(px(14.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(danger)
                            .children(self.problem),
                    )
                    .child(button("cancel").label("Cancel").on_click(
                        cx.listener(|this, _, window, cx| this.cancel(&Cancel, window, cx)),
                    ))
                    .child(
                        button("save")
                            .label("Save")
                            .style(ButtonStyle::Primary)
                            .disabled(self.saving)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save(&Save, window, cx)),
                            ),
                    ),
            )
    }
}
