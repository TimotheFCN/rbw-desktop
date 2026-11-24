//! The app's preferences, read once at start and shared by every window:
//! Settings changes them, the generator remembers its options in them, the
//! menu bar item follows them. A change applies at once and is saved; a save
//! that fails puts them back. Views observe [`Prefs`] to follow them.

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global};
use rbw_desktop_core::{Error, Preferences};

use crate::backend;
use crate::clipboard;
use crate::icons::Icons;

type Change = Box<dyn Fn(&mut Preferences)>;
type Then = Box<dyn FnOnce(&mut App)>;

pub struct Prefs {
    preferences: Preferences,
    /// Whether they have been read; until then they are the defaults.
    loaded: bool,
    /// Changes made before they were read, made again on what was read.
    early: Vec<Change>,
    /// What waits for them to be read.
    waiting: Vec<Then>,
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
                this.loaded = true;
                match result {
                    Ok(read) => {
                        let early = std::mem::take(&mut this.early);
                        this.preferences = read.clone();
                        for change in &early {
                            change(&mut this.preferences);
                        }
                        if this.preferences != read {
                            this.save(read, cx);
                        }
                    }
                    Err(error) => log::warn!("reading the preferences: {error}"),
                }
                apply(&this.preferences, cx);
                cx.notify();
                for then in std::mem::take(&mut this.waiting) {
                    cx.defer(then);
                }
            },
        )
        .detach();
        Prefs {
            preferences: Preferences::default(),
            loaded: false,
            early: vec![],
            waiting: vec![],
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

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    /// Changes the preferences now and saves them. A change made before
    /// they were read is saved on top of what was read, so that it never
    /// writes the defaults over the others.
    pub fn update(&mut self, change: impl Fn(&mut Preferences) + 'static, cx: &mut Context<Self>) {
        let before = self.preferences.clone();
        change(&mut self.preferences);
        if self.preferences == before {
            return;
        }
        apply(&self.preferences, cx);
        cx.notify();
        if self.loaded {
            self.save(before, cx);
        } else {
            self.early.push(Box::new(change));
        }
    }

    /// Saves the preferences, or puts back `before` if that fails.
    fn save(&mut self, before: Preferences, cx: &mut Context<Self>) {
        let saved = self.preferences.clone();
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

/// Runs `then` once the preferences have been read, at once if they have,
/// such as opening the main window where it was last.
pub fn when_read(cx: &mut App, then: impl FnOnce(&mut App) + 'static) {
    let prefs = Prefs::global(cx);
    if prefs.read(cx).loaded {
        then(cx);
    } else {
        prefs.update(cx, |prefs, _| prefs.waiting.push(Box::new(then)));
    }
}

/// [`Prefs::update`] from anywhere.
pub fn update(cx: &mut App, change: impl Fn(&mut Preferences) + 'static) {
    Prefs::global(cx).update(cx, |prefs, cx| prefs.update(change, cx));
}

/// Makes the rest of the app follow the preferences.
fn apply(preferences: &Preferences, cx: &mut App) {
    clipboard::set_clear_after(preferences.clipboard_clear, cx);
    let website_icons = preferences.website_icons;
    Icons::global(cx).update(cx, |icons, cx| icons.set_enabled(website_icons, cx));
}
