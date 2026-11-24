//! The vault window: sidebar, item list and the selected item.

mod context_menu;
mod detail;
mod editor;
mod list;
mod sidebar;

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use gpui::base::input::{InputEvent, InputState};
use gpui::{
    App, AppContext as _, Context, DragMoveEvent, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, ScrollStrategy,
    Styled as _, Subscription, Task, UniformListScrollHandle, Window, actions, div, px,
};
use rbw_desktop_core::{
    Account, AccountId, Error, Folder, FolderId, Item, ItemId, ItemKind, ItemSummary, SortOrder,
};
use rbw_desktop_ui::{Confirmation, SheetSlot, ToastSlot, motion};

use crate::accounts::{AccountMenu, Request};
use crate::backend::{self, Load};
use crate::generator::Generator;
use crate::icons::{self, Icons};
use crate::preferences::{self, Prefs};
use crate::recent;
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
        /// Clears the search, or leaves its field for the list.
        ClearSearch,
        SelectNext,
        SelectPrevious,
        /// Syncs the vault with the server now.
        Sync,
        Lock,
        /// Opens the selected item's context menu from the keyboard.
        OpenContextMenu,
        /// Shows the password generator.
        ShowGenerator,
        /// Copies the selected login's password.
        CopyPassword,
        /// Copies the selected login's username.
        CopyUsername,
        /// Copies the selected login's verification code.
        CopyCode,
    ]
);

const CONTEXT: &str = "Vault";

/// The item list's share of the room right of the sidebar, unless it was
/// moved, and the most it can take.
const LIST_SHARE: f32 = 0.4;
const MAX_LIST_SHARE: f32 = 0.6;
/// How long the divider between the list and the item rests after a drag
/// before where it was left is saved.
const SAVE_DIVIDER_AFTER: Duration = Duration::from_millis(400);

/// How many of the list's first rows have their websites' icons read with
/// it, so that they show with it: a tall window's worth.
const FIRST_ROWS: usize = 40;

/// What the divider between the list and the item drags.
struct ListDivider;

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(secondary!("n"), NewItem, Some(CONTEXT)),
        KeyBinding::new(secondary!("e"), EditItem, Some(CONTEXT)),
        KeyBinding::new(secondary!("backspace"), DeleteItem, Some("VaultList")),
        KeyBinding::new(secondary!("f"), Find, Some(CONTEXT)),
        KeyBinding::new("escape", ClearSearch, Some("Vault > Input")),
        KeyBinding::new("escape", ClearSearch, Some("VaultList")),
        KeyBinding::new(secondary!("r"), Sync, Some(CONTEXT)),
        KeyBinding::new(secondary!("l"), Lock, Some(CONTEXT)),
        KeyBinding::new(secondary!("shift-g"), ShowGenerator, Some(CONTEXT)),
        // As in the menu bar panel; Copy itself copies the password where
        // no text field has the keyboard.
        KeyBinding::new(secondary!("c"), CopyPassword, Some("VaultList")),
        KeyBinding::new(secondary!("c"), CopyPassword, Some("ItemDetail")),
        KeyBinding::new(secondary!("shift-c"), CopyUsername, Some(CONTEXT)),
        KeyBinding::new(secondary!("alt-c"), CopyCode, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some("VaultList")),
        KeyBinding::new("up", SelectPrevious, Some("VaultList")),
        KeyBinding::new("shift-f10", OpenContextMenu, Some("VaultList")),
        KeyBinding::new("menu", OpenContextMenu, Some("VaultList")),
    ]);
    editor::init(cx);
}

/// Something of a login to copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Username,
    Password,
    Code,
}

/// What the sidebar selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Favorites,
    Kind(ItemKind),
    Folder(FolderId),
    /// Items in no folder.
    NoFolder,
}

impl Filter {
    fn admits(&self, item: &ItemSummary) -> bool {
        match self {
            Filter::All => true,
            Filter::Favorites => item.favorite,
            Filter::Kind(kind) => item.kind == *kind,
            Filter::Folder(folder) => item.folder.as_ref() == Some(folder),
            Filter::NoFolder => item.folder.is_none(),
        }
    }
}

/// What the list's menu narrows it to, on top of the sidebar's choice:
/// what the sidebar doesn't offer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Only {
    /// Logins with a verification code.
    pub code: bool,
    /// Items shared with the account through an organization.
    pub shared: bool,
}

