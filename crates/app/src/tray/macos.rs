//! The item in the macOS menu bar, through tray-icon: a template image,
//! which the menu bar tints, and its menu on a right-click. A left click
//! opens the panel under the item.

use futures::channel::mpsc::UnboundedSender;
use gpui::{App, Bounds, DevicePixels, DisplayId, Pixels, SvgSize, Task, point, px, size};
use objc2_foundation::{MainThreadMarker, NSNumber, NSRect, ns_string};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use rbw_desktop_core::APP_NAME;

use super::Event;
use crate::panel::Anchor;

/// The icon's height in pixels: the menu bar's 18 points, at twice the
/// size for Retina screens.
const ICON_PIXELS: i32 = 36;

/// A left click on the item.
pub struct Click;

pub struct Item {
    icon: TrayIcon,
    lock: MenuItem,
}

impl Item {
    pub fn new(
        events: UnboundedSender<Event>,
        icon: &'static [u8],
        cx: &mut App,
    ) -> Task<Result<Self, String>> {
        Task::ready(Self::create(events, icon, cx))
    }

    fn create(events: UnboundedSender<Event>, svg: &[u8], cx: &App) -> Result<Self, String> {
        let image = template(svg, cx).ok_or("the icon can't be drawn")?;
        let open = MenuItem::with_id("open-vault", "Open Vault", true, None);
        let lock = MenuItem::with_id("lock", "Lock", false, None);
        let quit = MenuItem::with_id("quit", format!("Quit {APP_NAME}"), true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &lock, &PredefinedMenuItem::separator(), &quit])
            .map_err(|error| error.to_string())?;
        // The handlers can be set once: showing the item again keeps them,
        // and the channel they send to.
        let clicks = events.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                clicks.unbounded_send(Event::Toggle(Click)).ok();
            }
        }));
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let event = match event.id().as_ref() {
                "open-vault" => Event::OpenVault,
                "lock" => Event::Lock,
                "quit" => Event::Quit,
                _ => return,
            };
            events.unbounded_send(event).ok();
        }));
        let icon = TrayIconBuilder::new()
            .with_icon_templated(image)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(APP_NAME)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { icon, lock })
    }

    pub fn shown(&self) -> bool {
        true
    }

    pub fn set_can_lock(&mut self, can_lock: bool, _: &mut App) {
        self.lock.set_enabled(can_lock);
    }

    /// Under the item, from where AppKit placed it.
    pub fn anchor(&self, _: Click, _: &mut App) -> Anchor {
        self.place().unwrap_or(Anchor::Centered)
    }

    /// The item and the display it is on, as AppKit counts on the whole
    /// desktop; `panel::under_item` turns them into the display's own
    /// coordinates, which GPUI places windows in.
    fn place(&self) -> Option<Anchor> {
        let mtm = MainThreadMarker::new()?;
        let window = self.icon.ns_status_item()?.button(mtm)?.window()?;
        let screen = window.screen()?;
        // GPUI's displays are CoreGraphics' display ids, which AppKit
        // gives each screen as its number.
        let number = screen
            .deviceDescription()
            .objectForKey(ns_string!("NSScreenNumber"))?;
        let display = DisplayId::new(number.downcast_ref::<NSNumber>()?.as_u32().into());
        Some(Anchor::Item {
            item: bounds(window.frame()),
            screen: bounds(screen.frame()),
            display,
        })
    }
}

fn bounds(rect: NSRect) -> Bounds<Pixels> {
    Bounds::new(
        point(px(rect.origin.x as f32), px(rect.origin.y as f32)),
        size(px(rect.size.width as f32), px(rect.size.height as f32)),
    )
}

/// The icon as a template image: black, its shape in the alpha.
fn template(svg: &[u8], cx: &App) -> Option<tray_icon::Icon> {
    let renderer = cx.svg_renderer();
    let parsed = renderer.parse_svg(svg).ok()?;
    let pixels = size(DevicePixels(ICON_PIXELS), DevicePixels(ICON_PIXELS));
    let image = renderer
        .render_parsed(&parsed, SvgSize::Size(pixels))
        .ok()?;
    let rgba = image
        .as_bytes(0)?
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| [0, 0, 0, pixel[3]])
        .collect();
    tray_icon::Icon::from_rgba(rgba, ICON_PIXELS as u32, ICON_PIXELS as u32).ok()
}
