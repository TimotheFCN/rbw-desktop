//! The menu bar item: in the macOS menu bar, or in the system tray of Linux
//! desktops that show StatusNotifierItems, such as KDE Plasma or GNOME with
//! the AppIndicator extension. A click opens the panel ([`crate::panel`]);
//! its menu has Open Vault, Lock and Quit. It follows the preference to
//! show it. Where nothing shows it, as on GNOME without the extension, the
//! app is as it would be without one: closing its last window quits it.

#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(not(target_os = "macos"), path = "linux.rs")]
mod platform;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, AppContext as _, Context, Entity, Global, Subscription, Task};
use rbw_desktop_core::LockState;

use crate::accounts::{self, Accounts};
use crate::backend;
use crate::panel::{self, Anchor};
use crate::preferences::Prefs;
use crate::window;

/// The symbolic icon: a key, in the colour of the text around it.
const ICON: &[u8] = include_bytes!("../../assets/menu-bar-symbolic.svg");

/// What the item asks of the app, from whatever thread the platform
/// reports it on.
pub enum Event {
    /// A click: the panel opens, or closes if it is open.
    Toggle(platform::Click),
    OpenVault,
    Lock,
    Quit,
    /// Something started or stopped showing the item.
    #[cfg(not(target_os = "macos"))]
    HostChanged,
}

pub struct MenuBar {
    item: Option<platform::Item>,
    /// Whether the item is being set up, which can take a moment on Linux.
    starting: Option<Task<()>>,
    events: UnboundedSender<Event>,
    _subscriptions: [Subscription; 2],
    _events: Task<()>,
}

struct Shared(Entity<MenuBar>);

impl Global for Shared {}

/// Shows the item once the preferences say to. Call once the backend is
/// set up.
pub fn init(cx: &mut App) {
    let menu_bar = cx.new(|cx| {
        let (events, mut received) = unbounded();
        let handle_events = cx.spawn(async move |this, cx| {
            while let Some(event) = received.next().await {
                if this
                    .update(cx, |this: &mut MenuBar, cx| this.handle(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut this = MenuBar {
            item: None,
            starting: None,
            events,
            _subscriptions: [
                cx.observe(&Prefs::global(cx), |this, _, cx| this.follow_preference(cx)),
                cx.observe(&Accounts::global(cx), |this, _, cx| {
                    this.follow_accounts(cx)
                }),
            ],
            _events: handle_events,
        };
        this.follow_preference(cx);
        this
    });
    cx.set_global(Shared(menu_bar));
}

/// Whether closing the last window leaves the app running: while the menu
/// bar item shows, to open it again from.
pub fn keeps_app_running(cx: &App) -> bool {
    MenuBar::global(cx).is_some_and(|menu_bar| menu_bar.read(cx).shown())
}

/// What the preference to show the item is called here.
pub const SETTING: &str = if cfg!(target_os = "macos") {
    "Show in menu bar"
} else {
    "Show in system tray"
};

impl MenuBar {
    /// `None` until there is a backend.
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<Shared>().map(|shared| shared.0.clone())
    }

    /// Whether something shows the item.
    pub fn shown(&self) -> bool {
        self.item.as_ref().is_some_and(platform::Item::shown)
    }

    /// Whether the item is on but nothing shows it, such as on GNOME
    /// without its AppIndicator extension.
    pub fn unseen(&self) -> bool {
        self.starting.is_none() && !self.shown()
    }
}

impl MenuBar {
    fn follow_preference(&mut self, cx: &mut Context<Self>) {
        let prefs = Prefs::global(cx);
        let prefs = prefs.read(cx);
        let wanted = prefs.loaded() && prefs.get().menu_bar;
        match (wanted, self.item.is_some() || self.starting.is_some()) {
            (true, false) => self.show(cx),
            (false, true) => {
                self.starting = None;
                self.item = None;
                cx.notify();
            }
            _ => {}
        }
    }

    fn show(&mut self, cx: &mut Context<Self>) {
        let starting = platform::Item::new(self.events.clone(), ICON, cx);
        self.starting = Some(cx.spawn(async move |this, cx| {
            let item = starting.await;
            this.update(cx, |this, cx| {
                this.starting = None;
                match item {
                    Ok(item) => {
                        this.item = Some(item);
                        this.follow_accounts(cx);
                    }
                    Err(error) => log::info!("no menu bar item: {error}"),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Lock is there while the account the windows show is unlocked.
    fn follow_accounts(&mut self, cx: &mut Context<Self>) {
        let unlocked = Accounts::global(cx)
            .read(cx)
            .current()
            .is_some_and(|account| account.state == LockState::Unlocked);
        if let Some(item) = &mut self.item {
            item.set_can_lock(unlocked, cx);
        }
    }

    fn handle(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            // Changes nothing Settings shows about the item.
            Event::Toggle(click) => {
                let anchor = match &self.item {
                    Some(item) => item.anchor(click, cx),
                    None => Anchor::Centered,
                };
                panel::toggle(anchor, cx);
            }
            Event::OpenVault => {
                window::with_main_window(cx, |_, _, _| {});
                cx.activate(true);
            }
            Event::Lock => {
                let Some(account) = Accounts::global(cx).read(cx).current().cloned() else {
                    return;
                };
                let locking = backend::call(cx, move |backend| backend.lock(&account.id));
                cx.spawn(async move |_, cx| match locking.await {
                    Ok(()) => cx.update(accounts::refresh),
                    Err(error) => log::warn!("locking from the menu bar: {error}"),
                })
                .detach();
            }
            Event::Quit => cx.quit(),
            // Whether something shows the item, which Settings says.
            #[cfg(not(target_os = "macos"))]
            Event::HostChanged => cx.notify(),
        }
    }
}
