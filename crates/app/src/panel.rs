//! The menu bar item's panel: the items used last, with a search through
//! the whole vault, and the generator, with Open Vault and Lock under them.
//! On macOS it hangs from the menu bar; elsewhere it is a small window near
//! the pointer, or centred. It shows the account the main window shows,
//! and closes on Escape or when it loses the keyboard.

use std::time::{Duration, Instant};

use gpui::base::input::{InputEvent, InputState};
use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, Context, DisplayId, Entity,
    FocusHandle, Focusable, FontWeight, Global, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, Pixels, Point, Render, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, UniformListScrollHandle,
    Window, WindowOptions, actions, div, point, prelude::FluentBuilder as _, px, size,
    uniform_list,
};
use rbw_desktop_core::{
    APP_ID, APP_NAME, Account, AccountId, Error, Item, ItemContent, ItemId, ItemKind, ItemSummary,
    LockState, Secret,
};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Material, ToastSlot, backdrop, button, chrome, icon,
    list_row, motion, scroll_area, scrollbar, search_field, section_label, segmented_control,
};

use crate::CloseWindow;
use crate::accounts::{self, Accounts};
use crate::backend::{self, Load};
use crate::clipboard;
use crate::generator::Generator;
use crate::icons::{self, Icons};
use crate::recent;
use crate::secondary;
use crate::vault;
use crate::window;

actions!(
    quick_panel,
    [
        ClosePanel,
        SelectNext,
        SelectPrevious,
        /// Copies the selected login's username.
        CopyUsername,
        /// Copies the selected login's verification code, or a card's
        /// security code.
        CopyCode,
        /// Shows the selected item in the main window.
        OpenInVault,
    ]
);

const CONTEXT: &str = "QuickPanel";
const PASSWORDS: &str = "PanelPasswords";
/// The panel's size, without the frame Linux draws around it.
const SIZE: gpui::Size<Pixels> = size(px(306.), px(488.));
/// How many of the items used last it lists.
const RECENT: usize = 10;
/// A click on the menu bar item that closed the panel, by taking the
/// keyboard from it, doesn't open it again straight away.
const REOPEN_AFTER: Duration = Duration::from_millis(400);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", ClosePanel, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(PASSWORDS)),
        KeyBinding::new("up", SelectPrevious, Some(PASSWORDS)),
        // The search field keeps the arrow keys otherwise.
        KeyBinding::new("down", SelectNext, Some("PanelPasswords > Input")),
        KeyBinding::new("up", SelectPrevious, Some("PanelPasswords > Input")),
        KeyBinding::new(secondary!("shift-c"), CopyUsername, Some(PASSWORDS)),
        KeyBinding::new(secondary!("alt-c"), CopyCode, Some(PASSWORDS)),
        KeyBinding::new(secondary!("o"), OpenInVault, Some(PASSWORDS)),
    ]);
    cx.set_global(PanelWindow::default());
}

/// Where the panel opens from.
#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    /// The menu bar item, in the coordinates of its display: the panel
    /// hangs under it.
    #[cfg(target_os = "macos")]
    Item(Bounds<Pixels>, DisplayId),
    /// Where the pointer was, on the whole screen: the panel opens next to
    /// it, above it in the bottom half.
    #[cfg(not(target_os = "macos"))]
    Pointer(Point<Pixels>),
    Centered,
}

#[derive(Default)]
struct PanelWindow {
    window: Option<AnyWindowHandle>,
    closed_at: Option<Instant>,
}

impl Global for PanelWindow {}

/// Opens the panel, or closes it if it is open.
pub fn toggle(anchor: Anchor, cx: &mut App) {
    let state = cx.global::<PanelWindow>();
    if let Some(handle) = state.window {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
        return;
    }
    if state
        .closed_at
        .is_some_and(|closed| closed.elapsed() < REOPEN_AFTER)
    {
        return;
    }
    open(anchor, cx);
}