impl Only {
    fn admits(self, item: &ItemSummary) -> bool {
        (!self.code || item.has_code) && (!self.shared || item.organization.is_some())
    }

    fn any(self) -> bool {
        self.code || self.shared
    }
}

/// The items, as indices into `items`, that the sidebar's choice, the
/// menu's and a search in lower case admit, in `order`.
fn visible(
    items: &[ItemSummary],
    filter: &Filter,
    only: Only,
    query: &str,
    order: SortOrder,
    recent: &[ItemId],
) -> Vec<usize> {
    let mut visible: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| filter.admits(item) && only.admits(item) && matches(item, query))
        .map(|(index, _)| index)
        .collect();
    sort(&mut visible, items, order, recent);
    visible
}

/// Puts the list's items, given as indices into `items`, in `order`.
/// `items` are sorted by name already, and ties keep that order.
fn sort(visible: &mut [usize], items: &[ItemSummary], order: SortOrder, recent: &[ItemId]) {
    match order {
        SortOrder::Name => {}
        SortOrder::Modified => {
            visible.sort_by_key(|&index| std::cmp::Reverse(items[index].modified));
        }
        SortOrder::RecentlyUsed => {
            let rank: std::collections::HashMap<&ItemId, usize> = recent
                .iter()
                .enumerate()
                .map(|(rank, id)| (id, rank))
                .collect();
            visible.sort_by_key(|&index| rank.get(&items[index].id).copied().unwrap_or(usize::MAX));
        }
    }
}

pub struct VaultView {
    account: Account,
    /// Every account on this computer, for the account menu.
    accounts: Vec<Account>,
    account_menu_open: bool,
    account_menu_focus: FocusHandle,
    /// GNOME's primary menu, in the sidebar's header bar.
    main_menu_open: bool,
    main_menu_focus: FocusHandle,
    items: Load<Vec<ItemSummary>>,
    /// The items that weren't in the list before it last loaded, such as
    /// one just created, whose rows fade in.
    arrived: HashSet<ItemId>,
    folders: Vec<Folder>,
    /// The items used last, as they were when the vault loaded, for
    /// sorting by them; using an item doesn't move it while it shows.
    recent: Vec<ItemId>,
    filter: Filter,
    only: Only,
    /// What the line under the search field says the list shows only,
    /// kept while it closes.
    only_shown: Only,
    /// The order the list was last sorted in, to sort it again when the
    /// preferences, read after it loaded, say otherwise.
    sorted_by: SortOrder,
    sort_menu_open: bool,
    sort_menu_focus: FocusHandle,
    search: Entity<InputState>,
    /// Indices into the loaded items that pass the filter and the search,
    /// in display order.
    visible: Vec<usize>,
    /// The rows whose icons were last read ahead.
    prefetched: std::ops::Range<usize>,
    /// The rows the list drew last, which its selection comes in from the
    /// edge of.
    drew: std::ops::Range<usize>,
    selected: Option<ItemId>,
    /// Where the selection's highlight is drawn, in rows, as it slides.
    highlight_at: Option<f32>,
    /// The list changed under the selection, which then doesn't slide.
    highlight_jumps: bool,
    detail: Entity<ItemDetail>,
    sheet: SheetSlot,
    toast: ToastSlot,
    list_focus: FocusHandle,
    list_scroll: UniformListScrollHandle,
    context_menu: Option<ContextMenu>,
    /// The item something was copied of from its context menu, whose row
    /// says so for a moment.
    copied: Option<ItemId>,
    copied_timer: Task<()>,
    /// Fetches an item for the context menu; a newer one replaces it.
    menu_task: Task<()>,
    context_menu_focus: FocusHandle,
    /// The generator's popover, kept while it leaves.
    generator: Option<Entity<Generator>>,
    generator_open: bool,
    generator_focus: FocusHandle,
    generator_task: Task<()>,
    last_sync: Option<SystemTime>,
    load_task: Task<()>,
    /// The list's share while its divider is dragged, until it is saved.
    dragged_share: Option<f32>,
    save_share_task: Task<()>,
    _subscriptions: [Subscription; 2],
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
            main_menu_open: false,
            main_menu_focus: cx.focus_handle(),
            items: Load::Loading,
            arrived: HashSet::new(),
            folders: vec![],
            recent: vec![],
            filter: Filter::All,
            only: Only::default(),
            only_shown: Only::default(),
            sorted_by: SortOrder::Name,
            sort_menu_open: false,
            sort_menu_focus: cx.focus_handle(),
            search,
            visible: vec![],
            prefetched: 0..0,
            drew: 0..0,
            selected: None,
            highlight_at: None,
            highlight_jumps: false,
            detail,
            sheet: SheetSlot::default(),
            toast: ToastSlot::default(),
            list_focus: cx.focus_handle(),
            list_scroll: UniformListScrollHandle::new(),
            context_menu: None,
            copied: None,
            copied_timer: Task::ready(()),
            menu_task: Task::ready(()),
            context_menu_focus: cx.focus_handle(),
            generator: None,
            generator_open: false,
            generator_focus: cx.focus_handle(),
            generator_task: Task::ready(()),
            load_task: Task::ready(()),
            dragged_share: None,
            save_share_task: Task::ready(()),
            _subscriptions: [
                cx.observe(&Icons::global(cx), |_, _, cx| cx.notify()),
                cx.observe(&Prefs::global(cx), |this, prefs, cx| {
                    if prefs.read(cx).get().sort != this.sorted_by {
                        this.refilter(cx);
                    }
                    cx.notify();
                }),
            ],
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
        if account == self.account && accounts == self.accounts {
            return;
        }
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

