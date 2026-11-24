//! Window chrome. On macOS the system draws the traffic lights inside the
//! sidebar over a transparent title bar. On Linux the app draws its own
//! frame, GNOME-style: rounded corners, a hairline edge, a shadow it can be
//! resized from, header bars to drag it by, and the window buttons the desktop
//! asks for.

use gpui::{
    AnyElement, App, Bounds, CursorStyle, Decorations, Div, ElementId, HitboxBehavior,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point, ResizeEdge, Size,
    Stateful, StatefulInteractiveElement, Styled, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowButton, WindowDecorations, WindowOptions,
    canvas, div, point, prelude::FluentBuilder as _, px,
};

use crate::button::{ButtonStyle, button};
use crate::icon::IconName;
use crate::theme::{ActiveTheme, Look};

/// Where the macOS traffic lights sit, from the window's top-left corner.
/// They line up with the floating sidebar's first row.
const TRAFFIC_LIGHTS: Point<Pixels> = point(px(21.), px(21.));
/// Room the traffic lights take at the top of the sidebar.
pub const TRAFFIC_LIGHTS_HEIGHT: Pixels = px(42.);
/// The radius of the window's corners where the app draws them, and zero
/// where it doesn't. GPUI clips to rectangles only, so a pane that paints its
/// own background at a corner of the window rounds that corner itself.
pub fn corner_radius(window: &Window, cx: &App) -> Pixels {
    match window.window_decorations() {
        Decorations::Client { tiling } if !cfg!(target_os = "macos") && !tiling.is_tiled() => {
            cx.theme().metrics.window_radius
        }
        _ => px(0.),
    }
}

/// The invisible border of a client-side frame that resizes the window and
/// holds its shadow.
const FRAME_INSET: Pixels = px(12.);

/// Options for an app window of `size`, with the platform's chrome.
pub fn window_options(
    title: &str,
    size: Size<Pixels>,
    min: Size<Pixels>,
    cx: &App,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        window_min_size: Some(min),
        app_id: Some("rbw-desktop".into()),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_string().into()),
            appears_transparent: true,
            traffic_light_position: Some(TRAFFIC_LIGHTS),
        }),
        // macOS: the glass sidebar floats on the window's own blurred
        // material, which shows around it. Linux: the drawn frame's rounded
        // corners and shadow need a see-through window.
        window_background: if cfg!(target_os = "macos") {
            WindowBackgroundAppearance::Blurred
        } else {
            WindowBackgroundAppearance::Transparent
        },
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

