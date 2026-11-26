//! The vault window: sidebar, item list and the selected item.

mod context_menu;
mod detail;
mod editor;
mod list;
mod sidebar;
mod totp;

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui::base::input::{InputEvent, InputState};
use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, DragMoveEvent, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _,
    Render, ScrollStrategy, Styled as _, Subscription, Task, UniformListScrollHandle, WeakEntity,
    Window, actions, div, px,
};
use rbw_desktop_core::{
    Account, AccountId, Error, Folder, FolderId, Item, ItemId, ItemKind, ItemSummary, SortOrder,
};
use rbw_desktop_ui::{Confirmation, ListPointer, SheetSlot, motion};

use crate::accounts::{AccountMenu, Request};
use crate::backend::{self, Load};
use crate::copied::CopiedSlot;
use crate::generator::Generator;
use crate::icons::{self, Icons};
use crate::preferences::{self, Prefs};
use crate::recent;
use crate::secondary;
use crate::vault_cache::{self, VaultSnapshot};
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
/// How many rows, in all, have their icons read ahead for the sidebar's
/// other choices: well within the icons kept.
const OTHER_CHOICES_ROWS: usize = 200;

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

/// An item of a list, with what a search looks through, made once off
/// the main thread when the list loads rather than on every key typed.
#[derive(Clone)]
pub(crate) struct Indexed {
    pub summary: Arc<ItemSummary>,
    /// Its name, subtitle and websites in lower case, apart.
    search: String,
    /// Names the elements of its row without a string made for each.
    pub key: u64,
}

impl Indexed {
    pub fn new(summary: ItemSummary) -> Self {
        Self::from_shared(Arc::new(summary))
    }

    pub fn from_shared(summary: Arc<ItemSummary>) -> Self {
        // A character no search typed in one line holds, so that no search
        // finds the end of one field and the start of the next.
        const APART: char = '\u{1f}';
        let mut search = summary.name.to_lowercase();
        for text in summary.subtitle.iter().chain(&summary.websites) {
            search.push(APART);
            search.push_str(&text.to_lowercase());
        }
        let mut hasher = DefaultHasher::new();
        summary.id.hash(&mut hasher);
        Self {
            key: hasher.finish(),
            summary,
            search,
        }
    }

    /// Whether a search, in lower case and trimmed, finds the item.
    pub fn matches(&self, query: &str) -> bool {
        query.is_empty() || self.search.contains(query)
    }
}

/// The items sorted by name, ready for searching: off the main thread.
pub(crate) fn index(mut items: Vec<ItemSummary>) -> Vec<Indexed> {
    items.sort_by_cached_key(|item| item.name.to_lowercase());
    items.into_iter().map(Indexed::new).collect()
}

/// Same as [`index`] for already-shared summaries from the vault cache.
pub(crate) fn index_shared(items: Vec<Arc<ItemSummary>>) -> Vec<Indexed> {
    items.into_iter().map(Indexed::from_shared).collect()
}

/// What the vault holds, for the sidebar and the list's menu to leave out
/// what no item has: found once when the items load.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Holds {
    kinds: Vec<ItemKind>,
    /// Items in no folder.
    unfiled: bool,
    /// Logins with a verification code.
    codes: bool,
    /// Items shared through an organization.
    shared: bool,
}

impl Holds {
    fn of(items: &[Indexed]) -> Self {
        let mut holds = Self::default();
        for item in items {
            let item = &item.summary;
            if !holds.kinds.contains(&item.kind) {
                holds.kinds.push(item.kind);
            }
            holds.unfiled |= item.folder.is_none();
            holds.codes |= item.has_code;
            holds.shared |= item.organization.is_some();
        }
        holds
    }
}

/// The items, as indices into `items`, that the sidebar's choice, the
/// menu's and a search in lower case admit, in `order`.
fn visible(
    items: &[Indexed],
    filter: &Filter,
    only: Only,
    query: &str,
    order: SortOrder,
    recent: &[ItemId],
) -> Vec<usize> {
    let mut visible: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            filter.admits(&item.summary) && only.admits(&item.summary) && item.matches(query)
        })
        .map(|(index, _)| index)
        .collect();
    sort(&mut visible, items, order, recent);
    visible
}

