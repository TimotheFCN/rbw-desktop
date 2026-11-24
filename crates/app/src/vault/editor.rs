//! The sheet that creates an item or edits one. New logins get a generated
//! password; the generator's options open from the password's row.

use gpui::base::Button as BaseButton;
use gpui::base::input::{Input, InputEvent, InputState};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Task, WeakEntity, Window, actions, div, prelude::FluentBuilder as _,
    px, relative,
};
use rbw_desktop_core::{
    AccountId, Card, Error, Folder, FolderId, Identity, Item, ItemContent, ItemData, ItemId,
    ItemKind, Secret, Website,
};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Opening, Popover, button, card, form_row, icon,
    menu_item, menu_separator, password_text, popover, segmented_control, switch, toggle_row,
};

use super::VaultView;
use crate::backend;
use crate::generator::{self, Generated, Generator};
use crate::reprompt::{Reprompt, Unlocked};
use crate::secondary;

actions!(item_editor, [Cancel, Save, CancelNewFolder]);

const CONTEXT: &str = "ItemEditor";
const NEW_FOLDER_CONTEXT: &str = "NewFolder";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new(secondary!("enter"), Save, Some(CONTEXT)),
        KeyBinding::new("escape", CancelNewFolder, Some(NEW_FOLDER_CONTEXT)),
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
    folder_menu_focus: FocusHandle,
    /// The name of a folder being created from the folder menu.
    new_folder: Option<Entity<InputState>>,
    favorite: bool,
    /// Makes the password, with the options its popover shows.
    generator: Entity<Generator>,
    generator_open: bool,
    generator_focus: FocusHandle,
    /// Whether the password is still the generated one.
    generated: bool,
    problem: Option<&'static str>,
    /// Asks for the master password, when the item asks for it again
    /// before it is saved.
    reprompt: Option<Entity<Reprompt>>,
    _reprompt_events: Option<Subscription>,
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
        let mut this = Self::new(account, None, kind, folders, vault, true, window, cx);
        this.folder = folder;
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
        let mut this = Self::new(account, None, kind, folders, vault, false, window, cx);
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

    #[expect(
        clippy::too_many_arguments,
        reason = "the two ways to open the editor share it"
    )]
    fn new(
        account: AccountId,
        editing: Option<Item>,
        kind: ItemKind,
        folders: Vec<Folder>,
        vault: WeakEntity<VaultView>,
        generate_now: bool,
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
        let generator = cx.new(|cx| Generator::new(generate_now, cx));
        cx.subscribe_in(
            &generator,
            window,
            |this, _, Generated(value), window, cx| {
                this.inputs.password.update(cx, |input, cx| {
                    input.set_value(value.expose().to_string(), window, cx)
                });
                this.generated = true;
                cx.notify();
            },
        )
        .detach();
        Self {
            account,
            editing,
            kind,
            inputs,
            folders,
            folder: None,
            folder_menu_open: false,
            folder_menu_focus: cx.focus_handle(),
            new_folder: None,
            favorite: false,
            generator,
            generator_open: false,
            generator_focus: cx.focus_handle(),
            generated: false,
            problem: None,
            reprompt: None,
            _reprompt_events: None,
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
            InputEvent::PressEnter { .. } if Some(input) == self.new_folder.as_ref() => {
                self.create_folder(window, cx)
            }
            InputEvent::PressEnter { .. } => self.save(&Save, window, cx),
            InputEvent::Change | InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    fn toggle_generator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generator_open = !self.generator_open;
        window.focus(
            if self.generator_open {
                &self.generator_focus
            } else {
                &self.focus_handle
            },
            cx,
        );
        cx.notify();
    }

    fn close_generator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generator_open = false;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// The buttons at the end of the password's row: another password, and
    /// the generator's options in a popover.
    fn render_generator_buttons(&self, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity().downgrade();
        let options = popover(
            "generator-options",
            self.generator_open,
            &self.generator_focus,
            "Password Options",
        )
        .dialog()
        .opening(Opening::BelowEnd)
        .on_dismiss(move |window, cx| {
            this.update(cx, |this, cx| this.close_generator(window, cx))
                .ok();
        })
        .child(
            div().w(px(282.)).child(
                self.generator
                    .update(cx, |generator, cx| generator.render_options(cx)),
            ),
        );
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                button("generate")
                    .icon(IconName::Refresh)
                    .size(px(26.))
                    .accessibility_label("Generate Password")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.generator.update(cx, Generator::regenerate)
                    })),
            )
            .child(
                div()
                    .relative()
                    .child(
                        button("generator-options-button")
                            .icon(IconName::Sliders)
                            .size(px(26.))
                            .accessibility_label("Password Options")
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.toggle_generator(window, cx)
                                }),
                            ),
                    )
                    .child(options),
            )
            .into_any_element()
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
                    Err(Error::RepromptRequired) => this.ask_password(window, cx),
                    Err(error) => this.problem = Some(backend::describe(&error)),
                }
                cx.notify();
            },
        );
    }

    /// The item locked again since it was opened: its master password,
    /// then the save again.
    fn ask_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = &self.editing else { return };
        let (account, id) = (self.account.clone(), item.id.clone());
        let reprompt = cx.new(|cx| Reprompt::new(account, id, window, cx));
        reprompt.update(cx, |reprompt, cx| reprompt.focus(window, cx));
        self._reprompt_events =
            Some(
                cx.subscribe_in(&reprompt, window, |this, _, _: &Unlocked, window, cx| {
                    this.reprompt = None;
                    this.focus(window, cx);
                    this.save(&Save, window, cx);
                }),
            );
        self.reprompt = Some(reprompt);
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

    /// The password, its digits and symbols coloured while it isn't being
    /// edited. The field stays underneath, so it still takes the focus and
    /// the click.
    fn render_password(&self, mono: SharedString, window: &Window, cx: &App) -> AnyElement {
        let input = &self.inputs.password;
        let editing = input.read(cx).focus_handle(cx).is_focused(window);
        let value = input.read(cx).value();
        div()
            .flex_1()
            .min_w_0()
            .relative()
            .font_family(mono)
            .text_size(px(12.))
            .child(
                div()
                    .when(!editing, |d| d.text_color(gpui::transparent_black()))
                    .child(Input::new(input)),
            )
            .when(!editing, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(password_text(&value, cx)),
                )
            })
            .into_any_element()
    }

    fn render_fields(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
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
                        .child(self.render_password(mono, window, cx))
                        .child(self.render_generator_buttons(cx)),
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

    fn render_folder_menu(&self, cx: &mut Context<Self>) -> Popover {
        let choices = std::iter::once((None, "None".to_string())).chain(
            self.folders
                .iter()
                .map(|folder| (Some(folder.id.clone()), folder.name.clone())),
        );
        let this = cx.entity().downgrade();
        popover(
            "folder-menu",
            self.folder_menu_open,
            &self.folder_menu_focus,
            "Folder",
        )
        .on_dismiss(move |window, cx| {
            this.update(cx, |this, cx| this.close_folder_menu(window, cx))
                .ok();
        })
        .children(choices.enumerate().map(|(index, (id, name))| {
            let checked = id == self.folder;
            menu_item(("folder", index), name)
                .checked(checked)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.folder = id.clone();
                    this.close_folder_menu(window, cx);
                }))
        }))
        .child(menu_separator(cx))
        .child(
            menu_item("new-folder", "New Folder…")
                .on_click(cx.listener(|this, _, window, cx| this.start_new_folder(window, cx))),
        )
    }

    fn toggle_folder_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.folder_menu_open {
            self.close_folder_menu(window, cx);
        } else {
            self.folder_menu_open = true;
            window.focus(&self.folder_menu_focus, cx);
            cx.notify();
        }
    }

    fn close_folder_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.folder_menu_open = false;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn start_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.folder_menu_open = false;
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Folder name"));
        cx.subscribe_in(&input, window, Self::on_input).detach();
        input.update(cx, |input, cx| input.focus(window, cx));
        self.new_folder = Some(input);
        cx.notify();
    }

    fn cancel_new_folder(
        &mut self,
        _: &CancelNewFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_folder = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn create_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = &self.new_folder else {
            return;
        };
        let name = self.value(input, cx);
        if name.is_empty() {
            return;
        }
        let account = self.account.clone();
        self.task = backend::request_in(
            window,
            cx,
            move |backend| backend.create_folder(&account, &name),
            |this, result, window, cx| {
                match result {
                    Ok(folder) => {
                        this.folder = Some(folder.id.clone());
                        this.folders.push(folder);
                        this.new_folder = None;
                        window.focus(&this.focus_handle, cx);
                        // The sidebar lists the new folder at once.
                        this.vault.update(cx, |vault, cx| vault.reload(cx)).ok();
                    }
                    Err(error) => this.problem = Some(backend::describe(&error)),
                }
                cx.notify();
            },
        );
    }

    fn render_folder_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let faint = cx.theme().colors.text_tertiary;
        if let Some(input) = &self.new_folder {
            return div()
                .key_context(NEW_FOLDER_CONTEXT)
                .on_action(cx.listener(Self::cancel_new_folder))
                .child(form_row(
                    "Folder",
                    div().flex_1().child(Input::new(input)),
                    cx,
                ))
                .into_any_element();
        }
        let name = self.folder_name();
        let hover = cx.theme().colors.hover;
        BaseButton::new("folder")
            .accessibility_label(format!("Folder, {name}"))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_folder_menu(window, cx)))
            .w_full()
            .justify_start()
            .hover(move |s| s.bg(hover))
            .child(form_row(
                "Folder",
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .line_height(relative(1.35))
                    // The menu opens under the value, like a pop-up button's.
                    .child(
                        div()
                            .flex_1()
                            .relative()
                            .child(name)
                            .child(self.render_folder_menu(cx)),
                    )
                    .child(icon(IconName::ChevronsUpDown).text_color(faint)),
                cx,
            ))
            .into_any_element()
    }

    /// How strong the generated password is, under the fields.
    fn render_generated_note(&self, cx: &App) -> AnyElement {
        let generator = self.generator.read(cx);
        let noun = generator.noun();
        let (dot, note) = match generator.strength() {
            Some(strength) => {
                let (color, word) = generator::strength_look(strength, cx);
                (Some(color), format!("{word} {noun}, generated for you"))
            }
            None => (None, format!("{noun}, generated for you")),
        };
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .mt(px(8.))
            .mx(px(13.))
            .text_size(px(12.))
            .text_color(cx.theme().colors.text_secondary)
            .children(dot.map(|color| div().size(px(7.)).rounded_full().bg(color)))
            .child(note)
            .into_any_element()
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = &self.editing else { return };
        let (id, name) = (item.id.clone(), item.data.name.clone());
        self.vault
            .update(cx, |vault, cx| vault.delete_item(id, name, window, cx))
            .ok();
    }
}

impl Focusable for ItemEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ItemEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let (dim, danger) = (colors.text_secondary, colors.danger);
        let title = if self.editing.is_some() {
            "Edit Item"
        } else {
            "New Item"
        };
        let selected_kind = KINDS
            .iter()
            .position(|(kind, _)| *kind == self.kind)
            .unwrap_or_default();
        let fields = self.render_fields(window, cx);
        let folder_row = self.render_folder_row(cx);
        let deletable = self.editing.as_ref().is_some_and(|item| item.editable);
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
                d.child(self.render_generated_note(cx))
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
            .when_some(self.reprompt.clone(), |d, reprompt| {
                d.child(
                    div()
                        .mt(px(14.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(
                            div()
                                .mx(px(13.))
                                .text_size(px(12.))
                                .text_color(dim)
                                .child("Enter your master password to save this item."),
                        )
                        .child(reprompt),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .mt(px(14.))
                    .when(deletable, |d| {
                        d.child(
                            button("delete").label("Delete").on_click(
                                cx.listener(|this, _, window, cx| this.delete(window, cx)),
                            ),
                        )
                    })
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
