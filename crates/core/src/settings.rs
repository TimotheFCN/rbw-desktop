use std::time::Duration;

/// The app's own preferences, the same for every account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
    /// How long a copied secret stays on the clipboard; `None` to leave it.
    pub clipboard_clear: Option<Duration>,
    /// Whether to show websites' icons, which asks the account's server for
    /// them.
    pub website_icons: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            clipboard_clear: Some(Duration::from_secs(30)),
            website_icons: true,
        }
    }
}
