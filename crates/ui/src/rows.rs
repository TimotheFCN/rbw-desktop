//! Rows of the sidebar and the item list, and the letter tile that stands for
//! an item.

use gpui::base::Button as BaseButton;
use gpui::base::motion;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, InteractiveElement, IntoElement, ObjectFit,
    ParentElement, Pixels, RenderImage, RenderOnce, SharedString, Styled, StyledImage, Window, div,
    img, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px, relative,
};

use crate::card::copied_badge;
use crate::focus::{inner_ring, row_ring};
use crate::icon::{IconName, icon};
use crate::motion as m;
use crate::theme::{ActiveTheme, Look, mix};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// How far a row's selection highlight has come in, 0 to 1. Leaving and
/// arriving both fade, so moving the selection reads as one motion.
fn selection_progress(id: &ElementId, selected: bool, window: &mut Window, cx: &mut App) -> f32 {
    motion::transition(
        (id.clone(), "selection"),
        if selected { 1. } else { 0. },
        m::fade(m::QUICK),
        window,
        cx,
    )
}

/// A sidebar destination: an icon, a label, and the selection behind them.
#[derive(IntoElement)]
pub struct NavRow {
    id: ElementId,
    icon: IconName,
    label: SharedString,
    selected: bool,
    on_click: Option<ClickHandler>,
}

pub fn nav_row(
    id: impl Into<ElementId>,
    icon: IconName,
    label: impl Into<SharedString>,
    selected: bool,
) -> NavRow {
    NavRow {
        id: id.into(),
        icon,
        label: label.into(),
        selected,
        on_click: None,
    }
}

impl NavRow {
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for NavRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let progress = selection_progress(&self.id, self.selected, window, cx);
        let theme = cx.theme();
        let colors = &theme.colors;
        let metrics = theme.metrics;
        let icon_color = match theme.look {
            Look::MacOs => colors.accent_text,
            Look::Gnome => colors.text,
        };
        let fill = colors.fill;
        let hover = colors.hover;
        let ring = row_ring(colors.focus_ring);
        BaseButton::new(self.id)
            .selected(self.selected)
            .accessibility_label(self.label.clone())
            .when_some(self.on_click, |b, handler| b.on_click(handler))
            .focus_visible(ring)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(9.))
            .h(metrics.nav_row_height)
            .px(px(8.))
            .rounded(metrics.nav_row_radius)
            .bg(fill.opacity(progress))
            .when(!self.selected && theme.look == Look::Gnome, |b| {
                b.hover(move |s| s.bg(hover))
            })
            .when(self.selected, |b| b.font_weight(FontWeight::SEMIBOLD))
            .child(icon(self.icon).size(px(16.)).text_color(icon_color))
            .child(div().flex_1().min_w_0().truncate().child(self.label))
    }
}