    /// The search, in lower case.
    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_lowercase()
    }

    /// Loads the items and folders again, keeping the selection if it still
    /// exists. The first time, the icons of the first rows come with them.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let account = self.account_id();
        let first_load = matches!(self.items, Load::Loading);
        let (filter, only, query) = (self.filter.clone(), self.only, self.query(cx));
        let order = Prefs::global(cx).read(cx).get().sort;
        self.load_task = backend::request(
            cx,
            move |backend| {
                let mut items = backend.items(&account)?;
                items.sort_by_cached_key(|item| item.name.to_lowercase());
                let folders = backend.folders(&account)?;
                // Without them, the list is still worth showing.
                let recent = backend.recent_items(&account).unwrap_or_default();
                let icons = first_load.then(|| {
                    let rows = visible(&items, &filter, only, &query, order, &recent);
                    let first = rows.into_iter().take(FIRST_ROWS).map(|index| &items[index]);
                    icons::read_cached(backend, &account, first)
                });
                Ok((items, folders, recent, icons))
            },
            |this, result, cx| {
                match result {
                    Ok((items, folders, recent, icons)) => {
                        if let Some(icons) = icons {
                            Icons::global(cx)
                                .update(cx, |known, cx| known.insert_cached(icons, cx));
                        }
                        this.arrived = match &this.items {
                            Load::Ready(before) => {
                                let before: HashSet<&ItemId> =
                                    before.iter().map(|item| &item.id).collect();
                                items
                                    .iter()
                                    .filter(|item| !before.contains(&item.id))
                                    .map(|item| item.id.clone())
                                    .collect()
                            }
                            Load::Loading | Load::Failed(_) => HashSet::new(),
                        };
                        this.items = Load::Ready(items);
                        this.folders = folders;
                        this.recent = recent;
                    }
                    Err(error) => this.items = Load::Failed(error),
                }
                this.refilter(cx);
            },
        );
    }

    /// Lists the items the sidebar and the search admit, keeping the
    /// selection if it is among them, else selecting the first. Until the
    /// items are known, the selection stays as it is, such as an item
    /// shown from the menu bar panel as the vault opens.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let order = Prefs::global(cx).read(cx).get().sort;
        self.sorted_by = order;
        self.visible = match &self.items {
            Load::Ready(items) => {
                visible(items, &self.filter, self.only, &query, order, &self.recent)
            }
            Load::Loading => return,
            Load::Failed(_) => vec![],
        };
        self.prefetched = 0..0;
        self.highlight_jumps = true;
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

    /// Narrows the list to what the menu's choices admit.
    pub(crate) fn set_only(&mut self, only: Only, cx: &mut Context<Self>) {
        if self.only != only {
            self.only = only;
            self.refilter(cx);
            self.list_scroll.scroll_to_item(0, ScrollStrategy::Top);
        }
    }

    pub(crate) fn set_sort(&mut self, order: SortOrder, cx: &mut Context<Self>) {
        preferences::update(cx, move |preferences| preferences.sort = order);
        self.refilter(cx);
        self.list_scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    fn select(&mut self, item: Option<ItemId>, cx: &mut Context<Self>) {
        self.select_by(item, false, cx);
    }

    /// Selects an item; `by_pointer` when it was clicked, which counts as
    /// using it.
    fn select_by(&mut self, item: Option<ItemId>, by_pointer: bool, cx: &mut Context<Self>) {
        if by_pointer && let Some(item) = &item {
            recent::used(&self.account.id, item, cx);
        }
        if self.selected == item {
            if by_pointer {
                self.detail
                    .update(cx, |detail, cx| detail.focus_reprompt(cx));
            }
            return;
        }
        self.selected = item.clone();
        let summary = item.and_then(|id| self.summary(&id).cloned());
        // A selection that moves with the keys or the search is shown once
        // it rests, so that items only passed over aren't fetched with
        // their secrets.
        self.detail.update(cx, |detail, cx| {
            if by_pointer {
                detail.show(summary, true, cx)
            } else {
                detail.show_when_settled(summary, cx)
            }
        });
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

    /// Escape: empties the search, showing every item again with the
    /// selection kept; with nothing to clear, the list takes the keyboard.
    fn clear_search(&mut self, _: &ClearSearch, window: &mut Window, cx: &mut Context<Self>) {
        if self.query(cx).is_empty() {
            window.focus(&self.list_focus, cx);
        } else {
            self.search
                .update(cx, |search, cx| search.set_value("", window, cx));
            self.refilter(cx);
        }
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

    /// Edits the selected item, once it has arrived if it is on the way.
    fn edit_item(&mut self, _: &EditItem, window: &mut Window, cx: &mut Context<Self>) {
        let detail = self.detail.read(cx);
        if let Some(item) = detail.item().cloned() {
            self.open_editor(item, window, cx);
        } else if let Some(id) = detail.selected().map(|item| item.id.clone()) {
            self.fetch_and_edit(id, window, cx);
        }
    }

    /// Fetches the item, then edits it, unless a sheet opened meanwhile.
    fn fetch_and_edit(&mut self, id: ItemId, window: &mut Window, cx: &mut Context<Self>) {
        self.with_item(id, window, cx, |this, item, window, cx| {
            if !this.sheet.is_open() {
                this.open_editor(item, window, cx);
            }
        });
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
            recent::used(&self.account.id, &item, cx);
            self.select(Some(item), cx);
            self.detail.update(cx, |detail, cx| detail.reload(cx));
            self.reload(cx);
        }
        cx.notify();
    }

    /// Shows an item from elsewhere, such as the menu bar panel, as if it
    /// was clicked, in the middle of the list: whatever the search, the
    /// sidebar and the list's menu showed, and whether the items have
    /// arrived yet or not.
    pub(crate) fn reveal(&mut self, item: ItemId, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        self.filter = Filter::All;
        self.only = Only::default();
        window.focus(&self.list_focus, cx);
        self.select_by(Some(item), true, cx);
        self.refilter(cx);
        if let Some(index) = self.selected_index() {
            self.list_scroll
                .scroll_to_item(index, ScrollStrategy::Center);
        }
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
        let locked = account.clone();
        backend::request(
            cx,
            move |backend| backend.lock(&account),
            |this, result, cx| match result {
                Ok(()) => {
                    cx.emit(Request::Locked(locked));
                    crate::accounts::refresh(cx);
                }
                Err(error) => this.show_error(&error, cx),
            },
        )
        .detach();
    }

    fn delete_selected(&mut self, _: &DeleteItem, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = self.detail.read(cx).selected().filter(|item| item.editable) {
            let (id, name) = (item.id.clone(), item.name.clone());
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
        crate::alerts::confirm(confirmation, window, cx, move |window, cx| {
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
                    let neighbour = this.selected_index().and_then(|index| {
                        let items: Vec<_> =
                            this.visible_items().map(|item| item.id.clone()).collect();
                        neighbour(&items, index)
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

    fn copy_password(&mut self, _: &CopyPassword, _: &mut Window, cx: &mut Context<Self>) {
        self.detail
            .update(cx, |detail, cx| detail.copy_part(Part::Password, cx));
    }

    fn copy_username(&mut self, _: &CopyUsername, _: &mut Window, cx: &mut Context<Self>) {
        self.detail
            .update(cx, |detail, cx| detail.copy_part(Part::Username, cx));
    }

    fn copy_code(&mut self, _: &CopyCode, _: &mut Window, cx: &mut Context<Self>) {
        self.detail
            .update(cx, |detail, cx| detail.copy_part(Part::Code, cx));
    }

    fn show_generator(&mut self, _: &ShowGenerator, window: &mut Window, cx: &mut Context<Self>) {
        if self.generator_open {
            self.close_generator(window, cx);
            return;
        }
        let generator = cx.new(|cx| Generator::new(true, cx));
        window.focus(&generator.focus_handle(cx), cx);
        self.generator = Some(generator);
        self.generator_open = true;
        cx.notify();
    }

    fn close_generator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generator_open = false;
        window.focus(&self.list_focus, cx);
        // Once the popover has left, the generator goes, and the last
        // value it made with it.
        self.generator_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(motion::DISMISS).await;
            this.update(cx, |this, cx| {
                if !this.generator_open {
                    this.generator = None;
                    cx.notify();
                }
            })
            .ok();
        });
        cx.notify();
    }

    /// The item list's share of the room right of the sidebar.
    fn list_share(&self, cx: &App) -> f32 {
        self.dragged_share
            .or(Prefs::global(cx).read(cx).get().list_share)
            .unwrap_or(LIST_SHARE)
    }

    /// Follows the divider between the list and the item, and saves where it
    /// is once it rests. The list keeps its minimum width and leaves the
    /// item its own.
    fn drag_divider(
        &mut self,
        event: &DragMoveEvent<ListDivider>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let room = event.bounds.size.width;
        if room <= px(0.) {
            return;
        }
        let share = ((event.event.position.x - event.bounds.left()) / room)
            .clamp(list::MIN_WIDTH / room, MAX_LIST_SHARE);
        if self.dragged_share == Some(share) {
            return;
        }
        self.dragged_share = Some(share);
        cx.notify();
        self.save_share_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DIVIDER_AFTER).await;
            this.update(cx, |this, cx| this.save_list_share(Some(share), cx))
                .ok();
        });
    }

    /// Puts the divider back where it starts, on a double-click.
    fn reset_list_share(&mut self, cx: &mut Context<Self>) {
        self.save_share_task = Task::ready(());
        self.save_list_share(None, cx);
    }

    fn save_list_share(&mut self, share: Option<f32>, cx: &mut Context<Self>) {
        preferences::update(cx, move |preferences| preferences.list_share = share);
        self.dragged_share = None;
        cx.notify();
    }

    pub(crate) fn show_error(&mut self, error: &Error, cx: &mut Context<Self>) {
        self.toast.show(
            backend::describe(error),
            |this: &mut Self| &mut this.toast,
            cx,
        );
    }
}

/// The item that takes the place of the one at `index` when it goes: the
/// next, or the one before it at the end of the list.
fn neighbour(items: &[ItemId], index: usize) -> Option<ItemId> {
    items
        .get(index + 1)
        .or_else(|| index.checked_sub(1).and_then(|before| items.get(before)))
        .cloned()
}

/// Whether a search, in lower case, finds the item.
pub(crate) fn matches(item: &ItemSummary, query: &str) -> bool {
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
        // Put back, the list's edge settles where it starts.
        let held = self.dragged_share.is_some();
        let share = motion::settle("list-share", self.list_share(cx), held, window, cx);
        let list = self.render_list(share, window, cx);
        let divider = self.render_divider(share, cx);
        let sheet = self.sheet.render(px(360.), window, cx);
        let toast = self.toast.render(window, cx);
        // The sheet is outside the vault's key context, so that none of the
        // vault's shortcuts act behind it.
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .key_context(CONTEXT)
                    .on_action(cx.listener(Self::new_item))
                    .on_action(cx.listener(Self::edit_item))
                    .on_action(cx.listener(Self::delete_selected))
                    .on_action(cx.listener(Self::find))
                    .on_action(cx.listener(Self::clear_search))
                    .on_action(cx.listener(Self::select_next))
                    .on_action(cx.listener(Self::select_previous))
                    .on_action(cx.listener(Self::sync))
                    .on_action(cx.listener(Self::lock))
                    .on_action(cx.listener(Self::open_context_menu_from_keyboard))
                    .on_action(cx.listener(Self::show_generator))
                    .on_action(cx.listener(Self::copy_password))
                    .on_action(cx.listener(Self::copy_username))
                    .on_action(cx.listener(Self::copy_code))
                    .size_full()
                    .flex()
                    .child(sidebar)
                    .child(
                        div()
                            .id("panes")
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .on_drag_move(cx.listener(Self::drag_divider))
                            .child(list)
                            .child(self.detail.clone())
                            .child(divider),
                    )
                    .children(self.render_context_menu(cx)),
            )
            .children(sheet)
            .children(toast)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, kind: ItemKind) -> ItemSummary {
        ItemSummary {
            id: ItemId(name.to_lowercase()),
            kind,
            name: name.into(),
            subtitle: None,
            websites: vec![],
            folder: None,
            organization: None,
            favorite: false,
            has_code: false,
            reprompt: false,
            editable: true,
            modified: None,
        }
    }

    #[test]
    fn a_search_finds_names_subtitles_and_websites() {
        let login = ItemSummary {
            subtitle: Some("Alex.Martin".into()),
            websites: vec!["https://Bank.example".into()],
            ..item("Bank", ItemKind::Login)
        };
        for query in ["", "ban", "alex.m", "bank.example"] {
            assert!(matches(&login, query), "{query}");
        }
        assert!(!matches(&login, "github"));
    }

    #[test]
    fn the_sidebar_admits_its_items() {
        let folder = FolderId("travel".into());
        let card = ItemSummary {
            favorite: true,
            folder: Some(folder.clone()),
            ..item("Visa", ItemKind::Card)
        };
        let note = item("Codes", ItemKind::SecureNote);
        assert!(Filter::All.admits(&card) && Filter::All.admits(&note));
        assert!(Filter::Favorites.admits(&card) && !Filter::Favorites.admits(&note));
        assert!(Filter::Kind(ItemKind::Card).admits(&card));
        assert!(!Filter::Kind(ItemKind::Card).admits(&note));
        assert!(Filter::Folder(folder.clone()).admits(&card));
        assert!(!Filter::Folder(folder).admits(&note));
    }

    #[test]
    fn the_list_sorts_by_name_change_or_use() {
        let at = |seconds| Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds));
        let items = [
            ItemSummary {
                modified: at(10),
                ..item("Amazon", ItemKind::Login)
            },
            ItemSummary {
                modified: at(30),
                ..item("Bank", ItemKind::Login)
            },
            ItemSummary {
                modified: at(20),
                ..item("Codes", ItemKind::SecureNote)
            },
        ];
        let sorted = |order, recent: &[ItemId]| {
            let mut visible = vec![0, 1, 2];
            sort(&mut visible, &items, order, recent);
            visible
        };
        assert_eq!(sorted(SortOrder::Name, &[]), [0, 1, 2]);
        assert_eq!(sorted(SortOrder::Modified, &[]), [1, 2, 0]);
        // Those used last first, then the others by name.
        let recent = [ItemId("codes".into())];
        assert_eq!(sorted(SortOrder::RecentlyUsed, &recent), [2, 0, 1]);
    }

    #[test]
    fn the_menu_shows_only_codes_or_shared_items() {
        let with_code = ItemSummary {
            has_code: true,
            ..item("GitHub", ItemKind::Login)
        };
        let shared = ItemSummary {
            organization: Some("family".into()),
            ..item("Netflix", ItemKind::Login)
        };
        let codes = Only {
            code: true,
            shared: false,
        };
        assert!(codes.admits(&with_code) && !codes.admits(&shared));
        let both = Only {
            code: true,
            shared: true,
        };
        assert!(!both.admits(&with_code) && !both.admits(&shared));
        assert!(Only::default().admits(&shared) && !Only::default().any());
        assert!(Filter::NoFolder.admits(&shared));
    }

    #[test]
    fn a_deleted_item_gives_its_place_to_the_next() {
        let items: Vec<_> = ["a", "b", "c"].map(|id| ItemId(id.into())).into();
        assert_eq!(neighbour(&items, 0), Some(ItemId("b".into())));
        assert_eq!(neighbour(&items, 2), Some(ItemId("b".into())));
        assert_eq!(neighbour(&items[..1], 0), None);
    }
}
