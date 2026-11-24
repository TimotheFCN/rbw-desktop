//! Websites' icons, shown in place of a login's letter tile when the
//! preference allows.
//!
//! An icon rbw keeps on disk is read off the main thread, a batch at a
//! time: for the first rows of a list before it shows (`read_cached` with
//! the items), for the rows about to scroll in ([`Icons::prefetch`]) and
//! for any row drawn. One never kept is asked of the server only while a
//! row that shows it is drawn, a few at a time on threads of their own: the
//! first time, rbw asks the server, which can take seconds, and that must
//! never hold up what people wait for. Until an icon is known its tile is
//! neutral; when one arrives, the views observing [`Icons`] redraw and it
//! fades in. A website known to have none shows its letter. Icons are
//! decoded small, off the main thread, and the most recently shown are
//! kept.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, AppContext as _, Context, Entity, Global, RenderImage, Task};
use image::imageops::FilterType;
use rbw_desktop_core::{AccountId, Backend, IconFormat, ItemKind, ItemSummary, WebsiteIcon};
use rbw_desktop_ui::TileIcon;

use crate::backend;

/// The largest an icon is kept, in pixels: the item header's tile at twice
/// the size of a point.
const PIXELS: u32 = 128;
/// How many icons are asked of the server at once.
const AT_ONCE: usize = 3;
/// How many icons are kept; the ones shown longest ago go first.
const KEPT: usize = 400;
/// How long after a failure, such as being offline, an icon is asked again.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// How long a website known to have no icon shows its letter before rbw
/// is asked again, which answers from its own record of it for days.
const MISSING_FOR: Duration = Duration::from_secs(3600);

// An icon is the same for every login of a website, in an account.
type Key = (AccountId, String);

enum Slot {
    /// Being read from rbw's cache. `letter` keeps the letter on screen
    /// meanwhile, for a website that had none.
    Reading {
        letter: bool,
    },
    /// Not in rbw's cache: asked of the server when a row shows it.
    Uncached,
    /// Asked of the server; dropping the task drops the answer.
    Fetching {
        letter: bool,
        _answer: Task<()>,
    },
    Ready {
        image: Arc<RenderImage>,
        shown: Instant,
    },
    Missing(Instant),
    Failed(Instant),
}

/// Icons read from rbw's cache, to put in [`Icons`] with
/// [`Icons::insert_cached`].
pub struct Cached(Vec<(Key, Option<Option<Arc<RenderImage>>>)>);

pub struct Icons {
    enabled: bool,
    icons: HashMap<Key, Slot>,
    /// How many of them are ready, to know without counting when some
    /// must go.
    ready: usize,
    fetching: usize,
    /// Icons to read from rbw's cache once the current update is over,
    /// together.
    to_read: Vec<(Key, ItemSummary)>,
}

struct Shared(Entity<Icons>);

impl Global for Shared {}

/// Shows no icons until the preferences allow them ([`crate::preferences`]).
pub fn init(cx: &mut App) {
    let icons = cx.new(|_| Icons {
        enabled: false,
        icons: HashMap::new(),
        ready: 0,
        fetching: 0,
        to_read: vec![],
    });
    cx.set_global(Shared(icons));
}

/// The website an item's icon is kept under, if it is a login with one.
fn key(account: &AccountId, item: &ItemSummary) -> Option<Key> {
    (item.kind == ItemKind::Login)
        .then(|| Some((account.clone(), item.websites.first()?.clone())))
        .flatten()
}

