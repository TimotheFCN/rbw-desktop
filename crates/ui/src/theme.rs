//! Theme tokens for the two looks the app ships, each in light and dark.
//!
//! The look follows the platform (macOS on a Mac, GNOME elsewhere) and the
//! appearance follows the system. `RBW_DESKTOP_THEME` overrides both, so every
//! combination can be screenshotted on Linux: `macos`, `gnome`, or either with
//! `-light` / `-dark`.

use gpui::{
    App, Background, Global, Hsla, Pixels, SharedString, Window, WindowAppearance, hsla,
    linear_color_stop, linear_gradient, px, rgb, rgba,
};

use crate::platform;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    MacOs,
    Gnome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl From<WindowAppearance> for Appearance {
    fn from(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::Dark,
        }
    }
}

/// What `RBW_DESKTOP_THEME` asked for. An unset appearance follows the system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Override {
    look: Look,
    appearance: Option<Appearance>,
}

impl Override {
    fn from_env() -> Option<Self> {
        let value = std::env::var("RBW_DESKTOP_THEME").ok()?;
        let (look, appearance) = match value.split_once('-') {
            Some((look, appearance)) => (look, Some(appearance)),
            None => (value.as_str(), None),
        };
        let look = match look {
            "macos" => Look::MacOs,
            "gnome" => Look::Gnome,
            _ => return None,
        };
        let appearance = match appearance {
            None => None,
            Some("light") => Some(Appearance::Light),
            Some("dark") => Some(Appearance::Dark),
            Some(_) => return None,
        };
        Some(Self { look, appearance })
    }
}

/// Sizes that differ between the two looks.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub text_size: Pixels,
    pub small_text_size: Pixels,
    pub control_height: Pixels,
    pub control_radius: Pixels,
    pub toolbar_height: Pixels,
    pub sidebar_width: Pixels,
    /// Gap between the floating sidebar and the window edge; zero when the
    /// sidebar is a flush pane.
    pub sidebar_inset: Pixels,
    pub sidebar_radius: Pixels,
    pub nav_row_height: Pixels,
    pub nav_row_radius: Pixels,
    pub list_width: Pixels,
    pub row_radius: Pixels,
    pub card_radius: Pixels,
    pub window_radius: Pixels,
    /// Raised controls carry a highlight and a soft shadow on macOS and are
    /// flat on GNOME.
    pub raised_controls: bool,
}

impl Look {
    pub const fn metrics(self) -> Metrics {
        match self {
            Look::MacOs => Metrics {
                text_size: px(13.),
                small_text_size: px(11.5),
                control_height: px(30.),
                control_radius: px(15.),
                toolbar_height: px(50.),
                sidebar_width: px(186.),
                sidebar_inset: px(7.),
                sidebar_radius: px(13.),
                nav_row_height: px(30.),
                nav_row_radius: px(8.),
                list_width: px(236.),
                row_radius: px(10.),
                card_radius: px(13.),
                window_radius: px(16.),
                raised_controls: true,
            },
            Look::Gnome => Metrics {
                text_size: px(13.5),
                small_text_size: px(12.),
                control_height: px(32.),
                control_radius: px(8.),
                toolbar_height: px(46.),
                sidebar_width: px(196.),
                sidebar_inset: px(0.),
                sidebar_radius: px(0.),
                nav_row_height: px(32.),
                nav_row_radius: px(6.),
                list_width: px(240.),
                row_radius: px(8.),
                card_radius: px(12.),
                window_radius: px(12.),
                raised_controls: false,
            },
        }
    }
}