pub fn open(anchor: Anchor, cx: &mut App) {
    if !backend::ready(cx) || cx.global::<PanelWindow>().window.is_some() {
        return;
    }
    let options = WindowOptions {
        app_id: Some(APP_ID.into()),
        ..chrome::panel_options(APP_NAME, SIZE, origin(anchor, cx), cx)
    };
    match gpui::open_window(options, cx, |window, cx| {
        cx.new(|cx| QuickPanel::new(window, cx))
    }) {
        Ok((handle, _)) => cx.global_mut::<PanelWindow>().window = Some(handle),
        Err(error) => log::error!("opening the panel: {error:#}"),
    }
}

/// Where the panel's top-left corner goes, kept on the display.
fn origin(anchor: Anchor, cx: &App) -> Option<(Point<Pixels>, Option<DisplayId>)> {
    let margin = px(8.);
    let keep = |at, display| keep_on(at, display, margin);
    match anchor {
        #[cfg(target_os = "macos")]
        Anchor::Item(item, display) => {
            let screen = cx.find_display(display)?.bounds();
            let at = point(item.center().x - SIZE.width / 2., item.bottom() + px(5.));
            // the item's coordinates are the display's own
            let local = Bounds::new(Point::default(), screen.size);
            Some((keep(at, local), Some(display)))
        }
        #[cfg(not(target_os = "macos"))]
        Anchor::Pointer(at) => {
            let display = cx
                .displays()
                .into_iter()
                .find(|display| display.bounds().contains(&at))?;
            let screen = display.bounds();
            let below = at.y < screen.center().y;
            let y = if below {
                at.y + margin
            } else {
                at.y - SIZE.height - margin
            };
            Some((keep(point(at.x - SIZE.width / 2., y), screen), None))
        }
        Anchor::Centered => None,
    }
}