/// Puts the list's items, given as indices into `items`, in `order`.
/// `items` are sorted by name already, and ties keep that order.
fn sort(visible: &mut [usize], items: &[Indexed], order: SortOrder, recent: &[ItemId]) {
    match order {
        SortOrder::Name => {}
        SortOrder::Modified => {
            visible.sort_by_key(|&index| std::cmp::Reverse(items[index].summary.modified));
        }
        SortOrder::RecentlyUsed => {
            let rank: std::collections::HashMap<&ItemId, usize> = recent
                .iter()
                .enumerate()
                .map(|(rank, id)| (id, rank))
                .collect();
            visible.sort_by_key(|&index| {
                rank.get(&items[index].summary.id)
                    .copied()
                    .unwrap_or(usize::MAX)
            });
        }
    }
}

/// The logins whose icons are read ahead for the sidebar's choices other
/// than `shown`: the first rows of each, so that choosing one shows its
/// list complete. Off the main thread: it sorts the list once per choice.
fn icons_ahead(
    items: &[Indexed],
    folders: &[Folder],
    shown: &Filter,
    only: Only,
    query: &str,
    order: SortOrder,
    recent: &[ItemId],
) -> Vec<Arc<ItemSummary>> {
    [
        Filter::All,
        Filter::Favorites,
        Filter::Kind(ItemKind::Login),
        Filter::NoFolder,
    ]
    .into_iter()
    .chain(
        folders
            .iter()
            .map(|folder| Filter::Folder(folder.id.clone())),
    )
    .filter(|filter| filter != shown)
    .flat_map(|filter| {
        visible(items, &filter, only, query, order, recent)
            .into_iter()
            .take(FIRST_ROWS)
    })
    // Only logins have websites' icons.
    .filter(|&index| items[index].summary.kind == ItemKind::Login)
    .take(OTHER_CHOICES_ROWS)
    .map(|index| items[index].summary.clone())
    .collect()
}

