//! What the design system reads from the operating system: the default look,
//! UI fonts and the accent colour.

use gpui::{App, Hsla, SharedString};

use crate::theme::Look;

#[cfg(target_os = "macos")]
pub const LOOK: Look = Look::MacOs;
#[cfg(not(target_os = "macos"))]
pub const LOOK: Look = Look::Gnome;

/// The UI and monospace font families. A Mac uses its system font.
///
/// Elsewhere the UI font is the bundled Inter. GNOME's own Adwaita Sans is
/// derived from Inter, but it ships as a variable font, and GPUI's Linux text
/// system draws variable fonts at their default weight only: no medium, no
/// bold. Static Inter draws the same letters with working weights. Inter also
/// stands in for San Francisco when the macOS look is previewed.
pub fn fonts(cx: &App) -> (SharedString, SharedString) {
    if cfg!(target_os = "macos") {
        return (".SystemUIFont".into(), "Menlo".into());
    }
    let installed = cx.text_system().all_font_names();
    let mono = ["Adwaita Mono", "DejaVu Sans Mono", "Liberation Mono"]
        .into_iter()
        .find(|name| installed.iter().any(|font| font == name))
        .unwrap_or("monospace");
    ("Inter".into(), mono.into())
}

/// Registers the fonts the app bundles.
#[cfg(not(target_os = "macos"))]
pub fn load_fonts(cx: &App) {
    let fonts = [
        include_bytes!("../assets/fonts/Inter-Regular.otf").as_slice(),
        include_bytes!("../assets/fonts/Inter-Medium.otf"),
        include_bytes!("../assets/fonts/Inter-SemiBold.otf"),
        include_bytes!("../assets/fonts/Inter-Bold.otf"),
    ];
    if let Err(error) = cx
        .text_system()
        .add_fonts(fonts.into_iter().map(std::borrow::Cow::Borrowed).collect())
    {
        log::error!("loading the bundled fonts: {error:#}");
    }
}

#[cfg(target_os = "macos")]
pub fn accent_color() -> Option<Hsla> {
    use objc2_app_kit::{NSColor, NSColorSpace};

    let color =
        NSColor::controlAccentColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    Some(
        gpui::Rgba {
            r: color.redComponent() as f32,
            g: color.greenComponent() as f32,
            b: color.blueComponent() as f32,
            a: 1.,
        }
        .into(),
    )
}

/// The accent arrives from the desktop portal asynchronously, through
/// [`follow_accent_color`].
#[cfg(not(target_os = "macos"))]
pub fn accent_color() -> Option<Hsla> {
    None
}

/// Calls `apply` with the portal's accent colour now and whenever it changes.
#[cfg(target_os = "linux")]
pub fn follow_accent_color(cx: &mut App, apply: fn(Hsla, &mut App)) {
    use ashpd::desktop::settings::Settings;
    use futures::StreamExt as _;

    let to_hsla = |color: ashpd::desktop::Color| -> Option<Hsla> {
        let channels = [color.red(), color.green(), color.blue()];
        // The portal reports out-of-range channels when no accent is set.
        channels.iter().all(|c| (0.0..=1.0).contains(c)).then(|| {
            gpui::Rgba {
                r: channels[0] as f32,
                g: channels[1] as f32,
                b: channels[2] as f32,
                a: 1.,
            }
            .into()
        })
    };
    cx.spawn(async move |cx| {
        let settings = Settings::new().await.ok()?;
        if let Some(accent) = settings.accent_color().await.ok().and_then(to_hsla) {
            cx.update(|cx| apply(accent, cx));
        }
        let mut changes = settings.receive_accent_color_changed().await.ok()?;
        while let Some(color) = changes.next().await {
            if let Some(accent) = to_hsla(color) {
                cx.update(|cx| apply(accent, cx));
            }
        }
        Some(())
    })
    .detach();
}

/// macOS reports the accent colour synchronously; changes are picked up on the
/// next appearance change.
#[cfg(not(target_os = "linux"))]
pub fn follow_accent_color(_cx: &mut App, _apply: fn(Hsla, &mut App)) {}