/// The panel's corner at `at`, moved as little as needed for the panel to
/// stay `margin` inside the display.
fn keep_on(at: Point<Pixels>, display: Bounds<Pixels>, margin: Pixels) -> Point<Pixels> {
    point(
        at.x.clamp(
            display.left() + margin,
            display.right() - SIZE.width - margin,
        ),
        at.y.clamp(
            display.top() + margin,
            display.bottom() - SIZE.height - margin,
        ),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Passwords,
    Generator,
}

pub struct QuickPanel {
    tab: Tab,
    search: Entity<InputState>,
    /// The account the main window shows, which the panel follows.
    account: Option<Account>,
    several_accounts: bool,
    /// The account's items, for the search and to name the recent ones.
    items: Load<Vec<ItemSummary>>,
    loaded: Option<(AccountId, Option<std::time::SystemTime>)>,
    recent: Vec<ItemId>,
    /// Indices into the items of the rows shown, in order.
    shown: Vec<usize>,
    selected: usize,
    copied: Option<ItemId>,
    copied_timer: Task<()>,
    unlocking: bool,
    /// Made when its tab is first shown.
    generator: Option<Entity<Generator>>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    toast: ToastSlot,
    load_task: Task<()>,
    /// Whether the panel has had the keyboard, after which losing it
    /// closes the panel.
    was_active: bool,
    _subscriptions: Vec<Subscription>,
}

impl QuickPanel {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // A pop-up without traffic lights; Linux draws no title bar here.
        #[cfg(target_os = "macos")]
        window.set_native_titlebar_visibility(false);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let accounts = Accounts::global(cx);
        let subscriptions = vec![
            cx.subscribe_in(
                &search,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => this.refilter(cx),
                    InputEvent::PressEnter { .. } => this.copy_main(cx),
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ),
            cx.observe_in(&accounts, window, |this, _, window, cx| {
                this.follow_accounts(window, cx)
            }),
            cx.observe(&Icons::global(cx), |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, window, cx| {
                rbw_desktop_ui::Theme::follow_window_appearance(window, cx);
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.was_active = true;
                } else if this.was_active {
                    this.close(window, cx);
                }
            }),
        ];
        cx.on_release(|_, cx| {
            let state = cx.global_mut::<PanelWindow>();
            state.window = None;
            state.closed_at = Some(Instant::now());
        })
        .detach();
        let mut this = Self {
            tab: Tab::Passwords,
            search,
            account: None,
            several_accounts: false,
            items: Load::Loading,
            loaded: None,
            recent: vec![],
            shown: vec![],
            selected: 0,
            copied: None,
            copied_timer: Task::ready(()),
            unlocking: false,
            generator: None,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            toast: ToastSlot::default(),
            load_task: Task::ready(()),
            was_active: false,
            _subscriptions: subscriptions,
        };
        window.focus(&this.focus, cx);
        accounts::refresh(cx);
        this.follow_accounts(window, cx);
        this
    }

    fn close(&mut self, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn close_action(&mut self, _: &ClosePanel, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    /// Takes the current account's latest state, loading its items once it
    /// is unlocked and again after a sync.
    fn follow_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let accounts = Accounts::global(cx);
        let accounts = accounts.read(cx);
        let account = accounts.current().cloned();
        self.several_accounts = accounts.list().len() > 1;
        let unlocked = account
            .as_ref()
            .filter(|account| account.state == LockState::Unlocked)
            .map(|account| (account.id.clone(), account.last_sync));
        let was_unlocked = self.loaded.is_some();
        if unlocked != self.loaded {
            self.loaded = unlocked.clone();
            match unlocked {
                Some((id, _)) => self.load(id, cx),
                None => {
                    self.items = Load::Loading;
                    self.shown.clear();
                }
            }
        }
        if self.loaded.is_some() && !was_unlocked && self.tab == Tab::Passwords {
            self.search
                .update(cx, |search, cx| search.focus(window, cx));
        }
        self.account = account;
        cx.notify();
    }

    fn load(&mut self, account: AccountId, cx: &mut Context<Self>) {
        self.load_task = backend::request(
            cx,
            move |backend| Ok((backend.items(&account)?, backend.recent_items(&account)?)),
            |this, result, cx| {
                match result {
                    Ok((mut items, recent)) => {
                        items.sort_by_cached_key(|item| item.name.to_lowercase());
                        this.items = Load::Ready(items);
                        this.recent = recent;
                    }
                    Err(error) => this.items = Load::Failed(error),
                }
                this.refilter(cx);
            },
        );
    }

    fn account_id(&self) -> Option<AccountId> {
        self.account.as_ref().map(|account| account.id.clone())
    }

    /// The items used last, or those matching the search.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().to_lowercase();
        let query = query.trim();
        let items = self.items.ready().map(Vec::as_slice).unwrap_or_default();
        self.shown = if query.is_empty() {
            self.recent
                .iter()
                .filter_map(|id| items.iter().position(|item| item.id == *id))
                .take(RECENT)
                .collect()
        } else {
            items
                .iter()
                .enumerate()
                .filter(|(_, item)| vault::matches(item, query))
                .map(|(index, _)| index)
                .collect()
        };
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn item(&self, position: usize) -> Option<&ItemSummary> {
        let index = *self.shown.get(position)?;
        self.items.ready()?.get(index)
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected + 1 < self.shown.len() {
            self.select(self.selected + 1, cx);
        }
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.selected.checked_sub(1) {
            self.select(previous, cx);
        }
    }

    fn select(&mut self, position: usize, cx: &mut Context<Self>) {
        if self.selected != position {
            self.selected = position;
            self.scroll
                .scroll_to_item(position, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    fn copy_username(&mut self, _: &CopyUsername, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected(
            |kind| (kind == ItemKind::Login).then_some(Part::Username),
            cx,
        );
    }

    fn copy_code(&mut self, _: &CopyCode, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected(
            |kind| match kind {
                ItemKind::Login => Some(Part::Code),
                ItemKind::Card => Some(Part::SecurityCode),
                _ => None,
            },
            cx,
        );
    }

    /// Return: copies what the selected item is for.
    fn copy_main(&mut self, cx: &mut Context<Self>) {
        self.copy_selected(|kind| Some(Part::main(kind)), cx);
    }

    fn open_in_vault(&mut self, _: &OpenInVault, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = self.item(self.selected) {
            self.open_item(item.id.clone(), cx);
        }
    }

    /// Copies what `part` picks of the selected item's kind, if anything.
    fn copy_selected(&mut self, part: impl Fn(ItemKind) -> Option<Part>, cx: &mut Context<Self>) {
        if let Some(item) = self.item(self.selected).cloned()
            && let Some(part) = part(item.kind)
        {
            self.copy(item, part, cx);
        }
    }

    /// Copies something of an item. An item that asks for the master
    /// password again opens in the main window instead.
    fn copy(&mut self, item: ItemSummary, part: Part, cx: &mut Context<Self>) {
        let Some(account) = self.account_id() else {
            return;
        };
        let (requested, id) = (item.id.clone(), item.id);
        backend::request(
            cx,
            move |backend| match part {
                Part::Code => Ok(backend.verification_code(&account, &requested)?.code),
                _ => Ok(part.value(&backend.item(&account, &requested)?)),
            },
            move |this, result, cx| match result {
                Ok(value) if value.expose().is_empty() => this.say(part.missing(), cx),
                Ok(value) => this.copied(id, &value, part.secret(), cx),
                Err(Error::RepromptRequired) => this.open_item(id, cx),
                Err(error) => this.say(backend::describe(&error), cx),
            },
        )
        .detach();
    }

    fn copied(&mut self, item: ItemId, value: &Secret, secret: bool, cx: &mut Context<Self>) {
        let account = self.account_id();
        if secret {
            clipboard::copy_secret(value, account.as_ref(), cx);
        } else {
            clipboard::copy_text(value.expose(), cx);
        }
        if let Some(account) = &account {
            recent::used(account, &item, cx);
        }
        self.copied = Some(item);
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

    fn say(&mut self, message: &'static str, cx: &mut Context<Self>) {
        self.toast
            .show(message, |this: &mut Self| &mut this.toast, cx);
    }

    /// Shows the item in the main window, which asks for the master
    /// password if the item wants it.
    fn open_item(&mut self, item: ItemId, cx: &mut Context<Self>) {
        let Some(account) = self.account_id() else {
            return;
        };
        cx.defer(move |cx| {
            window::with_main_window(cx, move |main, window, cx| {
                main.open_item(account, item, window, cx)
            });
            cx.activate(true);
        });
        self.close_later(cx);
    }

    fn open_vault(&mut self, cx: &mut Context<Self>) {
        let account = self.account_id();
        cx.defer(move |cx| {
            window::with_main_window(cx, move |main, window, cx| {
                if let Some(account) = account {
                    main.show_account(account, window, cx);
                }
            });
            cx.activate(true);
        });
        self.close_later(cx);
    }

    /// Closes the panel once the current update is over.
    fn close_later(&self, cx: &mut Context<Self>) {
        let panel = cx.global::<PanelWindow>().window;
        cx.defer(move |cx| {
            if let Some(panel) = panel {
                panel.update(cx, |_, window, _| window.remove_window()).ok();
            }
        });
    }

    fn lock(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.account_id() else {
            return;
        };
        backend::request(
            cx,
            move |backend| backend.lock(&account),
            |this, result, cx| match result {
                Ok(()) => accounts::refresh(cx),
                Err(error) => this.say(backend::describe(&error), cx),
            },
        )
        .detach();
    }

    fn unlock_with_touch_id(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.account_id() else {
            return;
        };
        self.unlocking = true;
        cx.notify();
        backend::request(
            cx,
            move |backend| backend.unlock_with_touch_id(&account),
            |this, result, cx| {
                this.unlocking = false;
                match result {
                    Ok(()) => accounts::refresh(cx),
                    Err(error) => this.say(backend::describe(&error), cx),
                }
                cx.notify();
            },
        )
        .detach();
    }

    fn show_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        match tab {
            Tab::Passwords if self.loaded.is_some() => self
                .search
                .update(cx, |search, cx| search.focus(window, cx)),
            Tab::Passwords => window.focus(&self.focus, cx),
            Tab::Generator => {
                let generator = self
                    .generator
                    .get_or_insert_with(|| cx.new(|cx| Generator::new(true, cx)));
                window.focus(&generator.focus_handle(cx), cx);
            }
        }
        cx.notify();
    }

    fn render_passwords(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let account = self.account.clone();
        let known = !matches!(Accounts::global(cx).read(cx).load(), Load::Loading);
        let body = match account.as_ref().map(|account| account.state) {
            _ if !known => div().into_any_element(),
            Some(LockState::Unlocked) => self.render_items(cx),
            Some(LockState::Locked) => self.render_locked(cx),
            Some(LockState::SignedOut) | None => self.render_signed_out(cx),
        };
        div()
            .key_context(PASSWORDS)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::copy_username))
            .on_action(cx.listener(Self::copy_code))
            .on_action(cx.listener(Self::open_in_vault))
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(body)
            .into_any_element()
    }

    fn render_items(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let dim = theme.colors.text_secondary;
        let searching = !self.search.read(cx).value().trim().is_empty();
        let heading = if searching {
            "Results"
        } else {
            "Recently used"
        };
        let account_name = self
            .account
            .as_ref()
            .filter(|_| self.several_accounts)
            .map(accounts::name);
        let empty = match (&self.items, searching) {
            (Load::Failed(error), _) => Some(backend::describe(error)),
            (Load::Ready(_), true) if self.shown.is_empty() => Some("No Results"),
            (Load::Ready(_), false) if self.shown.is_empty() => {
                Some("The items you copy or open show here.")
            }
            _ => None,
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(div().flex().child(search_field(&self.search)))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(8.))
                    .child(div().flex_1().child(section_label(heading, cx)))
                    .children(account_name.map(|name| {
                        div()
                            .max_w(px(160.))
                            .truncate()
                            .pb(px(4.))
                            .px(px(7.))
                            .text_size(px(11.5))
                            .text_color(dim)
                            .child(name)
                    })),
            )
            .child(match empty {
                Some(message) => div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(16.))
                    .text_center()
                    .text_color(dim)
                    .child(message)
                    .into_any_element(),
                // Only the rows on screen are built, however many match.
                None => div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        uniform_list(
                            "panel-rows",
                            self.shown.len(),
                            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .filter_map(|position| this.render_row(position, cx))
                                    .collect()
                            }),
                        )
                        .track_scroll(&self.scroll)
                        .flex_1(),
                    )
                    .child(scrollbar("panel-rows-bar", &self.scroll, cx))
                    .into_any_element(),
            })
            .into_any_element()
    }

    /// A row: a click copies what the item is for, such as a login's
    /// password; the selected row also has a button for each thing it can
    /// copy, and one that opens the item in the vault.
    fn render_row(&self, position: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let item = self.item(position)?.clone();
        let account = self.account.as_ref()?.id.clone();
        let selected = position == self.selected;
        let icon = icons::icon(&account, &item, cx);
        let copied = self.copied.as_ref() == Some(&item.id);
        let actions = selected.then(|| self.render_row_actions(&item, cx));
        let clicked = item.clone();
        let main = Part::main(item.kind);
        Some(
            div()
                .id(("panel-row", position))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.select(position, cx);
                    }
                }))
                .child(
                    list_row(
                        SharedString::from(format!("panel-item-{}", item.id.0)),
                        item.name.clone(),
                        item.subtitle.clone().map(Into::into),
                        selected,
                    )
                    .compact()
                    // The search field keeps the keyboard; the arrow keys
                    // move through the rows.
                    .in_focusable_list(false)
                    .icon(icon)
                    .accessibility_label(format!("{}, {}", item.name, main.label()))
                    .copied(copied)
                    .trailing(div().flex().items_center().children(actions))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.copy(clicked.clone(), main, cx)),
                    ),
                )
                .into_any_element(),
        )
    }

    /// The selected row's buttons: one for each thing of the item there is
    /// to copy, then one that opens it in the vault. The list says whether
    /// a login has a username or a code; it doesn't say whether a password,
    /// a card's number or a note is empty, so those buttons always show and
    /// say so when it is.
    fn render_row_actions(&self, item: &ItemSummary, cx: &mut Context<Self>) -> AnyElement {
        let parts = Part::of(item);
        let id = item.id.clone();
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .children(parts.into_iter().map(|part| {
                let item = item.clone();
                button(SharedString::from(format!("panel-{}", part.label())))
                    .icon(part.icon())
                    .style(ButtonStyle::Flat)
                    .size(px(26.))
                    .accessibility_label(part.label())
                    .shortcut(part.keys(item.kind))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.copy(item.clone(), part, cx);
                    }))
            }))
            .child(
                button("panel-open")
                    .icon(IconName::Window)
                    .style(ButtonStyle::Flat)
                    .size(px(26.))
                    .accessibility_label("Open in Vault")
                    .shortcut(secondary!("o"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_item(id.clone(), cx);
                    })),
            )
            .into_any_element()
    }

    /// The account is locked: unlock it in the main window, or with Touch
    /// ID here.
    fn render_locked(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let account = self.account.clone();
        let touch_id = account
            .as_ref()
            .is_some_and(|account| account.touch_id_ready);
        let name = account.as_ref().map(accounts::name);
        let unlock = button("panel-unlock")
            .label("Unlock…")
            .style(if touch_id {
                ButtonStyle::Normal
            } else {
                ButtonStyle::Primary
            })
            .on_click(cx.listener(|this, _, _, cx| this.open_vault(cx)));
        let touch_id = touch_id.then(|| {
            button("panel-touch-id")
                .label("Unlock with Touch ID")
                .icon(IconName::TouchId)
                .style(ButtonStyle::Primary)
                .busy(self.unlocking)
                .on_click(cx.listener(|this, _, _, cx| this.unlock_with_touch_id(cx)))
        });
        message(
            IconName::Lock,
            "Your vault is locked",
            name.map(Into::into),
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .children(touch_id)
                .child(unlock),
            cx,
        )
    }

    fn render_signed_out(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let name = self.account.as_ref().map(accounts::name);
        message(
            IconName::Identity,
            "You aren’t signed in",
            name.map(Into::into),
            button("panel-sign-in")
                .label("Sign In…")
                .style(ButtonStyle::Primary)
                .on_click(cx.listener(|this, _, _, cx| this.open_vault(cx))),
            cx,
        )
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let unlocked = self.loaded.is_some();
        div()
            .mt(px(12.))
            .flex()
            .gap(px(8.))
            .child(
                div().flex_1().child(
                    button("open-vault")
                        .label("Open Vault")
                        .wide()
                        .on_click(cx.listener(|this, _, _, cx| this.open_vault(cx))),
                ),
            )
            .child(
                button("panel-lock")
                    .icon(IconName::Lock)
                    .accessibility_label("Lock")
                    .disabled(!unlocked)
                    .on_click(cx.listener(|this, _, _, cx| this.lock(cx))),
            )
            .into_any_element()
    }
}

