mod backend;
mod clipboard;
mod vault;

use std::sync::Arc;

use gpui::{App, AppContext as _, KeyBinding, Menu, MenuItem, actions};
use rbw_desktop_core::{Backend, Error, FakeBackend, LockState};

/// A key binding with the platform's command modifier: Command on macOS,
/// Control elsewhere.
#[macro_export]
macro_rules! secondary {
    ($key:literal) => {
        if cfg!(target_os = "macos") {
            concat!("cmd-", $key)
        } else {
            concat!("ctrl-", $key)
        }
    };
}

actions!(rbw_desktop, [Quit]);

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // rbw-desktop runs on the demo vault until the rbw backend lands.
    let fake = FakeBackend::new();
    let backend: Arc<dyn Backend> = Arc::new(fake);

    gpui::application()
        .with_assets(rbw_desktop_ui::Assets)
        .run(move |cx| {
            rbw_desktop_ui::init(cx);
            backend::init(backend.clone(), cx);
            cx.set_global(clipboard::ClearAfter(
                rbw_desktop_core::Preferences::default().clipboard_clear,
            ));
            vault::init(cx);
            cx.bind_keys([KeyBinding::new(secondary!("q"), Quit, None)]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.set_menus(menus());
            // Without a menu bar item to come back to, closing the last
            // window ends the app.
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            open_first_account(backend, cx);
            cx.activate(true);
        });
}

/// Opens the vault of the first account, unlocked: the lock screen and the
/// account switcher are not built yet.
fn open_first_account(backend: Arc<dyn Backend>, cx: &mut App) {
    let unlocked = cx.background_spawn(async move {
        let account = backend
            .accounts()?
            .into_iter()
            .next()
            .ok_or(Error::AccountNotFound)?;
        if account.state != LockState::Unlocked {
            backend.unlock(&account.id, &FakeBackend::PASSWORD.into())?;
        }
        backend.account(&account.id)
    });
    cx.spawn(async move |cx| match unlocked.await {
        Ok(account) => cx.update(|cx| vault::open(account, cx)),
        Err(error) => {
            log::error!("opening the demo vault: {error}");
            cx.update(|cx| cx.quit());
        }
    })
    .detach();
}

fn menus() -> Vec<Menu> {
    vec![
        Menu::new("rbw-desktop").items([MenuItem::action("Quit rbw-desktop", Quit)]),
        Menu::new("File").items([MenuItem::action("New Item", vault::NewItem)]),
        Menu::new("Vault").items([
            MenuItem::action("Find", vault::Find),
            MenuItem::separator(),
            MenuItem::action("Sync Now", vault::Sync),
            MenuItem::action("Lock", vault::Lock),
        ]),
    ]
}
