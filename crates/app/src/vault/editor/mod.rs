//! The sheet that creates an item or edits one: a header with the item's
//! tile and name, the kind of a new item chosen from tiles, then every
//! field the item view shows, the common ones first and the rest from "Add
//! Field". New logins get a generated password; the generator's options
//! open from the password's row. Saving writes back only what changed
//! ([`form`]); Escape asks before it throws changes away.

mod form;
mod sections;

use std::collections::BTreeMap;

use gpui::base::input::{InputEvent, InputState, TextareaState};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, KeyBinding, Render,
    ScrollHandle, Subscription, Task, WeakEntity, Window, actions,
};
use rbw_desktop_core::{
    AccountId, CustomField, Error, FieldValue, Folder, FolderId, Item, ItemContent, ItemData,
    ItemId, ItemKind, Secret,
};
use rbw_desktop_ui::{Confirmation, IconName};

use super::VaultView;
use crate::backend;
use crate::generator::{Generated, Generator};
use crate::icons::Icons;
use crate::reprompt::{Reprompt, Unlocked};
use crate::secondary;
use form::{FieldText, Key, SECRETS, Texts, WebsiteText};

actions!(item_editor, [Cancel, Save, CancelNewFolder]);

const CONTEXT: &str = "ItemEditor";
const NEW_FOLDER_CONTEXT: &str = "NewFolder";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new(secondary!("enter"), Save, Some(CONTEXT)),
        KeyBinding::new(secondary!("s"), Save, Some(CONTEXT)),
        KeyBinding::new("escape", CancelNewFolder, Some(NEW_FOLDER_CONTEXT)),
    ]);
}

/// The kinds the sheet can create, in the order of its choice of kind,
/// each with its glyph and the hue of its tile.
const KINDS: [(ItemKind, &str, IconName, f32); 5] = [
    (ItemKind::Login, "Login", IconName::Key, 215.),
    (ItemKind::Card, "Card", IconName::Card, 145.),
    (ItemKind::Identity, "Identity", IconName::Identity, 275.),
    (ItemKind::SecureNote, "Note", IconName::Note, 40.),
    (ItemKind::SshKey, "SSH Key", IconName::Terminal, 190.),
];

/// The kind's place among [`KINDS`].
fn kind_index(kind: ItemKind) -> usize {
    KINDS
        .iter()
        .position(|(k, ..)| *k == kind)
        .unwrap_or_default()
}