/// Something of an item that the panel copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Username,
    Password,
    Code,
    Number,
    SecurityCode,
    Note,
    Name,
    PublicKey,
}

impl Part {
    /// What a click or Return copies of an item of `kind`.
    fn main(kind: ItemKind) -> Self {
        match kind {
            ItemKind::Login => Part::Password,
            ItemKind::Card => Part::Number,
            ItemKind::SecureNote => Part::Note,
            ItemKind::Identity => Part::Name,
            ItemKind::SshKey => Part::PublicKey,
        }
    }

    /// The things to copy of an item, leaving out those the list knows it
    /// hasn't got.
    fn of(item: &ItemSummary) -> Vec<Self> {
        let named = item.subtitle.is_some();
        match item.kind {
            ItemKind::Login => [
                (Part::Username, named),
                (Part::Password, true),
                (Part::Code, item.has_code),
            ]
            .into_iter()
            .filter_map(|(part, has)| has.then_some(part))
            .collect(),
            ItemKind::Card => vec![Part::Number, Part::SecurityCode],
            ItemKind::SecureNote => vec![Part::Note],
            ItemKind::Identity | ItemKind::SshKey if !named => vec![],
            ItemKind::Identity => vec![Part::Name],
            ItemKind::SshKey => vec![Part::PublicKey],
        }
    }