impl Icons {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<Shared>().0.clone()
    }

    /// Shows icons or stops; stopping forgets them.
    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        if !enabled {
            for slot in std::mem::take(&mut self.icons).into_values() {
                if let Slot::Ready { image, .. } = slot {
                    cx.drop_image(image, None);
                }
            }
            // The answers on the way were dropped with their slots.
            self.ready = 0;
            self.fetching = 0;
            self.to_read.clear();
        }
        cx.notify();
    }

    /// What the item's tile shows. Call it while drawing the item: an icon
    /// not known yet is read from rbw's cache, or asked of the server if
    /// it isn't there and not too many are on the way already.
    pub fn icon(
        &mut self,
        account: &AccountId,
        item: &ItemSummary,
        cx: &mut Context<Self>,
    ) -> TileIcon {
        let Some(key) = key(account, item).filter(|_| self.enabled) else {
            return TileIcon::Letter;
        };
        let letter = match self.icons.get_mut(&key) {
            Some(Slot::Ready { image, shown }) => {
                *shown = Instant::now();
                return TileIcon::Image(image.clone());
            }
            Some(Slot::Reading { letter } | Slot::Fetching { letter, .. }) => {
                return if *letter {
                    TileIcon::Letter
                } else {
                    TileIcon::Pending
                };
            }
            Some(Slot::Missing(at)) if at.elapsed() < MISSING_FOR => return TileIcon::Letter,
            Some(Slot::Failed(at)) if at.elapsed() < RETRY_AFTER => return TileIcon::Letter,
            Some(Slot::Missing(_)) => {
                self.read(key, item, true, cx);
                return TileIcon::Letter;
            }
            None => {
                self.read(key, item, false, cx);
                return TileIcon::Pending;
            }
            Some(Slot::Failed(_)) => true,
            Some(Slot::Uncached) => false,
        };
        if self.fetching < AT_ONCE {
            self.fetch(key, item, letter, cx);
        }
        if letter {
            TileIcon::Letter
        } else {
            TileIcon::Pending
        }
    }

    /// Reads the icons of rows about to show from rbw's cache, so that
    /// they are there when the rows are drawn. Asks no server.
    pub fn prefetch<'a>(
        &mut self,
        account: &AccountId,
        items: impl IntoIterator<Item = &'a ItemSummary>,
        cx: &mut Context<Self>,
    ) {
        if !self.enabled {
            return;
        }
        for item in items {
            if let Some(key) = key(account, item)
                && !self.icons.contains_key(&key)
            {
                self.read(key, item, false, cx);
            }
        }
    }

    /// Puts icons read from rbw's cache before a list showed in place.
    pub fn insert_cached(&mut self, cached: Cached, cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }
        for (key, image) in cached.0 {
            self.settle(key, image);
        }
        self.forget_old(cx);
        cx.notify();
    }

    fn settle(&mut self, key: Key, image: Option<Option<Arc<RenderImage>>>) {
        let slot = match image {
            Some(Some(image)) => Slot::Ready {
                image,
                shown: Instant::now(),
            },
            Some(None) => Slot::Missing(Instant::now()),
            None => Slot::Uncached,
        };
        self.put(key, slot);
    }

    fn put(&mut self, key: Key, slot: Slot) {
        let ready = matches!(slot, Slot::Ready { .. });
        let was_ready = matches!(self.icons.insert(key, slot), Some(Slot::Ready { .. }));
        self.ready = self.ready + usize::from(ready) - usize::from(was_ready);
    }

    /// Reads the icon from rbw's cache with the others asked for in this
    /// update.
    fn read(&mut self, key: Key, item: &ItemSummary, letter: bool, cx: &mut Context<Self>) {
        self.put(key.clone(), Slot::Reading { letter });
        self.to_read.push((key, item.clone()));
        if self.to_read.len() > 1 {
            return;
        }
        cx.spawn(async move |this, cx| {
            let reading = this.update(cx, |this, cx| {
                let wanted = std::mem::take(&mut this.to_read);
                backend::call(cx, move |backend| Ok(read(backend, wanted)))
            });
            let Ok(reading) = reading else { return };
            if let Ok(cached) = reading.await {
                this.update(cx, |this, cx| {
                    // What was asked before icons were turned off and on
                    // again is asked again.
                    for (key, image) in cached.0 {
                        if matches!(this.icons.get(&key), Some(Slot::Reading { .. })) {
                            this.settle(key, image);
                        }
                    }
                    this.forget_old(cx);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Asks rbw for the icon, which asks the server.
    fn fetch(&mut self, key: Key, item: &ItemSummary, letter: bool, cx: &mut Context<Self>) {
        self.fetching += 1;
        let (account, item) = (key.0.clone(), item.clone());
        let answer_key = key.clone();
        let answer = backend::request_on_thread(
            "website icon",
            cx,
            move |backend| Ok(backend.website_icon(&account, &item)?.and_then(decode)),
            move |this, result, cx| {
                this.fetching -= 1;
                let slot = match result {
                    Ok(Some(image)) => Slot::Ready {
                        image,
                        shown: Instant::now(),
                    },
                    Ok(None) => Slot::Missing(Instant::now()),
                    Err(error) => {
                        log::info!("a website icon: {error}");
                        Slot::Failed(Instant::now())
                    }
                };
                this.put(answer_key, slot);
                this.forget_old(cx);
                cx.notify();
            },
        );
        self.put(
            key,
            Slot::Fetching {
                letter,
                _answer: answer,
            },
        );
    }

    /// Forgets the icons shown longest ago, beyond the ones kept.
    fn forget_old(&mut self, cx: &mut Context<Self>) {
        if self.ready <= KEPT {
            return;
        }
        let mut ready: Vec<(Instant, Key)> = self
            .icons
            .iter()
            .filter_map(|(key, slot)| match slot {
                Slot::Ready { shown, .. } => Some((*shown, key.clone())),
                _ => None,
            })
            .collect();
        ready.sort_unstable_by_key(|(shown, _)| *shown);
        for (_, key) in &ready[..ready.len() - KEPT] {
            if let Some(Slot::Ready { image, .. }) = self.icons.remove(key) {
                self.ready -= 1;
                cx.drop_image(image, None);
            }
        }
    }
}

/// [`Icons::icon`] from anywhere.
pub fn icon(account: &AccountId, item: &ItemSummary, cx: &mut App) -> TileIcon {
    Icons::global(cx).update(cx, |icons, cx| icons.icon(account, item, cx))
}

/// Reads the icons of `items` that rbw keeps on disk and decodes them, off
/// the main thread, for [`Icons::insert_cached`]: the icons of a list's
/// first rows, read with the list so that they show with it.
pub fn read_cached<'a>(
    backend: &dyn Backend,
    account: &AccountId,
    items: impl IntoIterator<Item = &'a ItemSummary>,
) -> Cached {
    let mut wanted: Vec<(Key, ItemSummary)> = vec![];
    for item in items {
        if let Some(key) = key(account, item)
            && !wanted.iter().any(|(known, _)| *known == key)
        {
            wanted.push((key, item.clone()));
        }
    }
    read(backend, wanted)
}

fn read(backend: &dyn Backend, wanted: Vec<(Key, ItemSummary)>) -> Cached {
    Cached(
        wanted
            .into_iter()
            .map(|(key, item)| {
                // A cache that can't be read is as good as empty.
                let cached = backend
                    .cached_website_icon(&key.0, &item)
                    .unwrap_or_default();
                // One that can't be decoded is as good as none.
                (key, cached.map(|icon| icon.and_then(decode)))
            })
            .collect(),
    )
}

/// The icon as an image to draw, at most [`PIXELS`] a side.
fn decode(icon: WebsiteIcon) -> Option<Arc<RenderImage>> {
    let format = match icon.format {
        IconFormat::Png => image::ImageFormat::Png,
        IconFormat::Jpeg => image::ImageFormat::Jpeg,
        IconFormat::Gif => image::ImageFormat::Gif,
        IconFormat::Webp => image::ImageFormat::WebP,
        IconFormat::Bmp => image::ImageFormat::Bmp,
        IconFormat::Ico => image::ImageFormat::Ico,
        // The backend passes no SVG on: one can refer to other files.
        IconFormat::Svg => return None,
    };
    let decoded = match image::load_from_memory_with_format(&icon.bytes, format) {
        Ok(decoded) => decoded,
        Err(error) => {
            log::info!("a website icon that can't be read: {error}");
            return None;
        }
    };
    let decoded = if decoded.width() > PIXELS || decoded.height() > PIXELS {
        decoded.resize(PIXELS, PIXELS, FilterType::Lanczos3)
    } else {
        decoded
    };
    let mut pixels = decoded.into_rgba8();
    // GPUI draws BGRA.
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new([image::Frame::new(pixels)])))
}
