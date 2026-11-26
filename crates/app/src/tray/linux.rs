//! The item as a StatusNotifierItem, through ksni, which reports clicks but
//! not where the item is. The host shows it by its icon name, from a theme
//! folder the app writes, as a symbolic icon it can recolour; the pixmaps
//! are for hosts that don't look names up.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use futures::channel::mpsc::UnboundedSender;
use gpui::{App, AppContext as _, DevicePixels, SvgRenderer, SvgSize, Task, point, px, size};
use ksni::TrayMethods as _;
use ksni::menu::{MenuItem, StandardItem};

use rbw_desktop_core::{APP_ID, APP_NAME};

use super::Event;
use crate::panel::Anchor;

/// The icon's name in the folder the app writes for the host.
fn icon_name() -> String {
    format!("{APP_ID}-symbolic")
}

/// Where a click on the item was, in X11's pixels, as the host says.
pub struct Click {
    x: i32,
    y: i32,
}

pub struct Item {
    handle: ksni::Handle<Tray>,
    /// Whether a host shows the item: it registered with the tray's
    /// watcher, which was running or came since.
    hosted: Arc<AtomicBool>,
}

impl Item {
    /// Registers the item with the tray's watcher. Without a session bus
    /// there is no item. Without a watcher, as on GNOME without the
    /// extension, there is one that no host shows, until a watcher starts.
    pub fn new(
        events: UnboundedSender<Event>,
        icon: &'static [u8],
        cx: &mut App,
    ) -> Task<Result<Self, String>> {
        let hosted = Arc::new(AtomicBool::new(true));
        let renderer = cx.svg_renderer();
        let mut tray = Tray {
            events,
            hosted: hosted.clone(),
            theme_path: String::new(),
            pixmaps: vec![],
            can_lock: false,
        };
        // Drawing the icon at each size and writing it out take a moment,
        // which the app's windows don't wait for.
        cx.background_spawn(async move {
            tray.pixmaps = pixmaps(icon, &renderer);
            tray.theme_path = write_icon(icon).unwrap_or_default();
            let handle = tray
                .assume_sni_available(true)
                .spawn()
                .await
                .map_err(|error| error.to_string())?;
            Ok(Self { handle, hosted })
        })
    }

    pub fn shown(&self) -> bool {
        self.hosted.load(Ordering::Acquire)
    }

    pub fn set_can_lock(&mut self, can_lock: bool, cx: &mut App) {
        let handle = self.handle.clone();
        cx.background_spawn(async move { handle.update(|tray| tray.can_lock = can_lock).await })
            .detach();
    }

    /// Next to the pointer, on X11. Under Wayland the app is a client of
    /// XWayland, whose coordinates aren't the host's, and the compositor
    /// places new windows itself.
    pub fn anchor(&self, click: Click, cx: &mut App) -> Anchor {
        if (click.x, click.y) == (0, 0) || std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return Anchor::Centered;
        }
        // The host gives pixels, windows open in points; every window of
        // the app has the same scale on X11. With none open, the scale seen
        // last, or none.
        let scale = cx
            .windows()
            .into_iter()
            .find_map(|handle| handle.update(cx, |_, window, _| window.scale_factor()).ok());
        let scale = match scale {
            Some(scale) => {
                LAST_SCALE.store(scale.to_bits(), Ordering::Relaxed);
                scale
            }
            None => f32::from_bits(LAST_SCALE.load(Ordering::Relaxed)),
        };
        let scale = if scale.is_finite() && scale > 0. {
            scale
        } else {
            1.
        };
        Anchor::Pointer(point(
            px(click.x as f32 / scale),
            px(click.y as f32 / scale),
        ))
    }
}

impl Drop for Item {
    fn drop(&mut self) {
        drop(self.handle.shutdown());
    }
}

struct Tray {
    events: UnboundedSender<Event>,
    hosted: Arc<AtomicBool>,
    theme_path: String,
    pixmaps: Vec<ksni::Icon>,
    can_lock: bool,
}

impl Tray {
    fn send(&self, event: Event) {
        // the app is quitting when nothing receives
        self.events.unbounded_send(event).ok();
    }

    fn set_hosted(&self, hosted: bool) {
        self.hosted.store(hosted, Ordering::Release);
        self.send(Event::HostChanged);
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        APP_ID.into()
    }

    fn title(&self) -> String {
        APP_NAME.into()
    }

    fn icon_name(&self) -> String {
        icon_name()
    }

    fn icon_theme_path(&self) -> String {
        self.theme_path.clone()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.pixmaps.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: APP_NAME.into(),
            ..Default::default()
        }
    }

    fn activate(&mut self, x: i32, y: i32) {
        self.send(Event::Toggle(Click { x, y }));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open Vault".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Event::OpenVault)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Lock".into(),
                enabled: self.can_lock,
                activate: Box::new(|tray: &mut Self| tray.send(Event::Lock)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Event::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }

    fn watcher_online(&self) {
        self.set_hosted(true);
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        log::info!("the system tray went away: {reason:?}");
        self.set_hosted(false);
        // wait for one to come
        true
    }
}

/// Writes the icon into a folder of the session for the host to find by
/// name, and gives the folder. Plasma wants it to be called `icons`; GTK
/// finds icons at the top of such a folder.
fn write_icon(svg: &[u8]) -> Option<String> {
    let dir = rbw_desktop_core::session_dir().ok()?.join("icons");
    let written = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join(format!("{}.svg", icon_name())), svg));
    if let Err(error) = written {
        log::warn!("writing the menu bar icon: {error}");
        return None;
    }
    Some(dir.to_string_lossy().into_owned())
}

/// The scale of the app's windows when one was seen last, as an `f32`'s
/// bits.
static LAST_SCALE: AtomicU32 = AtomicU32::new(f32::to_bits(1.));

/// The icon at a few sizes, in a grey that shows on light and dark panels.
fn pixmaps(svg: &[u8], renderer: &SvgRenderer) -> Vec<ksni::Icon> {
    const GREY: u8 = 0x8c;
    let Ok(parsed) = renderer.parse_svg(svg) else {
        return vec![];
    };
    [16, 22, 32, 44, 64]
        .into_iter()
        .filter_map(|side| {
            let pixels = size(DevicePixels(side), DevicePixels(side));
            let image = renderer
                .render_parsed(&parsed, SvgSize::Size(pixels))
                .ok()?;
            // BGRA, of which a one-colour icon needs only the alpha; SNI
            // wants ARGB
            let data = image
                .as_bytes(0)?
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|pixel| [pixel[3], GREY, GREY, GREY])
                .collect();
            Some(ksni::Icon {
                width: side,
                height: side,
                data,
            })
        })
        .collect()
}