    fn label(self) -> &'static str {
        match self {
            Part::Username => "Copy Username",
            Part::Password => "Copy Password",
            Part::Code => "Copy Verification Code",
            Part::Number => "Copy Number",
            Part::SecurityCode => "Copy Security Code",
            Part::Note => "Copy Note",
            Part::Name => "Copy Name",
            Part::PublicKey => "Copy Public Key",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Part::Username | Part::Name => IconName::Identity,
            Part::Password | Part::PublicKey => IconName::Key,
            Part::Code => IconName::Clock,
            Part::Number => IconName::Card,
            Part::SecurityCode => IconName::Shield,
            Part::Note => IconName::Note,
        }
    }

    /// The keys that copy it from the search field.
    fn keys(self, kind: ItemKind) -> &'static str {
        match self {
            _ if self == Part::main(kind) => "enter",
            Part::Username => secondary!("shift-c"),
            _ => secondary!("alt-c"),
        }
    }

    /// Whether it is a secret, which the clipboard forgets after a while.
    fn secret(self) -> bool {
        !matches!(self, Part::Username | Part::Name | Part::PublicKey)
    }

    /// The value in an item; empty when the item has none.
    fn value(self, item: &Item) -> Secret {
        let text = |text: &str| Secret::new(text.to_string());
        match (self, &item.data.content) {
            (Part::Username, ItemContent::Login(login)) => text(&login.username),
            (Part::Password, ItemContent::Login(login)) => login.password.clone(),
            (Part::Number, ItemContent::Card(card)) => card.number.clone(),
            (Part::SecurityCode, ItemContent::Card(card)) => card.code.clone(),
            (Part::Name, ItemContent::Identity(identity)) => text(&identity.full_name()),
            (Part::PublicKey, ItemContent::SshKey(key)) => text(&key.public_key),
            (Part::Note, _) => item.data.notes.clone(),
            _ => Secret::default(),
        }
    }

    /// What to say when the item has none.
    fn missing(self) -> &'static str {
        match self {
            Part::Username => "This login has no username.",
            Part::Password => "This login has no password.",
            Part::Code => "This login has no verification code.",
            Part::Number => "This card has no number.",
            Part::SecurityCode => "This card has no security code.",
            Part::Note => "This note is empty.",
            Part::Name => "This identity has no name.",
            Part::PublicKey => "This SSH key has no public key.",
        }
    }
}

