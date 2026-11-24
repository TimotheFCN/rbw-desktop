//! The sheet that creates an item or edits one: every field the item view
//! shows, the common ones first and the rest from "Add Field". New logins
//! get a generated password; the generator's options open from the
//! password's row. Saving writes back only what changed ([`form`]).

mod form;
mod sections;

use std::collections::BTreeMap;

use gpui::base::input::{InputEvent, InputState, TextareaState};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, KeyBinding, Render,
    ScrollHandle, Subscription, Task, WeakEntity, Window, actions,
};
use rbw_desktop_core::{
    AccountId, CustomField, Error, FieldValue, Folder, FolderId, Item, ItemData, ItemId, ItemKind,
    Secret,
};

use super::VaultView;
use crate::backend;
use crate::generator::{Generated, Generator};
use crate::reprompt::{Reprompt, Unlocked};
use crate::secondary;
use form::{FieldText, Key, Texts, WebsiteText};

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

/// Fields that hold secrets: hidden while another field has the keyboard.
const SECRETS: [Key; 7] = [
    Key::Password,
    Key::Authenticator,
    Key::Number,
    Key::SecurityCode,
    Key::Ssn,
    Key::Passport,
    Key::License,
];

/// Groups of fields that show only once they hold something, or once
/// "Add Field" asked for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Extra {
    Authenticator,
    TitleAndMiddleName,
    Address,
    Identification,
}

impl Extra {
    fn keys(self) -> &'static [Key] {
        match self {
            Extra::Authenticator => &[Key::Authenticator],
            Extra::TitleAndMiddleName => &[Key::Title, Key::MiddleName],
            Extra::Address => &[
                Key::Address1,
                Key::Address2,
                Key::Address3,
                Key::City,
                Key::State,
                Key::PostalCode,
                Key::Country,
            ],
            Extra::Identification => &[Key::Ssn, Key::Passport, Key::License],
        }
    }

    fn kind(self) -> ItemKind {
        match self {
            Extra::Authenticator => ItemKind::Login,
            Extra::TitleAndMiddleName | Extra::Address | Extra::Identification => {
                ItemKind::Identity
            }
        }
    }

    /// The item in "Add Field".
    fn label(self) -> &'static str {
        match self {
            Extra::Authenticator => "Authenticator Key",
            Extra::TitleAndMiddleName => "Title and Middle Name",
            Extra::Address => "Address",
            Extra::Identification => "Identity Documents",
        }
    }
}

const EXTRAS: [Extra; 4] = [
    Extra::Authenticator,
    Extra::TitleAndMiddleName,
    Extra::Address,
    Extra::Identification,
];

/// A website's row.
struct WebsiteRow {
    /// Names the row's elements while rows come and go.
    id: usize,
    origin: Option<usize>,
    input: Entity<InputState>,
}

/// A custom field's row.
struct FieldRow {
    id: usize,
    origin: Option<usize>,
    name: Entity<InputState>,
    value: FieldInput,
}

enum FieldInput {
    Text(Entity<InputState>),
    Hidden(Entity<InputState>),
    Boolean(bool),
    /// Shows another field of the item; the item view doesn't show it,
    /// and the editor keeps it as it is.
    Linked(u16),
}

impl FieldRow {
    fn text(&self, cx: &App) -> FieldText {
        let value = match &self.value {
            FieldInput::Text(input) => FieldValue::Text(input.read(cx).value().to_string()),
            FieldInput::Hidden(input) => {
                FieldValue::Hidden(Secret::new(input.read(cx).value().to_string()))
            }
            FieldInput::Boolean(on) => FieldValue::Boolean(*on),
            FieldInput::Linked(id) => FieldValue::Linked(*id),
        };
        FieldText {
            origin: self.origin,
            field: CustomField {
                name: self.name.read(cx).value().to_string(),
                value,
            },
        }
    }
}

/// The kind of a custom field "Add Field" adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NewField {
    Text,
    Hidden,
    Boolean,
}