/// Options for a small panel of `size`, such as the menu bar panel, its
/// top-left corner at `origin` on `display` or, without one, centred. On
/// macOS it is a pop-up over every other window, on the window's blurred
/// material, without a title bar once
/// [`Window::set_native_titlebar_visibility`] hides it; elsewhere it is an
/// ordinary window without a header bar, its drawn frame added around
/// `size`.
pub fn panel_options(
    title: &str,
    size: Size<Pixels>,
    origin: Option<(Point<Pixels>, Option<gpui::DisplayId>)>,
    cx: &App,
) -> WindowOptions {
    let macos = cfg!(target_os = "macos");
    let (size, origin) = if macos {
        (size, origin)
    } else {
        let frame = point(FRAME_INSET, FRAME_INSET);
        (
            gpui::size(
                size.width + FRAME_INSET * 2.,
                size.height + FRAME_INSET * 2.,
            ),
            origin.map(|(origin, display)| (origin - frame, display)),
        )
    };
    let display = origin.and_then(|(_, display)| display);
    let bounds = match origin {
        Some((origin, _)) => Bounds::new(origin, size),
        None => Bounds::centered(display, size, cx),
    };
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id: display,
        titlebar: (!macos).then(|| TitlebarOptions {
            title: Some(title.to_string().into()),
            ..Default::default()
        }),
        kind: if macos {
            gpui::WindowKind::PopUp
        } else {
            gpui::WindowKind::Normal
        },
        is_movable: !macos,
        is_resizable: false,
        is_minimizable: false,
        app_id: Some("rbw-desktop".into()),
        window_background: if macos {
            WindowBackgroundAppearance::Blurred
        } else {
            WindowBackgroundAppearance::Transparent
        },
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

/// Frames the whole window's content. Draws the client-side frame where the
/// app decorates its own window; elsewhere it only fills the window.
pub fn window_frame(content: impl IntoElement, window: &mut Window, cx: &mut App) -> AnyElement {
    let decorations = window.window_decorations();
    let theme = cx.theme();
    let content = div()
        .size_full()
        .flex()
        .overflow_hidden()
        // On macOS the window stays see-through for the sidebar's glass.
        .when(!cfg!(target_os = "macos"), |content| {
            content.bg(theme.colors.window)
        })
        .text_size(theme.metrics.text_size)
        .text_color(theme.colors.text)
        .font_family(theme.font_family.clone())
        .child(content);

    let Decorations::Client { tiling } = decorations else {
        window.set_client_inset(px(0.));
        return content.into_any_element();
    };
    if cfg!(target_os = "macos") {
        return content.into_any_element();
    }
    let floating = !tiling.is_tiled();
    window.set_client_inset(if floating { FRAME_INSET } else { px(0.) });
    let radius = theme.metrics.window_radius;
    let colors = &theme.colors;
    div()
        .id("window-frame")
        .size_full()
        .when(floating, |frame| {
            frame.p(FRAME_INSET).child(resize_handles()).on_mouse_down(
                MouseButton::Left,
                |event, window, _| {
                    let size = window.window_bounds().get_bounds().size;
                    if let Some(edge) = resize_edge(event.position, size) {
                        window.start_window_resize(edge);
                    }
                },
            )
        })
        .child(
            content
                .when(floating, |content| {
                    content
                        .rounded(radius)
                        .border_1()
                        .border_color(colors.sidebar_edge)
                        .shadow(vec![
                            gpui::BoxShadow::new(px(0.), px(3.), colors.shadow.opacity(0.5))
                                .blur_radius(FRAME_INSET - px(3.)),
                        ])
                })
                .on_mouse_move(|_, _, cx| cx.stop_propagation()),
        )
        .into_any_element()
}

fn resize_handles() -> impl IntoElement {
    canvas(
        |_, window, _| {
            window.insert_hitbox(
                Bounds::new(
                    point(px(0.), px(0.)),
                    window.window_bounds().get_bounds().size,
                ),
                HitboxBehavior::Normal,
            )
        },
        |_, hitbox, window, _| {
            let size = window.window_bounds().get_bounds().size;
            let Some(edge) = resize_edge(window.mouse_position(), size) else {
                return;
            };
            window.set_cursor_style(
                match edge {
                    ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                    ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
                    ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                        CursorStyle::ResizeUpLeftDownRight
                    }
                    ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                        CursorStyle::ResizeUpRightDownLeft
                    }
                },
                &hitbox,
            );
        },
    )
    .absolute()
    .size_full()
}

fn resize_edge(position: Point<Pixels>, window: Size<Pixels>) -> Option<ResizeEdge> {
    let corner = FRAME_INSET * 2.;
    let top = position.y < FRAME_INSET;
    let bottom = position.y > window.height - FRAME_INSET;
    let left = position.x < FRAME_INSET;
    let right = position.x > window.width - FRAME_INSET;
    let near_top = position.y < corner;
    let near_bottom = position.y > window.height - corner;
    let near_left = position.x < corner;
    let near_right = position.x > window.width - corner;
    Some(match () {
        _ if (top && near_left) || (left && near_top) => ResizeEdge::TopLeft,
        _ if (top && near_right) || (right && near_top) => ResizeEdge::TopRight,
        _ if (bottom && near_left) || (left && near_bottom) => ResizeEdge::BottomLeft,
        _ if (bottom && near_right) || (right && near_bottom) => ResizeEdge::BottomRight,
        _ if top => ResizeEdge::Top,
        _ if bottom => ResizeEdge::Bottom,
        _ if left => ResizeEdge::Left,
        _ if right => ResizeEdge::Right,
        _ => return None,
    })
}