/// What the vault's items load with.
struct Loaded {
    items: Vec<Indexed>,
    holds: Holds,
    folders: Vec<Folder>,
    recent: Vec<ItemId>,
    icons: Option<icons::Cached>,
    ahead: Vec<Arc<ItemSummary>>,
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
    /// The items, shared with the work done off the main thread.
    items: Load<Arc<Vec<Indexed>>>,
    /// What the items hold, found when they loaded.
    holds: Holds,
    /// The items that weren't in the list before it last loaded, such as
    /// one just created, whose rows fade in.
    arrived: HashSet<ItemId>,
    /// Forgets the items arrived once their rows have faded in.
    arrived_task: Task<()>,
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
    /// The list's share the preferences gave last, to redraw only when it
    /// changes rather than whenever they are saved.
    saved_share: Option<f32>,
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
    /// The selection's row among `visible`, kept with them.
    selected_index: Option<usize>,
    /// Where the selection's highlight is drawn, in rows, as it slides.
    highlight_at: Option<f32>,
    /// The list changed under the selection, which then doesn't slide.
    highlight_jumps: bool,
    detail: Entity<ItemDetail>,
    sheet: SheetSlot,
    /// The editor the sheet shows, if it shows one.
    editor: Option<WeakEntity<ItemEditor>>,
    /// The window, whose alerts say what went wrong.
    window: AnyWindowHandle,
    list_focus: FocusHandle,
    list_scroll: UniformListScrollHandle,
    list_pointer: ListPointer,
    /// The row the pointer is on, as the list last told.
    pointer_row: Option<usize>,
    /// The row drawn with the pointer's hover.
    hover_drawn: Option<usize>,
    context_menu: Option<ContextMenu>,
    /// Forgets the context menu once it has left.
    context_menu_task: Task<()>,
    /// The item something was copied of from its context menu, whose row
    /// says so for a moment.
    copied: CopiedSlot<ItemId>,
    /// Fetches an item for the context menu; a newer one replaces it.
    menu_task: Task<()>,
    context_menu_focus: FocusHandle,
    /// The generator's popover, kept while it leaves.
    generator: Option<Entity<Generator>>,
    generator_open: bool,
    generator_focus: FocusHandle,
    generator_task: Task<()>,
    last_sync: Option<SystemTime>,
    syncing: bool,
    sync_task: Task<()>,
    locking: bool,
    lock_task: Task<()>,
    /// The item being moved to the trash, until the backend answers.
    deleting: Option<ItemId>,
    delete_task: Task<()>,
    load_task: Task<()>,
    /// Reads ahead the icons of the sidebar's other choices.
    icons_task: Task<()>,
    /// Fetches ahead the row the pointer rests on.
    hover_task: Task<()>,
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
            holds: Holds::default(),
            arrived: HashSet::new(),
            arrived_task: Task::ready(()),
            folders: vec![],
            recent: vec![],
            filter: Filter::All,
            only: Only::default(),
            only_shown: Only::default(),
            sorted_by: SortOrder::Name,
            saved_share: Prefs::global(cx).read(cx).get().list_share,
            sort_menu_open: false,
            sort_menu_focus: cx.focus_handle(),
            search,
            visible: vec![],
            prefetched: 0..0,
            drew: 0..0,
            selected: None,
            selected_index: None,
            highlight_at: None,
            highlight_jumps: false,
            detail,
            sheet: SheetSlot::default(),
            editor: None,
            window: window.window_handle(),
            list_focus: cx.focus_handle(),
            list_scroll: UniformListScrollHandle::new(),
            list_pointer: ListPointer::default(),
            pointer_row: None,
            hover_drawn: None,
            context_menu: None,
            context_menu_task: Task::ready(()),
            copied: CopiedSlot::new(),
            menu_task: Task::ready(()),
            context_menu_focus: cx.focus_handle(),
            generator: None,
            generator_open: false,
            generator_focus: cx.focus_handle(),
            generator_task: Task::ready(()),
            syncing: false,
            sync_task: Task::ready(()),
            locking: false,
            lock_task: Task::ready(()),
            deleting: None,
            delete_task: Task::ready(()),
            load_task: Task::ready(()),
            icons_task: Task::ready(()),
            hover_task: Task::ready(()),
            dragged_share: None,
            save_share_task: Task::ready(()),
            _subscriptions: [
                cx.observe(&Icons::global(cx), |_, _, cx| cx.notify()),
                // The preferences are saved whenever the window moves: only
                // the order and the list's share change what shows here.
                cx.observe(&Prefs::global(cx), |this, prefs, cx| {
                    let prefs = prefs.read(cx).get();
                    let (sort, share) = (prefs.sort, prefs.list_share);
                    if sort != this.sorted_by {
                        this.refilter(cx);
                    }
                    if share != std::mem::replace(&mut this.saved_share, share) {
                        cx.notify();
                    }
                }),
            ],
        };
        if let Some(cached) = vault_cache::get(&this.account.id, cx) {
            let items = index_shared(cached.items);
            this.holds = Holds::of(&items);
            this.items = Load::Ready(Arc::new(items));
            this.folders = cached.folders;
            this.recent = cached.recent;
            this.refilter(cx);
            if cached.stale || cached.last_sync != this.last_sync {
                this.reload(cx);
            } else {
                this.read_icons_ahead(cx);
            }
        } else {
            this.reload(cx);
        }
        window.focus(&this.list_focus, cx);
        if crate::demo::wants("new-item") {
            cx.defer_in(window, |this, window, cx| {
                this.new_item(&NewItem, window, cx)
            });
        }
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
            self.detail.update(cx, |detail, cx| {
                detail.forget_shapes();
                detail.reload(cx);
            });
        }
        cx.notify();
    }

    /// The search, in lower case.
    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_lowercase()
    }

    /// Loads the items and folders again, keeping the selection if it still
    /// exists. The first time, the icons of the first rows come with them.
    /// What a search looks through, what the vault holds and the icons to
    /// read ahead are worked out with them, off the main thread.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let account = self.account_id();
        let first_load = matches!(self.items, Load::Loading);
        let (filter, only, query) = (self.filter.clone(), self.only, self.query(cx));
        let order = Prefs::global(cx).read(cx).get().sort;
        self.load_task = backend::request(
            cx,
            move |backend| {
                let items = index(backend.items(&account)?);
                let folders = backend.folders(&account)?;
                // Without them, the list is still worth showing.
                let recent = backend.recent_items(&account).unwrap_or_default();
                let icons = first_load.then(|| {
                    let rows = visible(&items, &filter, only, &query, order, &recent);
                    let first = rows
                        .into_iter()
                        .take(FIRST_ROWS)
                        .map(|index| &*items[index].summary);
                    icons::read_cached(backend, &account, first)
                });
                let ahead = icons_ahead(&items, &folders, &filter, only, &query, order, &recent);
                Ok(Loaded {
                    holds: Holds::of(&items),
                    items,
                    folders,
                    recent,
                    icons,
                    ahead,
                })
            },
            |this, result, cx| {
                match result {
                    Ok(loaded) => {
                        if let Some(icons) = loaded.icons {
                            Icons::global(cx)
                                .update(cx, |known, cx| known.insert_cached(icons, cx));
                        }
                        this.arrived = match &this.items {
                            Load::Ready(before) => {
                                let before: HashSet<&ItemId> =
                                    before.iter().map(|item| &item.summary.id).collect();
                                loaded
                                    .items
                                    .iter()
                                    .filter(|item| !before.contains(&item.summary.id))
                                    .map(|item| item.summary.id.clone())
                                    .collect()
                            }
                            Load::Loading | Load::Failed(_) => HashSet::new(),
                        };
                        this.forget_arrived_later(cx);
                        let cached_items: Vec<Arc<ItemSummary>> = loaded
                            .items
                            .iter()
                            .map(|item| item.summary.clone())
                            .collect();
                        let account = this.account.id.clone();
                        vault_cache::put(
                            account.clone(),
                            VaultSnapshot {
                                last_sync: this.last_sync,
                                items: cached_items,
                                folders: loaded.folders.clone(),
                                recent: loaded.recent.clone(),
                                stale: false,
                            },
                            cx,
                        );
                        this.items = Load::Ready(Arc::new(loaded.items));
                        this.holds = loaded.holds;
                        this.folders = loaded.folders;
                        this.recent = loaded.recent;
                        Icons::global(cx).update(cx, |icons, cx| {
                            icons.prefetch(&account, loaded.ahead.iter().map(|item| &**item), cx)
                        });
                    }
                    Err(error) => {
                        this.items = Load::Failed(error);
                        this.holds = Holds::default();
                    }
                }
                this.refilter(cx);
            },
        );
    }

    /// The rows that arrived fade in once: after that they are rows like
    /// any other.
    fn forget_arrived_later(&mut self, cx: &mut Context<Self>) {
        if self.arrived.is_empty() {
            self.arrived_task = Task::ready(());
            return;
        }
        self.arrived_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(motion::ARRIVE).await;
            this.update(cx, |this, cx| {
                this.arrived.clear();
                cx.notify();
            })
            .ok();
        });
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
        self.locate_selected();
        match self.selected_index {
            Some(index) => {
                self.list_scroll
                    .scroll_to_item(index, ScrollStrategy::Nearest);
                if let Some(item) = self.item_at(index).cloned() {
                    self.detail
                        .update(cx, |detail, cx| detail.set_summary(item, cx));
                }
            }
            None => {
                let first = self.item_at(0).map(|item| item.id.clone());
                self.select(first, cx);
            }
        }
        cx.notify();
    }

    /// Reads the icons of the first rows of the sidebar's other choices
    /// from rbw's cache, so that choosing one shows its list complete, its
    /// icons with it. Which rows those are is worked out off the main
    /// thread.
    fn read_icons_ahead(&mut self, cx: &mut Context<Self>) {
        let Some(items) = self.items.ready().cloned() else {
            return;
        };
        let (filter, only, query) = (self.filter.clone(), self.only, self.query(cx));
        let order = Prefs::global(cx).read(cx).get().sort;
        let (folders, recent) = (self.folders.clone(), self.recent.clone());
        let account = self.account.id.clone();
        self.icons_task = cx.spawn(async move |this, cx| {
            let rows = cx
                .background_spawn(async move {
                    icons_ahead(&items, &folders, &filter, only, &query, order, &recent)
                })
                .await;
            this.update(cx, |_, cx| {
                Icons::global(cx).update(cx, |icons, cx| {
                    icons.prefetch(&account, rows.iter().map(|item| &**item), cx)
                })
            })
            .ok();
        });
    }

    pub(crate) fn focus_list(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.list_focus, cx);
    }

    fn summary(&self, id: &ItemId) -> Option<&Arc<ItemSummary>> {
        self.items
            .ready()?
            .iter()
            .map(|item| &item.summary)
            .find(|item| item.id == *id)
    }

    /// The item at `position` in the list.
    fn item_at(&self, position: usize) -> Option<&Arc<ItemSummary>> {
        let index = *self.visible.get(position)?;
        Some(&self.items.ready()?.get(index)?.summary)
    }

    fn visible_items(&self) -> impl Iterator<Item = &Arc<ItemSummary>> {
        let items = self
            .items
            .ready()
            .map(|items| items.as_slice())
            .unwrap_or_default();
        self.visible.iter().map(move |&index| &items[index].summary)
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
            self.read_icons_ahead(cx);
        }
    }

    pub(crate) fn set_sort(&mut self, order: SortOrder, cx: &mut Context<Self>) {
        preferences::update(cx, move |preferences| preferences.sort = order);
        self.refilter(cx);
        self.list_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.read_icons_ahead(cx);
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
        self.locate_selected();
        let summary = item.and_then(|id| self.summary(&id).cloned());
        // A selection that moves with the keys or the search is fetched
        // once it rests; meanwhile it shows from its shape, or the list.
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
        let item = self.item_at(index).map(|item| item.id.clone());
        if item.is_some() {
            self.select(item, cx);
            self.list_scroll
                .scroll_to_item(index, ScrollStrategy::Nearest);
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    /// Finds the selection's row again, after the selection or the rows
    /// changed: drawing the list reads it rather than looking for it.
    fn locate_selected(&mut self) {
        self.selected_index = self
            .selected
            .as_ref()
            .and_then(|selected| self.visible_items().position(|item| &item.id == selected));
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        let next = self.selected_index().map_or(0, |index| index + 1);
        self.select_index(next, cx);
        self.prefetch_rows([next + 1], cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.selected_index().and_then(|index| index.checked_sub(1)) {
            self.select_index(index, cx);
            self.prefetch_rows(index.checked_sub(1), cx);
        }
    }

    /// Fetches the items of these rows ahead, so that they show complete
    /// at once when chosen: the next one the arrow keys go to, the one the
    /// pointer is on.
    fn prefetch_rows(&mut self, rows: impl IntoIterator<Item = usize>, cx: &mut Context<Self>) {
        let items: Vec<Arc<ItemSummary>> = rows
            .into_iter()
            .filter_map(|row| self.item_at(row).cloned())
            .collect();
        self.detail
            .update(cx, |detail, cx| detail.prefetch(items, cx));
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
    /// A read-only item, shared without the right to change it, isn't.
    fn edit_item(&mut self, _: &EditItem, window: &mut Window, cx: &mut Context<Self>) {
        let detail = self.detail.read(cx);
        if !detail.selected().is_some_and(|item| item.editable) {
            return;
        }
        if let Some(item) = detail.item().filter(|item| item.editable).cloned() {
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
        if !item.editable {
            return;
        }
        let vault = cx.entity().downgrade();
        let folders = self.folders.clone();
        let account = self.account_id();
        let editor = cx.new(|cx| ItemEditor::edit(account, item, folders, vault, window, cx));
        self.present(editor, window, cx);
    }

    fn present(&mut self, editor: Entity<ItemEditor>, window: &mut Window, cx: &mut Context<Self>) {
        editor.update(cx, |editor, cx| editor.focus(window, cx));
        self.editor = Some(editor.downgrade());
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
        self.editor = None;
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

    /// Syncs the vault, once at a time: another Sync while one runs waits
    /// for nothing.
    fn sync(&mut self, _: &Sync, _: &mut Window, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        self.syncing = true;
        cx.notify();
        let account = self.account_id();
        self.sync_task = backend::request_network(
            cx,
            move |backend| backend.sync(&account),
            |this, result, cx| {
                this.syncing = false;
                match result {
                    Ok(()) => crate::accounts::refresh(cx),
                    Err(error) => this.show_error(&error, cx),
                }
                cx.notify();
            },
        );
    }

    fn lock(&mut self, _: &Lock, _: &mut Window, cx: &mut Context<Self>) {
        if self.locking {
            return;
        }
        self.locking = true;
        let account = self.account_id();
        let locked = account.clone();
        self.lock_task = backend::request(
            cx,
            move |backend| backend.lock(&account),
            |this, result, cx| {
                this.locking = false;
                match result {
                    Ok(()) => {
                        vault_cache::clear_account(&locked, cx);
                        cx.emit(Request::Locked(locked));
                        crate::accounts::refresh(cx);
                    }
                    Err(error) => this.show_error(&error, cx),
                }
            },
        );
    }

    fn delete_selected(&mut self, _: &DeleteItem, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = self.detail.read(cx).selected().filter(|item| item.editable) {
            let (id, name) = (item.id.clone(), item.name.clone());
            self.delete_item(id, name, window, cx);
        }
    }

    /// Asks, then moves the item to the trash and selects the one after it.
    /// An item that asks for the master password again has it asked for
    /// first: it goes only once it is open.
    pub(crate) fn delete_item(
        &mut self,
        id: ItemId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.deleting.is_some() {
            return;
        }
        if self.summary(&id).is_some_and(|item| item.reprompt) {
            self.with_item(id, window, cx, move |this, item, window, cx| {
                this.confirm_delete(item.id, name, window, cx)
            });
        } else {
            self.confirm_delete(id, name, window, cx);
        }
    }

    fn confirm_delete(
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
        if self.deleting.is_some() {
            return;
        }
        self.deleting = Some(id.clone());
        let account = self.account_id();
        let deleted = id.clone();
        self.delete_task = backend::request_network_in(
            window,
            cx,
            move |backend| backend.delete_item(&account, &id),
            move |this, result, window, cx| {
                this.deleting = None;
                match result {
                    Ok(()) => {
                        vault_cache::invalidate(&this.account.id, cx);
                        // Only the sheet that edits the item goes with it.
                        if this.sheet.is_open() && this.edits(&deleted, cx) {
                            this.dismiss_sheet(None, window, cx);
                        }
                        // The next item takes its place, or the one before it
                        // at the end of the list.
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
                    Err(Error::RepromptRequired) => this.ask_password_for(deleted, cx),
                    Err(error) => this.show_error(&error, cx),
                }
            },
        );
    }

    /// Whether the sheet edits this item.
    fn edits(&self, item: &ItemId, cx: &App) -> bool {
        self.editor
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .is_some_and(|editor| editor.read(cx).editing() == Some(item))
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

    /// Says what went wrong at the bottom of the window, with the window's
    /// other news: one toast at a time.
    pub(crate) fn show_error(&mut self, error: &Error, cx: &mut Context<Self>) {
        let message = backend::describe(error);
        let window = self.window;
        // Once this view is done updating: the window is busy until then.
        cx.defer(move |cx| {
            window
                .update(cx, |_, window, cx| {
                    crate::alerts::toast(message, window, cx)
                })
                .ok();
        });
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
        let sheet = self.sheet.render(px(480.), window, cx);
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
        let login = Indexed::new(login);
        for query in ["", "ban", "alex.m", "bank.example"] {
            assert!(login.matches(query), "{query}");
        }
        assert!(!login.matches("github"));
        // Not across the end of one field and the start of the next.
        assert!(!login.matches("bankalex"));
    }

    #[test]
    fn the_vault_says_what_it_holds() {
        let items = index(vec![
            ItemSummary {
                has_code: true,
                folder: Some(FolderId("travel".into())),
                ..item("GitHub", ItemKind::Login)
            },
            ItemSummary {
                folder: Some(FolderId("travel".into())),
                ..item("Visa", ItemKind::Card)
            },
        ]);
        let holds = Holds::of(&items);
        assert_eq!(holds.kinds, [ItemKind::Login, ItemKind::Card]);
        assert!(holds.codes && !holds.shared && !holds.unfiled);
        // Sorted by name, each with a key of its own.
        assert_eq!(items[0].summary.name, "GitHub");
        assert_ne!(items[0].key, items[1].key);
    }

    #[test]
    fn icons_are_read_ahead_for_logins_of_the_other_choices() {
        let items = index(vec![
            ItemSummary {
                favorite: true,
                ..item("Bank", ItemKind::Login)
            },
            ItemSummary {
                favorite: true,
                ..item("Visa", ItemKind::Card)
            },
        ]);
        let ahead = |shown: &Filter| -> Vec<String> {
            icons_ahead(
                &items,
                &[],
                shown,
                Only::default(),
                "",
                SortOrder::Name,
                &[],
            )
            .iter()
            .map(|item| item.name.clone())
            .collect()
        };
        // Every choice but the one shown, and only their logins.
        assert!(ahead(&Filter::All).iter().all(|name| name == "Bank"));
        assert!(!ahead(&Filter::All).is_empty());
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
        let items = index(vec![
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
        ]);
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
