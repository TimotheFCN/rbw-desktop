//! The vault window: sidebar, item list and the selected item.

mod context_menu;
mod detail;
mod editor;
mod list;
mod sidebar;

use std::time::SystemTime;

use gpui::base::input::{InputEvent, InputState};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, ScrollStrategy,
    Styled as _, Subscription, Task, UniformListScrollHandle, Window, actions, div, px,
};
use rbw_desktop_core::{
    Account, AccountId, Error, Folder, FolderId, Item, ItemId, ItemKind, ItemSummary,
};
use rbw_desktop_ui::{Confirmation, SheetSlot, ToastSlot};

use crate::accounts::{AccountMenu, Request};
use crate::backend::{self, Load};
use crate::icons::Icons;
use crate::secondary;
use context_menu::ContextMenu;
use detail::ItemDetail;
use editor::ItemEditor;

actions!(
    vault,
    [
        /// Opens the New Item sheet.
        NewItem,
        /// Opens the selected item in the edit sheet.
        EditItem,
        /// Asks, then moves the selected item to the trash.
        DeleteItem,
        /// Moves the keyboard to the search field.
        Find,
        SelectNext,
        SelectPrevious,
        /// Syncs the vault with the server now.
        Sync,
        Lock,
        /// Opens the selected item's context menu from the keyboard.
        OpenContextMenu,
    ]
);

const CONTEXT: &str = "Vault";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(secondary!("n"), NewItem, Some(CONTEXT)),
        KeyBinding::new(secondary!("e"), EditItem, Some(CONTEXT)),
        KeyBinding::new(secondary!("backspace"), DeleteItem, Some("VaultList")),
        KeyBinding::new(secondary!("f"), Find, Some(CONTEXT)),
        KeyBinding::new(secondary!("r"), Sync, Some(CONTEXT)),
        KeyBinding::new(secondary!("l"), Lock, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some("VaultList")),
        KeyBinding::new("up", SelectPrevious, Some("VaultList")),
        KeyBinding::new("shift-f10", OpenContextMenu, Some("VaultList")),
        KeyBinding::new("menu", OpenContextMenu, Some("VaultList")),
    ]);
    editor::init(cx);
}

/// What the sidebar selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Favorites,
    Kind(ItemKind),
    Folder(FolderId),
}

impl Filter {
    fn admits(&self, item: &ItemSummary) -> bool {
        match self {
            Filter::All => true,
            Filter::Favorites => item.favorite,
            Filter::Kind(kind) => item.kind == *kind,
            Filter::Folder(folder) => item.folder.as_ref() == Some(folder),
        }
    }
}

pub struct VaultView {
    account: Account,
    /// Every account on this computer, for the account menu.
    accounts: Vec<Account>,
    account_menu_open: bool,
    account_menu_focus: FocusHandle,
    items: Load<Vec<ItemSummary>>,
    folders: Vec<Folder>,
    filter: Filter,
    search: Entity<InputState>,
    /// Indices into the loaded items that pass the filter and the search,
    /// in display order.
    visible: Vec<usize>,
    selected: Option<ItemId>,
    detail: Entity<ItemDetail>,
    sheet: SheetSlot,
    toast: ToastSlot,
    list_focus: FocusHandle,
    list_scroll: UniformListScrollHandle,
    context_menu: Option<ContextMenu>,
    context_menu_focus: FocusHandle,
    last_sync: Option<SystemTime>,
    load_task: Task<()>,
    _icons: Subscription,
}

impl EventEmitter<Request> for VaultView {}