/// Semantic colours. Names say what a colour is for, never what it looks like.
#[derive(Clone, Debug)]
pub struct Colors {
    /// The detail pane and the window behind everything else.
    pub window: Hsla,
    /// The item list pane.
    pub list: Hsla,
    /// The sidebar when no system material is drawn behind it.
    pub sidebar: Background,
    pub sidebar_edge: Hsla,
    pub card: Hsla,
    pub card_border: Option<Hsla>,
    pub separator: Hsla,
    pub text: Hsla,
    pub text_secondary: Hsla,
    pub text_tertiary: Hsla,
    /// Fills: selected list rows, primary buttons, switch tracks.
    pub accent: Hsla,
    /// Accent used as text or icon colour, which GNOME lightens in dark mode.
    pub accent_text: Hsla,
    pub on_accent: Hsla,
    /// Selected list rows. GNOME selects with a neutral fill, macOS with the accent.
    pub selection: Hsla,
    pub on_selection: Hsla,
    pub on_selection_secondary: Hsla,
    /// Neutral fill: selected sidebar rows, segmented-control and slider tracks.
    pub fill: Hsla,
    pub hover: Hsla,
    pub control: Hsla,
    pub control_edge: Hsla,
    pub control_highlight: Hsla,
    pub field: Hsla,
    pub segment_selected: Hsla,
    /// Sheets and popovers. Opaque: GPUI cannot blur what is behind them, and
    /// unblurred content showing through reads as clutter.
    pub frost: Hsla,
    /// Cards inside a sheet or popover.
    pub frost_card: Hsla,
    pub scrim: Hsla,
    pub shadow: Hsla,
    pub success: Hsla,
    /// Text and icons that warn, such as an error under a field.
    pub danger: Hsla,
    /// The fill of a button that removes something.
    pub destructive: Hsla,
    /// Behind a website's icon, which is drawn for a light background.
    pub icon_backdrop: Hsla,
    /// Symbols in a shown password; its digits take `accent_text`.
    pub symbol: Hsla,
    pub focus_ring: Hsla,
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn ca(hex: u32) -> Hsla {
    rgba(hex).into()
}

impl Colors {
    fn new(look: Look, appearance: Appearance, accent: Option<Hsla>) -> Self {
        let mut colors = match (look, appearance) {
            (Look::MacOs, Appearance::Light) => Self::macos_light(),
            (Look::MacOs, Appearance::Dark) => Self::macos_dark(),
            (Look::Gnome, Appearance::Light) => Self::gnome_light(),
            (Look::Gnome, Appearance::Dark) => Self::gnome_dark(),
        };
        if let Some(accent) = accent {
            colors.accent = accent;
            colors.accent_text = match (look, appearance) {
                (Look::Gnome, Appearance::Dark) => accent.blend(hsla(0., 0., 1., 0.35)),
                _ => accent,
            };
            if look == Look::MacOs {
                colors.selection = accent;
            }
        }
        colors.focus_ring = colors.accent.opacity(0.5);
        colors
    }

    fn macos_light() -> Self {
        let accent = c(0x007aff);
        Self {
            window: c(0xffffff),
            list: c(0xffffff),
            sidebar: linear_gradient(
                180.,
                linear_color_stop(c(0xeaf0fb), 0.),
                linear_color_stop(c(0xf7efec), 1.),
            ),
            sidebar_edge: ca(0xffffffe6),
            card: c(0xf3f3f6),
            card_border: None,
            separator: ca(0x00000014),
            text: c(0x1d1d1f),
            text_secondary: c(0x6e6e73),
            text_tertiary: c(0xa9a9af),
            accent,
            accent_text: accent,
            on_accent: c(0xffffff),
            selection: accent,
            on_selection: c(0xffffff),
            on_selection_secondary: ca(0xffffffd1),
            fill: ca(0x0000000e),
            hover: ca(0x00000009),
            control: ca(0xffffffdb),
            control_edge: ca(0xffffffe6),
            control_highlight: c(0xffffff),
            field: c(0xffffff),
            segment_selected: c(0xffffff),
            frost: c(0xf6f6f9),
            frost_card: ca(0xffffffb0),
            scrim: ca(0x0000002e),
            shadow: ca(0x1e285057),
            success: c(0x34c759),
            danger: c(0xff3b30),
            destructive: c(0xff3b30),
            symbol: c(0xe8590c),
            icon_backdrop: c(0xffffff),
            focus_ring: accent,
        }
    }

