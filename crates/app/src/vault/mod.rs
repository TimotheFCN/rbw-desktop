//! The vault window: sidebar, item list and the selected item.

mod detail;
mod editor;
mod list;
mod sidebar;

use std::time::{Duration, SystemTime};

use gpui::base::input::{InputEvent, InputState};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement as _, Render, ScrollStrategy, Styled as _, Task,
    UniformListScrollHandle, Window, actions, div, px, size,
};
use rbw_desktop_core::{
    Account, AccountId, Error, Folder, FolderId, ItemId, ItemKind, ItemSummary,
};
use rbw_desktop_ui::{SheetSlot, Theme, ToastSlot, chrome};

use crate::backend::{self, Load};
use crate::secondary;
use detail::ItemDetail;
use editor::ItemEditor;

actions!(
    vault,
    [
        /// Opens the New Item sheet.
        NewItem,
        /// Opens the selected item in the edit sheet.
        EditItem,
        /// Moves the keyboard to the search field.
        Find,
        SelectNext,
        SelectPrevious,
        /// Syncs the vault with the server now.
        Sync,
        Lock,
    ]
);

const CONTEXT: &str = "Vault";
/// How often the account is asked whether it locked or synced.
const POLL: Duration = Duration::from_secs(2);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(secondary!("n"), NewItem, Some(CONTEXT)),
        KeyBinding::new(secondary!("e"), EditItem, Some(CONTEXT)),
        KeyBinding::new(secondary!("f"), Find, Some(CONTEXT)),
        KeyBinding::new(secondary!("r"), Sync, Some(CONTEXT)),
        KeyBinding::new(secondary!("l"), Lock, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some("VaultList")),
        KeyBinding::new("up", SelectPrevious, Some("VaultList")),
    ]);
    editor::init(cx);
}

pub fn open(account: Account, cx: &mut App) {
    let options = chrome::window_options(
        "Vault",
        size(px(880.), px(580.)),
        size(px(680.), px(440.)),
        cx,
    );
    let window = gpui::open_window(options, cx, |window, cx| {
        cx.new(|cx| VaultView::new(account, window, cx))
    });
    if let Err(error) = window {
        log::error!("opening the vault window: {error:#}");
    }
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
    last_sync: Option<SystemTime>,
    load_task: Task<()>,
    _poll: Task<()>,
}

impl VaultView {
    fn new(account: Account, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        cx.observe_window_appearance(window, |_, window, cx| {
            Theme::follow_window_appearance(window, cx);
        })
        .detach();
        let vault = cx.entity().downgrade();
        let detail = cx.new(|_| ItemDetail::new(account.id.clone(), vault));
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                    break;
                }
            }
        });
        let mut this = Self {
            last_sync: account.last_sync,
            account,
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
            load_task: Task::ready(()),
            _poll: poll,
        };
        this.reload(cx);
        window.focus(&this.list_focus, cx);
        this
    }

    fn account_id(&self) -> AccountId {
        self.account.id.clone()
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

    fn poll(&mut self, cx: &mut Context<Self>) {
        let account = self.account_id();
        // The poll replaces nothing and is cheap, so its task can run
        // detached: the view going away ends it at the next update.
        backend::request(
            cx,
            move |backend| backend.account(&account),
            |this, result, cx| {
                let Ok(account) = result else { return };
                let synced = account.last_sync != this.last_sync;
                let state_changed = account.state != this.account.state;
                this.last_sync = account.last_sync;
                this.account = account;
                if synced || state_changed {
                    this.reload(cx);
                    this.detail.update(cx, |detail, cx| detail.reload(cx));
                }
            },
        )
        .detach();
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
            Some(index) => self
                .list_scroll
                .scroll_to_item(index, ScrollStrategy::Nearest),
            None => {
                let first = self.visible_items().next().map(|item| item.id.clone());
                self.select(first, cx);
            }
        }
        cx.notify();
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
        if self.selected == item {
            return;
        }
        self.selected = item.clone();
        self.detail.update(cx, |detail, cx| detail.show(item, cx));
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
                Ok(()) => this.poll(cx),
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
                Ok(()) => this.poll(cx),
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
        let content = div()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::new_item))
            .on_action(cx.listener(Self::edit_item))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::sync))
            .on_action(cx.listener(Self::lock))
            .relative()
            .size_full()
            .flex()
            .child(sidebar)
            .child(list)
            .child(self.detail.clone())
            .children(sheet)
            .children(toast);
        chrome::window_frame(content, window, cx)
    }
}