/// A heading over a group of sidebar rows or popover rows.
pub fn section_label(label: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .flex_none()
        .px(px(8.))
        .pt(px(13.))
        .pb(px(4.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.colors.text_secondary)
        .child(label.into())
}

/// The coloured rounded square with an item's initial, or the website's
/// icon in its place once there is one.
#[derive(IntoElement)]
pub struct Tile {
    letter: SharedString,
    hue: f32,
    size: Pixels,
    icon: Option<(ElementId, Option<Arc<RenderImage>>)>,
}

/// A tile for `name`, with a hue derived from it so an item keeps its colour.
pub fn tile(name: &str) -> Tile {
    const HUES: [f32; 10] = [350., 32., 215., 240., 135., 0., 205., 145., 270., 190.];
    let letter = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "#".to_string(), |c| c.to_uppercase().collect());
    let hash = name.bytes().fold(0u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    Tile {
        letter: letter.into(),
        hue: HUES[hash as usize % HUES.len()],
        size: px(32.),
        icon: None,
    }
}

impl Tile {
    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    /// Shows a website's icon over the letter, fading in when it arrives.
    /// `id` names the tile, so the icon fades in only once.
    pub fn icon(mut self, id: impl Into<ElementId>, icon: Option<Arc<RenderImage>>) -> Self {
        self.icon = Some((id.into(), icon));
        self
    }
}

impl RenderOnce for Tile {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let icon = self.icon.map(|(id, icon)| {
            let shown = motion::transition(
                (id, "icon"),
                if icon.is_some() { 1. } else { 0. },
                m::fade(m::STANDARD),
                window,
                cx,
            );
            (icon, shown)
        });
        let theme = cx.theme();
        let (light, dark) = theme.colors.tile(self.hue);
        let radius = self.size
            * match theme.look {
                Look::MacOs => 0.28,
                Look::Gnome => 0.22,
            };
        let backdrop = theme.colors.icon_backdrop;
        let edge = theme.colors.separator;
        let inset = (self.size * 0.14).round();
        div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(self.size)
            .rounded(radius)
            .bg(linear_gradient(
                160.,
                linear_color_stop(light, 0.),
                linear_color_stop(dark, 1.),
            ))
            .text_color(gpui::white())
            .text_size(self.size * 0.44)
            .font_weight(FontWeight::SEMIBOLD)
            .child(self.letter)
            .when_some(icon, |d, (image, shown)| {
                d.when_some(image.filter(|_| shown > 0.), |d, image| {
                    d.child(
                        div()
                            .absolute()
                            .inset_0()
                            .p(inset)
                            .rounded(radius)
                            .bg(backdrop)
                            .border(px(0.5))
                            .border_color(edge)
                            .opacity(shown)
                            .child(img(image).size_full().object_fit(ObjectFit::Contain)),
                    )
                })
            })
    }
}

/// The round badge with an account's initial, in a colour of its own
/// derived from `name`, so that each account keeps its colour and two
/// accounts rarely share one.
pub fn avatar(name: &str, cx: &App) -> impl IntoElement {
    // Hues far enough apart to tell at a glance.
    const HUES: [f32; 8] = [215., 350., 145., 275., 28., 185., 320., 48.];
    let initial: String = name.chars().take(1).flat_map(char::to_uppercase).collect();
    // FNV-1a: stable across runs and builds, unlike the standard hasher.
    let hash = name
        .to_lowercase()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    let (light, dark) = cx
        .theme()
        .colors
        .tile(HUES[(hash % HUES.len() as u64) as usize]);
    div()
        .size(px(26.))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(linear_gradient(
            160.,
            linear_color_stop(light, 0.),
            linear_color_stop(dark, 1.),
        ))
        .text_color(gpui::white())
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(initial)
}