    fn macos_dark() -> Self {
        let accent = c(0x0a84ff);
        Self {
            window: c(0x1c1c1e),
            list: c(0x1c1c1e),
            sidebar: linear_gradient(
                180.,
                linear_color_stop(c(0x262a36), 0.),
                linear_color_stop(c(0x2c2729), 1.),
            ),
            sidebar_edge: ca(0xffffff21),
            card: c(0x2a2a2e),
            card_border: None,
            separator: ca(0xffffff17),
            text: c(0xf5f5f7),
            text_secondary: c(0xa1a1a8),
            text_tertiary: c(0x6b6b72),
            accent,
            accent_text: accent,
            on_accent: c(0xffffff),
            selection: accent,
            on_selection: c(0xffffff),
            on_selection_secondary: ca(0xffffffd1),
            fill: ca(0xffffff17),
            hover: ca(0xffffff0d),
            control: ca(0x48484ee6),
            control_edge: ca(0xffffff21),
            control_highlight: ca(0xffffff29),
            field: c(0x1c1c1e),
            segment_selected: c(0x5b5b62),
            frost: c(0x2a2a30),
            frost_card: ca(0xffffff12),
            scrim: ca(0x0000006b),
            shadow: ca(0x000000a6),
            success: c(0x30d158),
            danger: c(0xff453a),
            destructive: c(0xff453a),
            symbol: c(0xff9f0a),
            icon_backdrop: c(0xf2f2f5),
            focus_ring: accent,
        }
    }

    fn gnome_light() -> Self {
        let accent = c(0x3584e4);
        Self {
            window: c(0xfafafb),
            list: c(0xffffff),
            sidebar: c(0xebebed).into(),
            sidebar_edge: ca(0x0000001a),
            card: c(0xffffff),
            card_border: Some(ca(0x0000001a)),
            separator: ca(0x0000001a),
            text: c(0x1e1e20),
            text_secondary: c(0x5e5c64),
            text_tertiary: c(0x9a9996),
            accent,
            accent_text: c(0x1c71d8),
            on_accent: c(0xffffff),
            selection: ca(0x00000012),
            on_selection: c(0x1e1e20),
            on_selection_secondary: c(0x5e5c64),
            fill: ca(0x00000012),
            hover: ca(0x0000000a),
            control: ca(0x0000000f),
            control_edge: ca(0x00000000),
            control_highlight: ca(0x00000000),
            field: ca(0x0000000f),
            segment_selected: c(0xffffff),
            frost: c(0xfafafb),
            frost_card: c(0xffffff),
            scrim: ca(0x00000052),
            shadow: ca(0x00000066),
            success: c(0x26a269),
            danger: c(0xe01b24),
            destructive: c(0xe01b24),
            symbol: c(0xc64600),
            icon_backdrop: c(0xffffff),
            focus_ring: accent,
        }
    }

    fn gnome_dark() -> Self {
        let accent = c(0x3584e4);
        Self {
            window: c(0x222226),
            list: c(0x1d1d20),
            sidebar: c(0x2e2e32).into(),
            sidebar_edge: ca(0x00000059),
            card: ca(0xffffff14),
            card_border: Some(ca(0x00000059)),
            separator: ca(0xffffff1a),
            text: c(0xffffff),
            text_secondary: ca(0xffffffb3),
            text_tertiary: ca(0xffffff70),
            accent,
            accent_text: c(0x78aeed),
            on_accent: c(0xffffff),
            selection: ca(0xffffff1a),
            on_selection: c(0xffffff),
            on_selection_secondary: ca(0xffffffb3),
            fill: ca(0xffffff1a),
            hover: ca(0xffffff0f),
            control: ca(0xffffff1a),
            control_edge: ca(0x00000000),
            control_highlight: ca(0x00000000),
            field: ca(0xffffff1a),
            segment_selected: ca(0xffffff26),
            frost: c(0x36363a),
            frost_card: ca(0xffffff14),
            scrim: ca(0x00000080),
            shadow: ca(0x000000a0),
            success: c(0x78e9ab),
            danger: c(0xff7b63),
            destructive: c(0xc01c28),
            symbol: c(0xffa348),
            icon_backdrop: c(0xf0f0f2),
            focus_ring: accent,
        }
    }

