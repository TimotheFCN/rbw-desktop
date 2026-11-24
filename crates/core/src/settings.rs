use std::time::Duration;

use crate::GeneratorOptions;

/// The app's own preferences, the same for every account.
#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    /// How long a copied secret stays on the clipboard; `None` to leave it.
    pub clipboard_clear: Option<Duration>,
    /// Whether to show websites' icons, which asks the account's server for
    /// them. Turning it off also forgets the icons kept on disk.
    pub website_icons: bool,
    /// What the generator made last, to make the same kind next time.
    pub generator: GeneratorOptions,
    /// Whether the app has an item in the menu bar or the system tray.
    pub menu_bar: bool,
    /// The vault's item list, as a share of the room right of the sidebar;
    /// `None` for the app's default.
    pub list_share: Option<f32>,
    /// How the vault's item list is sorted.
    pub sort: SortOrder,
}

/// How the vault's item list is sorted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortOrder {
    /// By name, A to Z.
    #[default]
    Name,
    /// The item changed last first.
    Modified,
    /// The items used last in the app first, then the others by name.
    RecentlyUsed,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            clipboard_clear: Some(Duration::from_secs(30)),
            website_icons: true,
            generator: GeneratorOptions::default(),
            menu_bar: true,
            list_share: None,
            sort: SortOrder::Name,
        }
    }
}
