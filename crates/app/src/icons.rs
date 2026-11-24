//! Websites' icons, shown in place of a login's letter tile when the
//! preference allows.
//!
//! An icon is asked for only while a row that shows it is drawn, a few at a
//! time on threads of their own: the first time, rbw asks the server, which
//! can take seconds, and that must never hold up what people wait for.
//! When one arrives, the views observing [`Icons`] redraw, and the rows
//! still on screen ask for the next ones; rows scrolled past ask for
//! nothing. Icons are decoded small, off the main thread, and the most
//! recently shown are kept.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, AppContext as _, Context, Entity, Global, RenderImage, Task};
use image::imageops::FilterType;
use rbw_desktop_core::{AccountId, IconFormat, ItemKind, ItemSummary, WebsiteIcon};

use crate::backend;

/// The largest an icon is kept, in pixels: the item header's tile at twice
/// the size of a point.
const PIXELS: u32 = 128;
/// How many icons are asked for at once.
const AT_ONCE: usize = 3;
/// How many icons are kept; the ones shown longest ago go first.
const KEPT: usize = 400;
/// How long after a failure, such as being offline, an icon is asked again.
const RETRY_AFTER: Duration = Duration::from_secs(60);

// An icon is the same for every login of a website, in an account.
type Key = (AccountId, String);

enum Slot {
    // Dropping the task drops the answer.
    Loading {
        _answer: Task<()>,
    },
    Ready {
        image: Option<Arc<RenderImage>>,
        shown: Instant,
    },
    Failed(Instant),
}

pub struct Icons {
    enabled: bool,
    icons: HashMap<Key, Slot>,
    loading: usize,
}

struct Shared(Entity<Icons>);

impl Global for Shared {}

/// Shows no icons until the preferences allow them ([`crate::preferences`]).
pub fn init(cx: &mut App) {
    let icons = cx.new(|_| Icons {
        enabled: false,
        icons: HashMap::new(),
        loading: 0,
    });
    cx.set_global(Shared(icons));
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
                if let Slot::Ready {
                    image: Some(image), ..
                } = slot
                {
                    cx.drop_image(image, None);
                }
            }
            // The answers on the way were dropped with their slots.
            self.loading = 0;
        }
        cx.notify();
    }

    /// The item's icon once it has arrived, and `None` until then, when it
    /// has none, or while icons are off. Call it while drawing the item:
    /// the first call for a website asks for its icon, if not too many are
    /// on the way already.
    pub fn icon(
        &mut self,
        account: &AccountId,
        item: &ItemSummary,
        cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        if !self.enabled || item.kind != ItemKind::Login {
            return None;
        }
        let key = (account.clone(), item.websites.first()?.clone());
        match self.icons.get_mut(&key) {
            Some(Slot::Ready { image, shown }) => {
                *shown = Instant::now();
                return image.clone();
            }
            Some(Slot::Loading { .. }) => return None,
            Some(Slot::Failed(at)) if at.elapsed() < RETRY_AFTER => return None,
            Some(Slot::Failed(_)) | None => {}
        }
        if self.loading >= AT_ONCE {
            return None;
        }
        self.loading += 1;
        let (account, item) = (account.clone(), item.clone());
        let answer_key = key.clone();
        let answer = backend::request_on_thread(
            "website icon",
            cx,
            move |backend| Ok(backend.website_icon(&account, &item)?.and_then(decode)),
            move |this, result, cx| {
                this.loading -= 1;
                let slot = match result {
                    Ok(image) => Slot::Ready {
                        image,
                        shown: Instant::now(),
                    },
                    Err(error) => {
                        log::info!("a website icon: {error}");
                        Slot::Failed(Instant::now())
                    }
                };
                this.icons.insert(answer_key, slot);
                this.forget_old(cx);
                cx.notify();
            },
        );
        self.icons.insert(key, Slot::Loading { _answer: answer });
        None
    }

    /// Forgets the icons shown longest ago, beyond the ones kept.
    fn forget_old(&mut self, cx: &mut Context<Self>) {
        let mut ready: Vec<(Instant, Key)> = self
            .icons
            .iter()
            .filter_map(|(key, slot)| match slot {
                Slot::Ready { shown, .. } => Some((*shown, key.clone())),
                Slot::Loading { .. } | Slot::Failed(_) => None,
            })
            .collect();
        if ready.len() <= KEPT {
            return;
        }
        ready.sort_unstable_by_key(|(shown, _)| *shown);
        for (_, key) in &ready[..ready.len() - KEPT] {
            if let Some(Slot::Ready {
                image: Some(image), ..
            }) = self.icons.remove(key)
            {
                cx.drop_image(image, None);
            }
        }
    }
}

/// [`Icons::icon`] from anywhere.
pub fn icon(account: &AccountId, item: &ItemSummary, cx: &mut App) -> Option<Arc<RenderImage>> {
    Icons::global(cx).update(cx, |icons, cx| icons.icon(account, item, cx))
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
