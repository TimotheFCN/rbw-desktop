//! Websites' icons, shown in place of a login's letter tile when the
//! preference allows. Each is asked of the backend and decoded off the main
//! thread once, then kept in memory, so scrolling never asks again. Views
//! that show icons observe [`Icons`] to redraw as they arrive.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext as _, Context, DevicePixels, Entity, Global, Image, ImageFormat, RenderImage,
    SvgRenderer, SvgSize, Task, size,
};
use rbw_desktop_core::{AccountId, IconFormat, ItemKind, ItemSummary, WebsiteIcon};

use crate::backend;

/// The size icons are drawn at, in pixels: the item header's tile at twice
/// the size of a point.
const PIXELS: i32 = 128;
/// How long after a failure, such as being offline, an icon is asked again.
const RETRY_AFTER: Duration = Duration::from_secs(60);

// An icon is the same for every login of a website, in an account.
type Key = (AccountId, String);

enum Slot {
    // dropping the request cancels it
    Loading { _request: Task<()> },
    Ready(Option<Arc<RenderImage>>),
    Failed(Instant),
}

pub struct Icons {
    enabled: bool,
    icons: HashMap<Key, Slot>,
}

struct Shared(Entity<Icons>);

impl Global for Shared {}

/// Shows no icons until the preferences allow them ([`crate::preferences`]).
pub fn init(cx: &mut App) {
    let icons = cx.new(|_| Icons {
        enabled: false,
        icons: HashMap::new(),
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
                if let Slot::Ready(Some(image)) = slot {
                    cx.drop_image(image, None);
                }
            }
        }
        cx.notify();
    }

    /// The item's icon once it has arrived, and `None` until then, when it
    /// has none, or while icons are off. The first call for a website asks
    /// for its icon.
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
        match self.icons.get(&key) {
            Some(Slot::Ready(image)) => return image.clone(),
            Some(Slot::Loading { .. }) => return None,
            Some(Slot::Failed(at)) if at.elapsed() < RETRY_AFTER => return None,
            Some(Slot::Failed(_)) | None => {}
        }
        let (account, item) = (account.clone(), item.clone());
        let svg = cx.svg_renderer();
        let task_key = key.clone();
        let task = backend::request(
            cx,
            move |backend| {
                let icon = backend.website_icon(&account, &item)?;
                Ok(icon.and_then(|icon| decode(icon, &svg)))
            },
            move |this, result, cx| {
                if !this.enabled {
                    return;
                }
                let slot = match result {
                    Ok(image) => Slot::Ready(image),
                    Err(error) => {
                        log::info!("a website icon: {error}");
                        Slot::Failed(Instant::now())
                    }
                };
                this.icons.insert(task_key, slot);
                cx.notify();
            },
        );
        self.icons.insert(key, Slot::Loading { _request: task });
        None
    }
}

/// [`Icons::icon`] from anywhere.
pub fn icon(account: &AccountId, item: &ItemSummary, cx: &mut App) -> Option<Arc<RenderImage>> {
    Icons::global(cx).update(cx, |icons, cx| icons.icon(account, item, cx))
}

/// The icon as an image to draw. Vector icons are drawn at the size they
/// are shown at; images are decoded as they are.
fn decode(icon: WebsiteIcon, svg: &SvgRenderer) -> Option<Arc<RenderImage>> {
    let format = match icon.format {
        IconFormat::Svg => {
            let parsed = svg.parse_svg(&icon.bytes).ok()?;
            let pixels = size(DevicePixels(PIXELS), DevicePixels(PIXELS));
            return svg.render_parsed(&parsed, SvgSize::Size(pixels)).ok();
        }
        IconFormat::Png => ImageFormat::Png,
        IconFormat::Jpeg => ImageFormat::Jpeg,
        IconFormat::Gif => ImageFormat::Gif,
        IconFormat::Webp => ImageFormat::Webp,
        IconFormat::Bmp => ImageFormat::Bmp,
        IconFormat::Ico => ImageFormat::Ico,
    };
    let image = Image::from_bytes(format, icon.bytes.to_vec());
    match image.to_image_data(svg.clone()) {
        Ok(image) => Some(image),
        Err(error) => {
            log::info!("a website icon that can't be read: {error:#}");
            None
        }
    }
}