/// The app's own mark, a lock on a blue square, that opens the lock screen
/// and the sign-in steps.
pub fn app_icon(cx: &App) -> impl IntoElement {
    let size = px(62.);
    let radius = match cx.theme().look {
        Look::MacOs => 0.28,
        Look::Gnome => 0.22,
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(size)
        .rounded(size * radius)
        .bg(linear_gradient(
            160.,
            linear_color_stop(gpui::rgb(0x5aa9ff), 0.),
            linear_color_stop(gpui::rgb(0x0a63e0), 1.),
        ))
        .shadow(vec![
            gpui::BoxShadow {
                color: gpui::white().opacity(0.5),
                offset: gpui::point(px(0.), px(1.)),
                blur_radius: px(0.),
                spread_radius: px(0.),
                inset: true,
            },
            gpui::BoxShadow::new(px(0.), px(8.), gpui::rgba(0x005adc8c).into())
                .blur_radius(px(18.))
                .spread_radius(px(-6.)),
        ])
        .text_color(gpui::white())
        .child(icon(IconName::Lock).size(px(28.)))
}

/// A row of the item list: tile, name and subtitle.
#[derive(IntoElement)]
pub struct ListRow {
    id: ElementId,
    name: SharedString,
    subtitle: Option<SharedString>,
    selected: bool,
    icon: Option<Arc<RenderImage>>,
    trailing: Option<AnyElement>,
    copied: bool,
    compact: bool,
    /// Whether the row is a stop of Tab, rather than reached with the
    /// arrow keys through its list.
    tab_stop: bool,
    /// Whether its list has the keyboard, which it shows for it.
    focused: bool,
    accessibility_label: Option<SharedString>,
    on_click: Option<ClickHandler>,
}

const ROW_HEIGHT: Pixels = px(46.);
const COMPACT_ROW_HEIGHT: Pixels = px(40.);
/// Text in rows reads at the mockups' line height. gpui-base's buttons set
/// it to the font size, which presses a name onto its subtitle.
pub(crate) const ROW_LINE_HEIGHT: f32 = 1.35;

pub fn list_row(
    id: impl Into<ElementId>,
    name: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    selected: bool,
) -> ListRow {
    ListRow {
        id: id.into(),
        name: name.into(),
        subtitle,
        selected,
        icon: None,
        trailing: None,
        copied: false,
        compact: false,
        tab_stop: true,
        focused: false,
        accessibility_label: None,
        on_click: None,
    }
}

impl ListRow {
    /// A row of a panel or popover, as in the menu bar panel: a smaller
    /// tile, and a neutral highlight in place of the accent, as a menu's.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// The website's icon to show in the tile, once it has arrived.
    pub fn icon(mut self, icon: Option<Arc<RenderImage>>) -> Self {
        self.icon = icon;
        self
    }

    /// What assistive technologies call the row, when its name alone
    /// doesn't say what a click does.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
        self
    }

    /// A row of a list that takes the keyboard as a whole and moves its
    /// selection with the arrow keys: Tab passes it by. `focused` says the
    /// list has the keyboard, which the selected row shows with the ring.
    pub fn in_focusable_list(mut self, focused: bool) -> Self {
        self.tab_stop = false;
        self.focused = focused;
        self
    }

    /// Something of the row's item was just copied: a short "Copied" takes
    /// the place of the trailing element.
    pub fn copied(mut self, copied: bool) -> Self {
        self.copied = copied;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for ListRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let progress = selection_progress(&self.id, self.selected, window, cx);
        let confirmation = m::presence((self.id.clone(), "copied"), self.copied, window, cx);
        let trailing = if confirmation.should_render() {
            let badge = copied_badge(cx).opacity(confirmation.progress);
            // On the accent of a selected row, in the row's own colour.
            Some(
                badge
                    .when(self.selected && !self.compact, |b| {
                        b.text_color(cx.theme().colors.on_selection)
                    })
                    .into_any_element(),
            )
        } else {
            self.trailing
        };
        let theme = cx.theme();
        let colors = &theme.colors;
        let hover = colors.hover;
        // On the accent of a selected row, the ring takes the row's text
        // colour.
        let ring_color = if self.selected && !self.compact && theme.look == Look::MacOs {
            colors.on_selection.opacity(0.7)
        } else {
            colors.focus_ring
        };
        let list_focused = self.focused && window.last_input_was_keyboard();
        let (selection, name_color, subtitle_color) = if self.compact {
            (colors.fill, colors.text, colors.text_secondary)
        } else {
            (
                colors.selection,
                mix(colors.text, colors.on_selection, progress),
                mix(
                    colors.text_secondary,
                    colors.on_selection_secondary,
                    progress,
                ),
            )
        };
        let name = self.name;
        let tile = tile(&name)
            .size(if self.compact { px(28.) } else { px(32.) })
            .icon((self.id.clone(), "tile"), self.icon);
        BaseButton::new(self.id)
            .selected(self.selected)
            .accessibility_label(self.accessibility_label.unwrap_or_else(|| name.clone()))
            .when_some(self.on_click, |b, handler| b.on_click(handler))
            .tab_stop(self.tab_stop)
            .focus_visible(row_ring(ring_color))
            .when(list_focused && self.selected, |b| {
                b.shadow(inner_ring(ring_color))
            })
            .flex()
            .items_center()
            .gap(px(10.))
            .h(if self.compact {
                COMPACT_ROW_HEIGHT
            } else {
                ROW_HEIGHT
            })
            .px(if self.compact { px(7.) } else { px(9.) })
            .rounded(theme.metrics.row_radius)
            .bg(selection.opacity(progress))
            .when(!self.selected, |b| b.hover(move |s| s.bg(hover)))
            .child(tile)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .line_height(relative(ROW_LINE_HEIGHT))
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(name_color)
                            .child(name),
                    )
                    .when_some(self.subtitle, |d, subtitle| {
                        d.child(
                            div()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(subtitle_color)
                                .child(subtitle),
                        )
                    }),
            )
            .children(trailing)
    }
}
