mod accounts;
mod backend;
mod clipboard;
mod demo;
mod icons;
mod lock;
mod reprompt;
mod settings;
mod sign_in;
mod vault;
mod window;

use std::sync::Arc;

use gpui::{App, KeyBinding, Menu, MenuItem, actions};
use rbw_desktop_core::Backend;

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

    let backend = backend::connect();

    gpui::application()
        .with_assets(rbw_desktop_ui::Assets)
        .run(move |cx| {
            rbw_desktop_ui::init(cx);
            sign_in::init(cx);
            vault::init(cx);
            settings::init(cx);
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
            let missing = match backend {
                Ok(backend) => {
                    start(backend, cx);
                    None
                }
                Err(error) => Some(error),
            };
            window::open(missing, cx);
            cx.activate(true);
        });
}

/// Starts what runs on the backend, once there is one.
pub fn start(backend: Arc<dyn Backend>, cx: &mut App) {
    backend::init(backend, cx);
    clipboard::init(cx);
    icons::init(cx);
    accounts::init(cx);
}

fn menus() -> Vec<Menu> {
    vec![
        Menu::new("rbw-desktop").items([
            MenuItem::action("About rbw-desktop", settings::About),
            MenuItem::separator(),
            MenuItem::action("Settings…", settings::OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Quit rbw-desktop", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Item", vault::NewItem),
            MenuItem::action("Delete Item…", vault::DeleteItem),
            MenuItem::separator(),
            MenuItem::action("Add Account…", window::AddAccount),
        ]),
        Menu::new("Vault").items([
            MenuItem::action("Find", vault::Find),
            MenuItem::separator(),
            MenuItem::action("Sync Now", vault::Sync),
            MenuItem::action("Lock", vault::Lock),
        ]),
    ]
}
