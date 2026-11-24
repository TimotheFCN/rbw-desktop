//! The app's preferences, read once at start and shared by every window:
//! Settings changes them, the generator remembers its options in them, the
//! menu bar item follows them. A change applies at once and is saved; a save
//! that fails puts them back. Views observe [`Prefs`] to follow them.

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global};
use rbw_desktop_core::{Error, Preferences};

use crate::backend;
use crate::clipboard::ClearAfter;
use crate::icons::Icons;

pub struct Prefs {
    preferences: Preferences,
}

/// A change could not be saved; the preferences are back as they were.
pub struct SaveFailed(pub Error);

impl EventEmitter<SaveFailed> for Prefs {}

struct Shared(Entity<Prefs>);

impl Global for Shared {}

/// Reads the preferences. Call once the backend is set up.
pub fn init(cx: &mut App) {
    let prefs = cx.new(|cx| {
        backend::request(
            cx,
            |backend| backend.preferences(),
            |this: &mut Prefs, result, cx| {
                match result {
                    Ok(preferences) => this.preferences = preferences,
                    Err(error) => log::warn!("reading the preferences: {error}"),
                }
                apply(&this.preferences, cx);
                cx.notify();
            },
        )
        .detach();
        Prefs {
            preferences: Preferences::default(),
        }
    });
    cx.set_global(Shared(prefs));
}

impl Prefs {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<Shared>().0.clone()
    }

    pub fn get(&self) -> &Preferences {
        &self.preferences
    }

    /// Changes the preferences now and saves them.
    pub fn update(&mut self, change: impl FnOnce(&mut Preferences), cx: &mut Context<Self>) {
        let before = self.preferences.clone();
        change(&mut self.preferences);
        if self.preferences == before {
            return;
        }
        let saved = self.preferences.clone();
        apply(&saved, cx);
        cx.notify();
        backend::request(
            cx,
            move |backend| backend.set_preferences(&saved),
            move |this, result, cx| {
                if let Err(error) = result {
                    log::warn!("saving the preferences: {error}");
                    apply(&before, cx);
                    this.preferences = before;
                    cx.emit(SaveFailed(error));
                    cx.notify();
                }
            },
        )
        .detach();
    }
}

/// [`Prefs::update`] from anywhere.
pub fn update(cx: &mut App, change: impl FnOnce(&mut Preferences)) {
    Prefs::global(cx).update(cx, |prefs, cx| prefs.update(change, cx));
}

/// Makes the rest of the app follow the preferences.
fn apply(preferences: &Preferences, cx: &mut App) {
    cx.set_global(ClearAfter(preferences.clipboard_clear));
    let website_icons = preferences.website_icons;
    Icons::global(cx).update(cx, |icons, cx| icons.set_enabled(website_icons, cx));
}