impl VaultView {
    pub fn new(
        account: Account,
        accounts: Vec<Account>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        cx.subscribe_in(
            &search,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.refilter(cx),
                InputEvent::PressEnter { .. } => {
                    window.focus(&this.list_focus, cx);
                    this.select_index(0, cx);
                }
                InputEvent::Focus | InputEvent::Blur => {}
            },
        )
        .detach();
        let vault = cx.entity().downgrade();
        let detail = cx.new(|cx| ItemDetail::new(account.id.clone(), vault, cx));
        let mut this = Self {
            last_sync: account.last_sync,
            account,
            accounts,
            account_menu_open: false,
            account_menu_focus: cx.focus_handle(),
            items: Load::Loading,
            folders: vec![],
            filter: Filter::All,
            search,
            visible: vec![],
            selected: None,
            detail,
            sheet: SheetSlot::default(),
            toast: ToastSlot::default(),
            list_focus: cx.focus_handle(),
            list_scroll: UniformListScrollHandle::new(),
            context_menu: None,
            context_menu_focus: cx.focus_handle(),
            load_task: Task::ready(()),
            _icons: cx.observe(&Icons::global(cx), |_, _, cx| cx.notify()),
        };
        this.reload(cx);
        window.focus(&this.list_focus, cx);
        this
    }

    pub fn account(&self) -> &AccountId {
        &self.account.id
    }

    fn account_id(&self) -> AccountId {
        self.account.id.clone()
    }

    /// Takes the account's latest state, and reloads the vault after a sync,
    /// whether this app, the command line or the agent ran it.
    pub fn set_account(
        &mut self,
        account: Account,
        accounts: Vec<Account>,
        cx: &mut Context<Self>,
    ) {
        let synced = account.last_sync != self.last_sync;
        self.last_sync = account.last_sync;
        self.account = account;
        self.accounts = accounts;
        if synced {
            self.reload(cx);
            self.detail.update(cx, |detail, cx| detail.reload(cx));
        }
        cx.notify();
    }

    /// Loads the items and folders again, keeping the selection if it still
    /// exists.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let account = self.account_id();
        self.load_task = backend::request(
            cx,
            move |backend| Ok((backend.items(&account)?, backend.folders(&account)?)),
            |this, result, cx| {
                match result {
                    Ok((mut items, folders)) => {
                        items.sort_by_cached_key(|item| item.name.to_lowercase());
                        this.items = Load::Ready(items);
                        this.folders = folders;
                    }
                    Err(error) => this.items = Load::Failed(error),
                }
                this.refilter(cx);
            },
        );
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().to_lowercase();
        let query = query.trim();
        self.visible = match &self.items {
            Load::Ready(items) => items
                .iter()
                .enumerate()
                .filter(|(_, item)| self.filter.admits(item) && matches(item, query))
                .map(|(index, _)| index)
                .collect(),
            Load::Loading | Load::Failed(_) => vec![],
        };
        match self.selected_index() {
            Some(index) => {
                self.list_scroll
                    .scroll_to_item(index, ScrollStrategy::Nearest);
                if let Some(item) = self.visible_items().nth(index).cloned() {
                    self.detail
                        .update(cx, |detail, cx| detail.set_summary(item, cx));
                }
            }
            None => {
                let first = self.visible_items().next().map(|item| item.id.clone());
                self.select(first, cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn focus_list(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.list_focus, cx);
    }

    fn summary(&self, id: &ItemId) -> Option<&ItemSummary> {
        self.items.ready()?.iter().find(|item| item.id == *id)
    }

    fn visible_items(&self) -> impl Iterator<Item = &ItemSummary> {
        let items = self.items.ready().map(Vec::as_slice).unwrap_or_default();
        self.visible.iter().map(move |&index| &items[index])
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        if self.filter != filter {
            self.filter = filter;
            self.refilter(cx);
            self.list_scroll.scroll_to_item(0, ScrollStrategy::Top);
        }
    }

    fn select(&mut self, item: Option<ItemId>, cx: &mut Context<Self>) {
        self.select_by(item, false, cx);
    }

    /// Selects an item; `by_pointer` when it was clicked.
    fn select_by(&mut self, item: Option<ItemId>, by_pointer: bool, cx: &mut Context<Self>) {
        if self.selected == item {
            if by_pointer {
                self.detail
                    .update(cx, |detail, cx| detail.focus_reprompt(cx));
            }
            return;
        }
        self.selected = item.clone();
        let summary = item.and_then(|id| self.summary(&id).cloned());
        self.detail
            .update(cx, |detail, cx| detail.show(summary, by_pointer, cx));
        cx.notify();
    }

    fn select_index(&mut self, index: usize, cx: &mut Context<Self>) {
        let item = self.visible_items().nth(index).map(|item| item.id.clone());
        if item.is_some() {
            self.select(item, cx);
            self.list_scroll
                .scroll_to_item(index, ScrollStrategy::Nearest);
        }
    }

    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.visible_items().position(|item| &item.id == selected)
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        let next = self.selected_index().map_or(0, |index| index + 1);
        self.select_index(next, cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.selected_index().and_then(|index| index.checked_sub(1)) {
            self.select_index(index, cx);
        }
    }

    fn find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |search, cx| search.focus(window, cx));
    }

    fn new_item(&mut self, _: &NewItem, window: &mut Window, cx: &mut Context<Self>) {
        let vault = cx.entity().downgrade();
        let folder = match &self.filter {
            Filter::Folder(folder) => Some(folder.clone()),
            _ => None,
        };
        let kind = match self.filter {
            Filter::Kind(kind) => kind,
            _ => ItemKind::Login,
        };
        let editor = cx.new(|cx| {
            ItemEditor::create(
                self.account_id(),
                kind,
                folder,
                self.folders.clone(),
                vault,
                window,
                cx,
            )
        });
        self.present(editor, window, cx);
    }

    fn edit_item(&mut self, _: &EditItem, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.detail.read(cx).item().cloned() else {
            return;
        };
        self.open_editor(item, window, cx);
    }

    fn open_editor(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        let vault = cx.entity().downgrade();
        let folders = self.folders.clone();
        let account = self.account_id();
        let editor = cx.new(|cx| ItemEditor::edit(account, item, folders, vault, window, cx));
        self.present(editor, window, cx);
    }

    fn present(&mut self, editor: Entity<ItemEditor>, window: &mut Window, cx: &mut Context<Self>) {
        editor.update(cx, |editor, cx| editor.focus(window, cx));
        self.sheet.present(editor);
        cx.notify();
    }

    /// Called by the editor when it is done, with the item it saved.
    pub(crate) fn dismiss_sheet(
        &mut self,
        saved: Option<ItemId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sheet.dismiss();
        window.focus(&self.list_focus, cx);
        if let Some(item) = saved {
            self.select(Some(item), cx);
            self.detail.update(cx, |detail, cx| detail.reload(cx));
            self.reload(cx);
        }
        cx.notify();
    }

    fn sync(&mut self, _: &Sync, _: &mut Window, cx: &mut Context<Self>) {
        let account = self.account_id();
        backend::request(
            cx,
            move |backend| backend.sync(&account),
            |this, result, cx| match result {
                Ok(()) => crate::accounts::refresh(cx),
                Err(error) => this.show_error(&error, cx),
            },
        )
        .detach();
    }

    fn lock(&mut self, _: &Lock, _: &mut Window, cx: &mut Context<Self>) {
        let account = self.account_id();
        backend::request(
            cx,
            move |backend| backend.lock(&account),
            |this, result, cx| match result {
                Ok(()) => crate::accounts::refresh(cx),
                Err(error) => this.show_error(&error, cx),
            },
        )
        .detach();
    }

    fn delete_selected(&mut self, _: &DeleteItem, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = self.detail.read(cx).item().filter(|item| item.editable) {
            let (id, name) = (item.id.clone(), item.data.name.clone());
            self.delete_item(id, name, window, cx);
        }
    }

    /// Asks, then moves the item to the trash and selects the one after it.
    pub(crate) fn delete_item(
        &mut self,
        id: ItemId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let confirmation = Confirmation {
            title: format!("Delete “{name}”?").into(),
            message: "It moves to the trash of your vault.".into(),
            confirm: "Delete".into(),
            destructive: true,
        };
        let vault = cx.entity().downgrade();
        crate::window::confirm(confirmation, window, cx, move |window, cx| {
            vault
                .update(cx, |vault, cx| vault.delete_confirmed(id, window, cx))
                .ok();
        });
    }

    fn delete_confirmed(&mut self, id: ItemId, window: &mut Window, cx: &mut Context<Self>) {
        let account = self.account_id();
        let deleted = id.clone();
        backend::request_in(
            window,
            cx,
            move |backend| backend.delete_item(&account, &id),
            move |this, result, window, cx| match result {
                Ok(()) => {
                    if this.sheet.is_open() {
                        this.dismiss_sheet(None, window, cx);
                    }
                    // The next item takes its place, or the one before it at
                    // the end of the list.
                    let position = this.selected_index();
                    let neighbour = position.and_then(|index| {
                        let items: Vec<_> =
                            this.visible_items().map(|item| item.id.clone()).collect();
                        items
                            .get(index + 1)
                            .or_else(|| index.checked_sub(1).and_then(|before| items.get(before)))
                            .cloned()
                    });
                    if this.selected.as_ref() == Some(&deleted) {
                        this.select(neighbour, cx);
                    }
                    this.reload(cx);
                }
                Err(error) => this.show_error(&error, cx),
            },
        )
        .detach();
    }

    pub(crate) fn show_error(&mut self, error: &Error, cx: &mut Context<Self>) {
        self.toast.show(
            backend::describe(error),
            |this: &mut Self| &mut this.toast,
            cx,
        );
    }
}