/// A state in place of the items, such as a locked vault: an icon, what is
/// the matter, whose account, and what to do.
fn message(
    name: IconName,
    title: &'static str,
    detail: Option<SharedString>,
    actions: impl IntoElement,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let dim = theme.colors.text_secondary;
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .px(px(16.))
        .child(icon(name).size(px(28.)).text_color(dim))
        .child(
            div()
                .mt(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .children(detail.map(|detail| {
            div()
                .mt(px(2.))
                .max_w_full()
                .truncate()
                .text_size(px(12.))
                .text_color(dim)
                .child(detail)
        }))
        .child(div().mt(px(14.)).child(actions))
        .into_any_element()
}

impl Focusable for QuickPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for QuickPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = match self.tab {
            Tab::Passwords => self.render_passwords(cx),
            // Room at the sides for the buttons' shadows, which the
            // scrolling would clip.
            Tab::Generator => scroll_area("panel-generator")
                .flex_1()
                .min_h_0()
                .mx(px(-8.))
                .px(px(8.))
                .children(self.generator.clone())
                .into_any_element(),
        };
        let this = cx.entity().downgrade();
        let tabs = segmented_control(
            "panel-tab",
            ["Passwords", "Generator"],
            match self.tab {
                Tab::Passwords => 0,
                Tab::Generator => 1,
            },
        )
        .on_select(move |index, window, cx| {
            let tab = if index == 0 {
                Tab::Passwords
            } else {
                Tab::Generator
            };
            this.update(cx, |this, cx| this.show_tab(tab, window, cx))
                .ok();
        });
        // Locked or signed out, the Passwords tab's own button opens the
        // vault: the footer would say it twice.
        let footer =
            (self.tab == Tab::Generator || self.loaded.is_some()).then(|| self.render_footer(cx));
        let toast = self.toast.render(window, cx);
        let content = div()
            .id("quick-panel")
            .key_context(CONTEXT)
            // While the search field is there it holds the keyboard; a
            // click on an empty part of the panel mustn't take it.
            .when(self.loaded.is_none() || self.tab == Tab::Generator, |d| {
                d.track_focus(&self.focus)
            })
            .on_action(cx.listener(Self::close_action))
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .p(px(12.))
            .rounded(chrome::corner_radius(window, cx))
            .child(tabs)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .mt(px(10.))
                    .flex()
                    .flex_col()
                    .child(motion::content_enter(
                        ("panel-tab-content", self.tab as usize),
                        div().flex_1().min_h_0().flex().flex_col().child(tab),
                    )),
            )
            .children(footer)
            .children(toast);
        // On macOS the system's popover material, as under the menu bar's
        // own panels; elsewhere the colour of popovers.
        let content = match backdrop("panel-material", Material::Popover, window, cx) {
            Some(material) => content.child(material),
            None => content.bg(cx.theme().colors.frost),
        };
        chrome::window_frame(content, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_offer_what_their_items_have() {
        let item = |kind, subtitle: Option<&str>, has_code| ItemSummary {
            id: ItemId("a".into()),
            kind,
            name: "A".into(),
            subtitle: subtitle.map(Into::into),
            websites: vec![],
            folder: None,
            organization: None,
            favorite: false,
            has_code,
            reprompt: false,
            editable: true,
            modified: None,
        };
        let login = item(ItemKind::Login, Some("alex"), true);
        assert_eq!(
            Part::of(&login),
            [Part::Username, Part::Password, Part::Code]
        );
        assert_eq!(
            Part::of(&item(ItemKind::Login, None, false)),
            [Part::Password]
        );
        assert_eq!(
            Part::of(&item(ItemKind::Card, Some("Visa"), false)),
            [Part::Number, Part::SecurityCode]
        );
        assert_eq!(Part::of(&item(ItemKind::Identity, None, false)), []);
        // Return copies the main thing; the others have keys of their own.
        assert_eq!(Part::Password.keys(ItemKind::Login), "enter");
        assert_eq!(Part::Number.keys(ItemKind::Card), "enter");
        assert_ne!(Part::SecurityCode.keys(ItemKind::Card), "enter");
    }

    #[test]
    fn the_panel_stays_on_its_display() {
        let display = Bounds::new(point(px(1920.), px(0.)), size(px(1280.), px(800.)));
        let margin = px(8.);
        // Inside, it stays.
        let at = point(px(2400.), px(100.));
        assert_eq!(keep_on(at, display, margin), at);
        // Past a corner, it comes back inside.
        let corner = keep_on(point(px(3190.), px(790.)), display, margin);
        assert_eq!(corner.x, px(3200.) - SIZE.width - margin);
        assert_eq!(corner.y, px(800.) - SIZE.height - margin);
        let start = keep_on(point(px(1900.), px(-20.)), display, margin);
        assert_eq!(start, point(px(1928.), px(8.)));
    }
}