    /// The two colours of an item's letter tile or an account's avatar,
    /// top-left to bottom-right.
    pub fn tile(&self, hue: f32) -> (Hsla, Hsla) {
        let h = hue / 360.;
        (hsla(h, 0.78, 0.64, 1.), hsla(h, 0.72, 0.48, 1.))
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub look: Look,
    pub appearance: Appearance,
    pub colors: Colors,
    pub metrics: Metrics,
    pub font_family: SharedString,
    pub mono_font_family: SharedString,
}

impl Global for Theme {}

/// Inputs the theme is computed from. Kept so a change to any one of them
/// recomputes the rest.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ThemeInputs {
    look: Look,
    appearance: Appearance,
    accent: Option<Hsla>,
}

impl Global for ThemeInputs {}

impl Theme {
    fn from_inputs(inputs: ThemeInputs, cx: &App) -> Self {
        let (font_family, mono_font_family) = platform::fonts(cx);
        Self {
            look: inputs.look,
            appearance: inputs.appearance,
            colors: Colors::new(inputs.look, inputs.appearance, inputs.accent),
            metrics: inputs.look.metrics(),
            font_family,
            mono_font_family,
        }
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Installs the theme for the current system state and keeps it in step
    /// with the system accent colour where the platform reports changes.
    pub(crate) fn init(cx: &mut App) {
        let theme_override = Override::from_env();
        let inputs = ThemeInputs {
            look: theme_override.map_or(platform::LOOK, |o| o.look),
            appearance: theme_override
                .and_then(|o| o.appearance)
                .unwrap_or_else(|| cx.window_appearance().into()),
            accent: theme_override
                .is_none()
                .then(platform::accent_color)
                .flatten(),
        };
        cx.set_global(inputs);
        cx.set_global(Self::from_inputs(inputs, cx));
        if theme_override.is_none() {
            platform::follow_accent_color(cx, |accent, cx| {
                Self::update_inputs(cx, |inputs| inputs.accent = Some(accent));
            });
        }
    }

    /// Follows the system appearance of `window`, unless an override pins it.
    /// Also re-reads the accent colour on platforms that report it
    /// synchronously, since changing it there changes the appearance too.
    pub fn follow_window_appearance(window: &mut Window, cx: &mut App) {
        let theme_override = Override::from_env();
        if theme_override.is_some_and(|o| o.appearance.is_some()) {
            return;
        }
        let appearance = window.appearance().into();
        let accent = theme_override
            .is_none()
            .then(platform::accent_color)
            .flatten();
        Self::update_inputs(cx, |inputs| {
            inputs.appearance = appearance;
            if accent.is_some() {
                inputs.accent = accent;
            }
        });
    }

    fn update_inputs(cx: &mut App, update: impl FnOnce(&mut ThemeInputs)) {
        let mut inputs = *cx.global::<ThemeInputs>();
        update(&mut inputs);
        if inputs != *cx.global::<ThemeInputs>() {
            cx.set_global(inputs);
            cx.set_global(Self::from_inputs(inputs, cx));
            crate::sync_base_theme(cx);
            cx.refresh_windows();
        }
    }
}

/// The colour a fraction `t` of the way from `from` to `to`, mixed in RGB so
/// that a fade between two hues does not pass through a third.
pub(crate) fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let (a, b) = (from.to_rgb(), to.to_rgb());
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    gpui::Rgba {
        r: lerp(a.r, b.r),
        g: lerp(a.g, b.g),
        b: lerp(a.b, b.b),
        a: lerp(a.a, b.a),
    }
    .into()
}

/// `cx.theme()` from any context that derefs to [`App`].
pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        Theme::global(self)
    }
}