fn matches(item: &ItemSummary, query: &str) -> bool {
    query.is_empty()
        || item.name.to_lowercase().contains(query)
        || item
            .subtitle
            .as_ref()
            .is_some_and(|subtitle| subtitle.to_lowercase().contains(query))
        || item
            .websites
            .iter()
            .any(|website| website.to_lowercase().contains(query))
}

impl AccountMenu for VaultView {
    fn menu_open(&self) -> bool {
        self.account_menu_open
    }

    fn set_menu_open(&mut self, open: bool) {
        self.account_menu_open = open;
    }

    fn menu_focus(&self) -> &FocusHandle {
        &self.account_menu_focus
    }

    fn menu_return(&self, _: &App) -> FocusHandle {
        self.list_focus.clone()
    }
}

impl Focusable for VaultView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.list_focus.clone()
    }
}

impl Render for VaultView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = self.render_sidebar(window, cx);
        let list = self.render_list(window, cx);
        let sheet = self.sheet.render(px(360.), window, cx);
        let toast = self.toast.render(window, cx);
        div()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::new_item))
            .on_action(cx.listener(Self::edit_item))
            .on_action(cx.listener(Self::delete_selected))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::sync))
            .on_action(cx.listener(Self::lock))
            .on_action(cx.listener(Self::open_context_menu_from_keyboard))
            .relative()
            .size_full()
            .flex()
            .child(sidebar)
            .child(list)
            .child(self.detail.clone())
            .children(self.render_context_menu(cx))
            .children(sheet)
            .children(toast)
    }
}