pub struct ItemEditor {
    account: AccountId,
    /// The item being edited; `None` while creating one.
    editing: Option<Item>,
    kind: ItemKind,
    /// What the fields showed first, so that only what changed is saved.
    initial: Texts,
    inputs: BTreeMap<Key, Entity<InputState>>,
    notes: Entity<TextareaState>,
    websites: Vec<WebsiteRow>,
    fields: Vec<FieldRow>,
    next_row: usize,
    /// Groups shown though empty, which "Add Field" asked for.
    added: Vec<Extra>,
    add_menu_open: bool,
    add_menu_focus: FocusHandle,
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
    /// Whether the password is still the generated one, which shows.
    generated: bool,
    /// Whether a value the generator makes goes into the password: not
    /// after the person typed one, until they ask the generator again.
    take_generated: bool,
    /// Whether the eye shows the password while it doesn't have the
    /// keyboard.
    password_shown: bool,
    problem: Option<&'static str>,
    /// Asks for the master password, when the item asks for it again
    /// before it is saved.
    reprompt: Option<Entity<Reprompt>>,
    _reprompt_events: Option<Subscription>,
    saving: bool,
    vault: WeakEntity<VaultView>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    /// The group last scrolled into view because one of its fields took
    /// the keyboard.
    revealed_group: Option<usize>,
    task: Task<()>,
    _subscriptions: Vec<Subscription>,
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
        let data = ItemData {
            folder,
            ..ItemData::new(kind)
        };
        Self::new(account, None, data, false, folders, vault, window, cx)
    }

    pub fn edit(
        account: AccountId,
        item: Item,
        folders: Vec<Folder>,
        vault: WeakEntity<VaultView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (data, favorite) = (item.data.clone(), item.favorite);
        Self::new(
            account,
            Some(item),
            data,
            favorite,
            folders,
            vault,
            window,
            cx,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the two ways to open the editor share it"
    )]
    fn new(
        account: AccountId,
        editing: Option<Item>,
        data: ItemData,
        favorite: bool,
        folders: Vec<Folder>,
        vault: WeakEntity<VaultView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let kind = data.content.kind();
        let initial = form::texts(&data);
        let new_item = editing.is_none();
        let mut subscriptions = vec![];
        let mut inputs = BTreeMap::new();
        for key in ALL_KEYS {
            let input = new_input(placeholder(key), SECRETS.contains(&key), window, cx);
            input.update(cx, |input, cx| {
                input.set_value(initial.get(key).to_string(), window, cx)
            });
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, input, event, window, cx| {
                    this.on_input(Some(key), input, event, window, cx)
                },
            ));
            inputs.insert(key, input);
        }
        let notes = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .placeholder("Notes")
        });
        notes.update(cx, |notes, cx| {
            notes.set_value(initial.notes.clone(), window, cx)
        });
        let generator = cx.new(|cx| Generator::new(new_item && kind == ItemKind::Login, cx));
        subscriptions.push(cx.subscribe_in(
            &generator,
            window,
            |this, _, Generated(value), window, cx| this.take_generated(value, window, cx),
        ));
        let mut this = Self {
            account,
            editing,
            kind,
            initial: initial.clone(),
            inputs,
            notes,
            websites: vec![],
            fields: vec![],
            next_row: 0,
            added: vec![],
            add_menu_open: false,
            add_menu_focus: cx.focus_handle(),
            folders,
            folder: data.folder.clone(),
            folder_menu_open: false,
            folder_menu_focus: cx.focus_handle(),
            new_folder: None,
            favorite,
            generator,
            generator_open: false,
            generator_focus: cx.focus_handle(),
            generated: false,
            take_generated: new_item,
            password_shown: false,
            problem: None,
            reprompt: None,
            _reprompt_events: None,
            saving: false,
            vault,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            revealed_group: None,
            task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        for website in &initial.websites {
            this.add_website(website.origin, &website.uri, window, cx);
        }
        if this.websites.is_empty() {
            this.add_website(None, "", window, cx);
        }
        for field in &initial.fields {
            this.add_field_row(field, window, cx);
        }
        this
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input(Key::Name)
            .update(cx, |name, cx| name.focus(window, cx));
    }

    fn input(&self, key: Key) -> &Entity<InputState> {
        &self.inputs[&key]
    }

    fn value(&self, key: Key, cx: &App) -> String {
        self.input(key).read(cx).value().to_string()
    }

    fn next_row(&mut self) -> usize {
        self.next_row += 1;
        self.next_row
    }

    fn add_website(
        &mut self,
        origin: Option<usize>,
        uri: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let input = new_input("example.com", false, window, cx);
        input.update(cx, |input, cx| input.set_value(uri.to_string(), window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &input,
            window,
            |this, input, event, window, cx| this.on_input(None, input, event, window, cx),
        ));
        let id = self.next_row();
        self.websites.push(WebsiteRow {
            id,
            origin,
            input: input.clone(),
        });
        input
    }

    fn add_field_row(&mut self, text: &FieldText, window: &mut Window, cx: &mut Context<Self>) {
        let mut line =
            |placeholder: &'static str, value: &str, secret: bool, cx: &mut Context<Self>| {
                let input = new_input(placeholder, secret, window, cx);
                input.update(cx, |input, cx| {
                    input.set_value(value.to_string(), window, cx)
                });
                self._subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    |this, input, event, window, cx| this.on_input(None, input, event, window, cx),
                ));
                input
            };
        let name = line("Name", &text.field.name, false, cx);
        let value = match &text.field.value {
            FieldValue::Text(value) => FieldInput::Text(line("Value", value, false, cx)),
            FieldValue::Hidden(value) => {
                FieldInput::Hidden(line("Value", value.expose(), true, cx))
            }
            FieldValue::Boolean(on) => FieldInput::Boolean(*on),
            FieldValue::Linked(id) => FieldInput::Linked(*id),
        };
        let id = self.next_row();
        self.fields.push(FieldRow {
            id,
            origin: text.origin,
            name,
            value,
        });
    }

    /// The fields as they are now.
    fn texts(&self, cx: &App) -> Texts {
        Texts {
            values: self
                .inputs
                .iter()
                .map(|(key, input)| (*key, input.read(cx).value().to_string()))
                .collect(),
            notes: self.notes.read(cx).value().to_string(),
            websites: self
                .websites
                .iter()
                .map(|row| WebsiteText {
                    origin: row.origin,
                    uri: row.input.read(cx).value().to_string(),
                })
                .collect(),
            fields: self.fields.iter().map(|row| row.text(cx)).collect(),
        }
    }

    fn on_input(
        &mut self,
        key: Option<Key>,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change if key == Some(Key::Password) => {
                self.generated = false;
                self.take_generated = false;
                cx.notify();
            }
            InputEvent::Change if key == Some(Key::Name) => {
                self.problem = None;
                cx.notify();
            }
            InputEvent::PressEnter { .. } if Some(input) == self.new_folder.as_ref() => {
                self.create_folder(window, cx)
            }
            InputEvent::PressEnter { .. } => self.save(&Save, window, cx),
            // A secret shows while it has the keyboard.
            InputEvent::Focus | InputEvent::Blur => {
                let focused = matches!(event, InputEvent::Focus);
                let masked = match key {
                    Some(Key::Password) => !focused && !self.password_visible(),
                    _ if self.is_secret(input) => !focused,
                    _ => return,
                };
                input.update(cx, |input, cx| input.set_masked(masked, window, cx));
                cx.notify();
            }
            InputEvent::Change => {}
        }
    }

    /// Whether `input` holds a secret, a hidden custom field.
    fn is_secret(&self, input: &Entity<InputState>) -> bool {
        SECRETS.iter().any(|key| self.input(*key) == input)
            || self
                .fields
                .iter()
                .any(|row| matches!(&row.value, FieldInput::Hidden(hidden) if hidden == input))
    }

    /// Whether the password shows while it doesn't have the keyboard: a
    /// generated one does, as does one the eye showed.
    fn password_visible(&self) -> bool {
        self.generated || self.password_shown
    }

    fn toggle_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.password_shown = !self.password_shown;
        let password = self.input(Key::Password).clone();
        let focused = password.read(cx).focus_handle(cx).is_focused(window);
        let masked = !focused && !self.password_visible();
        password.update(cx, |input, cx| input.set_masked(masked, window, cx));
        cx.notify();
    }

    fn take_generated(&mut self, value: &Secret, window: &mut Window, cx: &mut Context<Self>) {
        if !self.take_generated || self.kind != ItemKind::Login {
            return;
        }
        self.input(Key::Password).clone().update(cx, |input, cx| {
            input.set_value(value.expose().to_string(), window, cx);
            input.set_masked(false, window, cx);
        });
        self.generated = true;
        cx.notify();
    }

    fn regenerate(&mut self, cx: &mut Context<Self>) {
        self.take_generated = true;
        self.generator.update(cx, Generator::regenerate);
    }

    fn toggle_generator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generator_open = !self.generator_open;
        if self.generator_open {
            // Changing an option makes a new password for this field.
            self.take_generated = true;
            window.focus(&self.generator_focus, cx);
        } else {
            window.focus(&self.focus_handle, cx);
        }
        cx.notify();
    }

    fn close_generator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generator_open = false;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn set_kind(&mut self, kind: ItemKind, cx: &mut Context<Self>) {
        if self.kind == kind {
            return;
        }
        self.kind = kind;
        // The first switch to Login makes its password.
        if kind == ItemKind::Login
            && self.take_generated
            && self.value(Key::Password, cx).is_empty()
        {
            self.generator.update(cx, Generator::regenerate);
        }
        cx.notify();
    }

    /// Whether a group of fields shows: it holds something, or was added.
    fn shows(&self, extra: Extra, cx: &App) -> bool {
        self.added.contains(&extra)
            || extra
                .keys()
                .iter()
                .any(|key| !self.value(*key, cx).is_empty())
    }

    fn add_extra(&mut self, extra: Extra, window: &mut Window, cx: &mut Context<Self>) {
        self.close_add_menu(window, cx);
        self.added.push(extra);
        let first = extra.keys()[0];
        self.input(first)
            .clone()
            .update(cx, |input, cx| input.focus(window, cx));
    }

    fn add_website_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_add_menu(window, cx);
        let input = self.add_website(None, "", window, cx);
        input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn remove_website(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.websites.retain(|row| row.id != id);
        if self.websites.is_empty() {
            self.add_website(None, "", window, cx);
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn add_custom_field(&mut self, kind: NewField, window: &mut Window, cx: &mut Context<Self>) {
        self.close_add_menu(window, cx);
        let value = match kind {
            NewField::Text => FieldValue::Text(String::new()),
            NewField::Hidden => FieldValue::Hidden(Secret::default()),
            NewField::Boolean => FieldValue::Boolean(false),
        };
        let text = FieldText {
            origin: None,
            field: CustomField {
                name: String::new(),
                value,
            },
        };
        self.add_field_row(&text, window, cx);
        if let Some(row) = self.fields.last() {
            row.name
                .clone()
                .update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn remove_field(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.fields.retain(|row| row.id != id);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn set_checkbox(&mut self, id: usize, on: bool, cx: &mut Context<Self>) {
        if let Some(row) = self.fields.iter_mut().find(|row| row.id == id) {
            row.value = FieldInput::Boolean(on);
            cx.notify();
        }
    }

    fn toggle_add_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.add_menu_open {
            self.close_add_menu(window, cx);
        } else {
            self.add_menu_open = true;
            window.focus(&self.add_menu_focus, cx);
            cx.notify();
        }
    }

    fn close_add_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_menu_open = false;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.close(None, window, cx);
    }

    fn close(&mut self, saved: Option<ItemId>, window: &mut Window, cx: &mut Context<Self>) {
        self.vault
            .update(cx, |vault, cx| vault.dismiss_sheet(saved, window, cx))
            .ok();
    }

    /// The item to save, with what changed in the fields.
    fn item_data(&self, cx: &App) -> ItemData {
        let original = match &self.editing {
            Some(item) => item.data.clone(),
            None => ItemData::new(self.kind),
        };
        let mut data = form::apply(&original, &self.initial, &self.texts(cx));
        data.folder = self.folder.clone();
        data
    }

    fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let data = self.item_data(cx);
        if data.name.trim().is_empty() {
            self.problem = Some("Give the item a name.");
            self.focus(window, cx);
            cx.notify();
            return;
        }
        let favorite = self.favorite;
        let editing = self
            .editing
            .as_ref()
            .map(|item| (item.id.clone(), item.favorite, item.data != data));
        if let Some((_, was_favorite, false)) = editing
            && was_favorite == favorite
        {
            // Nothing changed, so nothing is sent.
            self.close(None, window, cx);
            return;
        }
        self.saving = true;
        cx.notify();
        let account = self.account.clone();
        self.task = backend::request_in(
            window,
            cx,
            move |backend| {
                let (id, was_favorite) = match editing {
                    Some((id, was_favorite, changed)) => {
                        if changed {
                            backend.edit_item(&account, &id, &data)?;
                        }
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
            .map_or_else(|| "No Folder".to_string(), |folder| folder.name.clone())
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
        let input = new_input("Folder name", false, window, cx);
        self._subscriptions.push(cx.subscribe_in(
            &input,
            window,
            |this, input, event, window, cx| this.on_input(None, input, event, window, cx),
        ));
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
        let name = input.read(cx).value().trim().to_string();
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

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = &self.editing else { return };
        let (id, name) = (item.id.clone(), item.data.name.clone());
        self.vault
            .update(cx, |vault, cx| vault.delete_item(id, name, window, cx))
            .ok();
    }
}

/// Every field the editor has a line for, of every kind: a new item can
/// change its kind, and an item edited keeps the others empty.
const ALL_KEYS: [Key; 26] = [
    Key::Name,
    Key::Username,
    Key::Password,
    Key::Authenticator,
    Key::Cardholder,
    Key::Brand,
    Key::Number,
    Key::Expiry,
    Key::SecurityCode,
    Key::Title,
    Key::FirstName,
    Key::MiddleName,
    Key::LastName,
    Key::Email,
    Key::Phone,
    Key::IdentityUsername,
    Key::Address1,
    Key::Address2,
    Key::Address3,
    Key::City,
    Key::State,
    Key::PostalCode,
    Key::Country,
    Key::Ssn,
    Key::Passport,
    Key::License,
];

/// What an empty field shows, where its label alone doesn't say enough.
fn placeholder(key: Key) -> &'static str {
    match key {
        Key::Name => "Required",
        Key::Expiry => "MM/YYYY",
        Key::Email => "name@example.com",
        _ => "",
    }
}

fn new_input(
    placeholder: &'static str,
    secret: bool,
    window: &mut Window,
    cx: &mut Context<ItemEditor>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(secret)
    })
}

impl Focusable for ItemEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ItemEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_sheet(window, cx)
    }
}
