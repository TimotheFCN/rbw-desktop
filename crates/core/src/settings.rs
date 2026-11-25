use std::time::Duration;

use crate::{GeneratorOptions, OtherRbw};

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
    /// Where the main window was last, to open there again.
    pub window: Option<WindowPlace>,
    /// The other `rbw` whose notice the person dismissed in the main
    /// window, which says nothing more until another is found.
    pub dismissed_rbw: Option<OtherRbw>,
}

/// Where a window is on the screen, in points, and whether it fills it.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowPlace {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Maximized, at `x`, `y`, `width` and `height` once it is no longer.
    pub maximized: bool,
    /// The display it was on, by its UUID. On macOS `x` and `y` count
    /// from that display's top-left corner; elsewhere from the whole
    /// screen's.
    pub display: Option<String>,
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
            window: None,
            dismissed_rbw: None,
        }
    }
}