/// Makes `element`, a header bar or toolbar, move the window when dragged,
/// maximize it on double-click and open the window menu on right-click, where
/// the app draws its own chrome. The move starts once the pointer moves with
/// the button down, so a click on a control inside the bar stays a click.
pub fn drag_region(
    id: impl Into<ElementId>,
    element: Div,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let id = id.into();
    let element = element.id(id.clone());
    if cfg!(target_os = "macos") {
        return element;
    }
    let pressed = window.use_keyed_state((id, "drag"), cx, |_, _| false);
    let (down, moved, up) = (pressed.clone(), pressed.clone(), pressed);
    element
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            down.update(cx, |pressed, _| *pressed = true);
        })
        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
            up.update(cx, |pressed, _| *pressed = false);
        })
        .on_mouse_move(move |event, window, cx| {
            if *moved.read(cx) && event.dragging() {
                moved.update(cx, |pressed, _| *pressed = false);
                window.start_window_move();
            }
        })
        .on_click(|event, window, _| {
            if event.click_count() == 2 {
                window.zoom_window();
            }
        })
        .on_mouse_down(MouseButton::Right, |event, window, _| {
            window.show_window_menu(event.position);
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Start,
    End,
}

/// The window buttons the desktop puts on one side of the title bar, drawn by
/// the app on Linux. `None` on macOS and when that side has none. On GNOME
/// the default is a single close button at the end.
pub fn window_buttons(side: Side, window: &Window, cx: &App) -> Option<AnyElement> {
    // A previewed macOS look shows stand-ins for the traffic lights instead.
    if cfg!(target_os = "macos")
        || cx.theme().look == Look::MacOs
        || window.window_decorations() == Decorations::Server
    {
        return None;
    }
    let buttons: Vec<WindowButton> = match cx.button_layout() {
        Some(layout) => match side {
            Side::Start => layout.left.iter().flatten().copied().collect(),
            Side::End => layout.right.iter().flatten().copied().collect(),
        },
        None if side == Side::End => vec![WindowButton::Close],
        None => vec![],
    };
    if buttons.is_empty() {
        return None;
    }
    let colors = &cx.theme().colors;
    let fill = colors.fill;
    Some(
        div()
            .flex()
            .gap(px(12.))
            .px(px(4.))
            .children(buttons.into_iter().map(|which| {
                let (icon, label) = match which {
                    WindowButton::Minimize => (IconName::Minimize, "Minimize"),
                    WindowButton::Maximize => (IconName::Maximize, "Maximize"),
                    WindowButton::Close => (IconName::Close, "Close"),
                };
                div().rounded_full().bg(fill).child(
                    button(which.id())
                        .icon(icon)
                        .style(ButtonStyle::Flat)
                        .size(px(24.))
                        .accessibility_label(label)
                        .on_click(move |_, window, _| match which {
                            WindowButton::Minimize => window.minimize_window(),
                            WindowButton::Maximize => window.zoom_window(),
                            WindowButton::Close => window.remove_window(),
                        }),
                )
            }))
            .into_any_element(),
    )
}

/// Room for the traffic lights at the top of the macOS sidebar. When the
/// macOS look is previewed on another system, stand-ins show where the
/// system would draw them.
pub fn traffic_lights(cx: &App) -> Option<AnyElement> {
    if cx.theme().look != Look::MacOs {
        return None;
    }
    let lights = div()
        .h(TRAFFIC_LIGHTS_HEIGHT)
        .flex()
        .items_center()
        .px(px(7.));
    if cfg!(target_os = "macos") {
        return Some(lights.into_any_element());
    }
    Some(
        lights
            .gap(px(8.))
            .children(
                [0xff5f57, 0xfebc2e, 0x28c840]
                    .map(|color| div().size(px(12.)).rounded_full().bg(gpui::rgb(color))),
            )
            .into_any_element(),
    )
}