/// What a new item of a kind might be called, as its name's field shows.
fn name_example(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Login => "Netflix",
        ItemKind::Card => "Travel card",
        ItemKind::Identity => "Personal",
        ItemKind::SecureNote => "Wi-Fi at home",
        ItemKind::SshKey => "Work laptop",
    }
}

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

    /// The item in "Add Field": its name, what it is for, and its glyph.
    fn describe(self) -> (&'static str, &'static str, IconName) {
        match self {
            Extra::Authenticator => (
                "Verification Code",
                "The setup key the website gives",
                IconName::Clock,
            ),
            Extra::TitleAndMiddleName => (
                "Title and Middle Name",
                "Such as Dr, and a second name",
                IconName::Identity,
            ),
            Extra::Address => ("Address", "Street, city and country", IconName::Pin),
            Extra::Identification => (
                "Identity Documents",
                "Passport and license numbers",
                IconName::Badge,
            ),
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
    /// Goes with the row.
    _subscription: Subscription,
}

/// A custom field's row.
struct FieldRow {
    id: usize,
    origin: Option<usize>,
    name: Entity<InputState>,
    value: FieldInput,
    /// Go with the row.
    _subscriptions: Vec<Subscription>,
}

/// The name of a folder being created, with what follows its field.
struct NewFolder {
    input: Entity<InputState>,
    _subscription: Subscription,
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
    /// What the fields held first, so that only what changed is saved.
    initial: Texts,
    inputs: BTreeMap<Key, Entity<InputState>>,
    notes: Entity<TextareaState>,
    /// A new SSH key's private key.
    private_key: Entity<TextareaState>,
    websites: Vec<WebsiteRow>,
    fields: Vec<FieldRow>,
    next_row: usize,
    /// Groups shown though empty, which "Add Field" asked for.
    added: Vec<Extra>,
    add_menu_open: bool,
    add_menu_focus: FocusHandle,
    folders: Vec<Folder>,
    folder: Option<FolderId>,
    /// The folder the sheet opened with, to tell whether it changed.
    initial_folder: Option<FolderId>,
    folder_menu_open: bool,
    folder_menu_focus: FocusHandle,
    /// The name of a folder being created from the folder menu.
    new_folder: Option<NewFolder>,
    /// Whether the folder named is being created, which Return waits for.
    creating_folder: bool,
    folder_task: Task<()>,
    favorite: bool,
    /// Makes the password, with the options its popover shows.
    generator: Entity<Generator>,
    generator_open: bool,
    generator_focus: FocusHandle,
    /// Whether the password is still the generated one, which shows.
    generated: bool,
    /// Counts the values generated, each of which arrives in place.
    generations: usize,
    /// Whether a value the generator makes goes into the password: not
    /// after the person typed one, until they ask the generator again.
    take_generated: bool,
    /// Whether the eye shows the password while it doesn't have the
    /// keyboard.
    password_shown: bool,
    problem: Option<&'static str>,
    /// Whether Save found the name empty, which the name says until it is
    /// typed, and how many times, each of which shakes it.
    name_missing: usize,
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
        let kind = if KINDS.iter().any(|(k, ..)| *k == kind) {
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
        let shown = form::texts(&data);
        let new_item = editing.is_none();
        let mut subscriptions = vec![
            // The header's tile shows a website's icon once read from disk.
            cx.observe(&Icons::global(cx), |_, _, cx| cx.notify()),
        ];
        let mut inputs = BTreeMap::new();
        // An item edited keeps its kind: only its own fields have a line.
        let keys: &[Key] = if new_item { &ALL_KEYS } else { kind_keys(kind) };
        for &key in std::iter::once(&Key::Name).chain(keys) {
            let example = match key {
                Key::Name => name_example(kind),
                key => describe(key).2,
            };
            let input = new_input(example, SECRETS.contains(&key), window, cx);
            input.update(cx, |input, cx| input.set_value(shown.get(key), window, cx));
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
                .placeholder("Anything else to keep with it")
        });
        notes.update(cx, |notes, cx| {
            notes.set_value(shown.notes.expose(), window, cx)
        });
        let private_key = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 8)
                .placeholder("-----BEGIN OPENSSH PRIVATE KEY-----")
        });
        private_key.update(cx, |key, cx| {
            key.set_value(shown.private_key.expose(), window, cx)
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
            initial: Texts::default(),
            inputs,
            notes,
            private_key,
            websites: vec![],
            fields: vec![],
            next_row: 0,
            added: vec![],
            add_menu_open: false,
            add_menu_focus: cx.focus_handle(),
            folders,
            folder: data.folder.clone(),
            initial_folder: data.folder.clone(),
            folder_menu_open: false,
            folder_menu_focus: cx.focus_handle(),
            new_folder: None,
            creating_folder: false,
            folder_task: Task::ready(()),
            favorite,
            generator,
            generator_open: false,
            generator_focus: cx.focus_handle(),
            generated: false,
            generations: 0,
            take_generated: new_item,
            password_shown: false,
            problem: None,
            name_missing: 0,
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
        for website in &shown.websites {
            this.add_website(website.origin, &website.uri, window, cx);
        }
        if this.websites.is_empty() {
            this.add_website(None, "", window, cx);
        }
        for field in &shown.fields {
            this.add_field_row(field, window, cx);
        }
        // What the fields hold, which may differ from the item: a one-line
        // field drops tabs and line breaks.
        this.initial = this.texts(cx);
        this
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input(Key::Name)
            .update(cx, |name, cx| name.focus(window, cx));
    }

    /// The item being edited, if it isn't a new one.
    pub(super) fn editing(&self) -> Option<&ItemId> {
        self.editing.as_ref().map(|item| &item.id)
    }

    /// The line of `key`: of the item's kind, or of any while it is new.
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
        let subscription = cx.subscribe_in(&input, window, |this, input, event, window, cx| {
            this.on_input(None, input, event, window, cx)
        });
        let id = self.next_row();
        self.websites.push(WebsiteRow {
            id,
            origin,
            input: input.clone(),
            _subscription: subscription,
        });
        input
    }

    fn add_field_row(&mut self, text: &FieldText, window: &mut Window, cx: &mut Context<Self>) {
        let mut subscriptions = vec![];
        let mut line =
            |placeholder: &'static str, value: &str, secret: bool, cx: &mut Context<Self>| {
                let input = new_input(placeholder, secret, window, cx);
                input.update(cx, |input, cx| {
                    input.set_value(value.to_string(), window, cx)
                });
                subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    |this, input, event, window, cx| this.on_input(None, input, event, window, cx),
                ));
                input
            };
        let name = line("Field name", &text.field.name, false, cx);
        let value = match &text.field.value {
            FieldValue::Text(value) => FieldInput::Text(line("Value", value, false, cx)),
            FieldValue::Hidden(value) => {
                FieldInput::Hidden(line("Hidden value", value.expose(), true, cx))
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
            _subscriptions: subscriptions,
        });
    }

    /// The fields as they are now.
    fn texts(&self, cx: &App) -> Texts {
        Texts {
            values: self
                .inputs
                .iter()
                .map(|(key, input)| (*key, Secret::new(input.read(cx).value().to_string())))
                .collect(),
            notes: Secret::new(self.notes.read(cx).value().to_string()),
            private_key: Secret::new(self.private_key.read(cx).value().to_string()),
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
                self.name_missing = 0;
                cx.notify();
            }
            InputEvent::PressEnter { .. }
                if self
                    .new_folder
                    .as_ref()
                    .is_some_and(|folder| folder.input == *input) =>
            {
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
        SECRETS
            .iter()
            .any(|key| self.inputs.get(key) == Some(input))
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
        self.generations += 1;
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

    fn set_kind(&mut self, kind: ItemKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.kind == kind {
            return;
        }
        self.kind = kind;
        self.input(Key::Name).clone().update(cx, |name, cx| {
            name.set_placeholder(name_example(kind), window, cx)
        });
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

    /// Escape or Cancel: closes the sheet, after asking if that throws
    /// changes away.
    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_changes(cx) {
            self.close(None, window, cx);
            return;
        }
        let confirmation = Confirmation {
            title: if self.editing.is_some() {
                "Discard your changes?"
            } else {
                "Discard this new item?"
            }
            .into(),
            message: "What you typed in it is lost.".into(),
            confirm: "Discard".into(),
            destructive: true,
        };
        let editor = cx.entity().downgrade();
        crate::alerts::confirm(confirmation, window, cx, move |window, cx| {
            editor
                .update(cx, |editor, cx| editor.close(None, window, cx))
                .ok();
        });
    }

    /// Whether saving would change anything: what Escape asks about. A
    /// password the sheet generated for a new login isn't a change of the
    /// person's.
    fn has_changes(&self, cx: &App) -> bool {
        let data = self.item_data(cx);
        match &self.editing {
            Some(item) => item.data != data || item.favorite != self.favorite,
            None => {
                let mut blank = ItemData {
                    folder: self.initial_folder.clone(),
                    ..ItemData::new(self.kind)
                };
                if self.generated
                    && let (ItemContent::Login(blank), ItemContent::Login(made)) =
                        (&mut blank.content, &data.content)
                {
                    blank.password = made.password.clone();
                }
                data != blank || self.favorite
            }
        }
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
            self.name_missing += 1;
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
        self.task = backend::request_network_in(
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
                    Ok(id) => {
                        crate::vault_cache::invalidate(&this.account, cx);
                        this.close(Some(id), window, cx);
                    }
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
        let subscription = cx.subscribe_in(&input, window, |this, input, event, window, cx| {
            this.on_input(None, input, event, window, cx)
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.new_folder = Some(NewFolder {
            input,
            _subscription: subscription,
        });
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

    /// Creates the folder named, once at a time, and not while the item
    /// saves: it has its own task, so that neither replaces the other.
    fn create_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || self.creating_folder {
            return;
        }
        let Some(folder) = &self.new_folder else {
            return;
        };
        let name = folder.input.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        self.creating_folder = true;
        let account = self.account.clone();
        self.folder_task = backend::request_network_in(
            window,
            cx,
            move |backend| backend.create_folder(&account, &name),
            |this, result, window, cx| {
                this.creating_folder = false;
                match result {
                    Ok(folder) => {
                        this.folder = Some(folder.id.clone());
                        this.folders.push(folder);
                        this.new_folder = None;
                        window.focus(&this.focus_handle, cx);
                        crate::vault_cache::invalidate(&this.account, cx);
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

/// The fields of a kind, besides the name.
fn kind_keys(kind: ItemKind) -> &'static [Key] {
    match kind {
        ItemKind::Login => &[Key::Username, Key::Password, Key::Authenticator],
        ItemKind::Card => &[
            Key::Cardholder,
            Key::Brand,
            Key::Number,
            Key::Expiry,
            Key::SecurityCode,
        ],
        ItemKind::Identity => &[
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
        ],
        ItemKind::SecureNote | ItemKind::SshKey => &[],
    }
}

/// Every field the editor has a line for, of every kind, besides the
/// name: a new item can change its kind.
const ALL_KEYS: [Key; 25] = [
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

/// What a field is called, its glyph, and the example its empty field
/// shows. A new item's name has an example of its kind's
/// ([`name_example`]).
fn describe(key: Key) -> (&'static str, IconName, &'static str) {
    match key {
        Key::Name => ("Name", IconName::Edit, ""),
        Key::Username => ("Username", IconName::Identity, "name@example.com"),
        Key::Password => ("Password", IconName::Key, ""),
        Key::Authenticator => ("Setup key", IconName::Clock, "JBSW Y3DP EHPK 3PXP"),
        Key::Cardholder => ("Cardholder", IconName::Identity, "Alex Martin"),
        Key::Brand => ("Brand", IconName::Card, "Visa"),
        Key::Number => ("Number", IconName::Card, "1234 5678 9012 3456"),
        Key::Expiry => ("Expires", IconName::Calendar, "MM/YYYY"),
        Key::SecurityCode => ("Security code", IconName::Shield, "123"),
        Key::Title => ("Title", IconName::Identity, "Dr"),
        Key::FirstName => ("First name", IconName::Identity, "Alex"),
        Key::MiddleName => ("Middle name", IconName::Identity, "Jordan"),
        Key::LastName => ("Last name", IconName::Identity, "Martin"),
        Key::Email => ("Email", IconName::Mail, "name@example.com"),
        Key::Phone => ("Phone", IconName::Phone, "+1 555 0100"),
        Key::IdentityUsername => ("Username", IconName::Identity, "alexmartin"),
        Key::Address1 => ("Address", IconName::Pin, "1 Main Street"),
        Key::Address2 => ("Address 2", IconName::Pin, "Apartment 4"),
        Key::Address3 => ("Address 3", IconName::Pin, ""),
        Key::City => ("City", IconName::Pin, "Springfield"),
        Key::State => ("State", IconName::Pin, ""),
        Key::PostalCode => ("Postal code", IconName::Pin, "12345"),
        Key::Country => ("Country", IconName::Pin, ""),
        Key::Ssn => ("SSN", IconName::Badge, ""),
        Key::Passport => ("Passport", IconName::Badge, ""),
        Key::License => ("License", IconName::Badge, ""),
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
