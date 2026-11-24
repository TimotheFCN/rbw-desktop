//! Rows of the sidebar and the item list, and the letter tile that stands for
//! an item.

use gpui::base::Button as BaseButton;
use gpui::base::motion;
use gpui::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Pixels, RenderOnce, SharedString, Styled, Window, div, linear_color_stop,
    linear_gradient, prelude::FluentBuilder as _, px,
};

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
        BaseButton::new(self.id)
            .selected(self.selected)
            .accessibility_label(self.label.clone())
            .when_some(self.on_click, |b, handler| b.on_click(handler))
            .flex()
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
        .px(px(8.))
        .pt(px(13.))
        .pb(px(4.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.colors.text_secondary)
        .child(label.into())
}

/// The coloured rounded square with an item's initial.
#[derive(IntoElement)]
pub struct Tile {
    letter: SharedString,
    hue: f32,
    size: Pixels,
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
    }
}

impl Tile {
    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for Tile {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let (light, dark) = theme.colors.tile(self.hue);
        let radius = match theme.look {
            Look::MacOs => 0.28,
            Look::Gnome => 0.22,
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(self.size)
            .rounded(self.size * radius)
            .bg(linear_gradient(
                160.,
                linear_color_stop(light, 0.),
                linear_color_stop(dark, 1.),
            ))
            .text_color(gpui::white())
            .text_size(self.size * 0.44)
            .font_weight(FontWeight::SEMIBOLD)
            .child(self.letter)
    }
}

/// The round badge with an account's initial.
pub fn avatar(email: &str, cx: &App) -> impl IntoElement {
    let initial: String = email.chars().take(1).flat_map(char::to_uppercase).collect();
    div()
        .size(px(26.))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(cx.theme().colors.avatar)
        .text_color(gpui::white())
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(initial)
}

/// A row of the item list: tile, name and subtitle.
#[derive(IntoElement)]
pub struct ListRow {
    id: ElementId,
    name: SharedString,
    subtitle: Option<SharedString>,
    selected: bool,
    trailing: Option<AnyElement>,
    on_click: Option<ClickHandler>,
}

const ROW_HEIGHT: Pixels = px(44.);

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
        trailing: None,
        on_click: None,
    }
}

impl ListRow {
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
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
        let theme = cx.theme();
        let colors = &theme.colors;
        let selection = colors.selection;
        let hover = colors.hover;
        let name_color = mix(colors.text, colors.on_selection, progress);
        let subtitle_color = mix(
            colors.text_secondary,
            colors.on_selection_secondary,
            progress,
        );
        let name = self.name;
        BaseButton::new(self.id)
            .selected(self.selected)
            .accessibility_label(name.clone())
            .when_some(self.on_click, |b, handler| b.on_click(handler))
            .flex()
            .items_center()
            .gap(px(10.))
            .h(ROW_HEIGHT)
            .px(px(9.))
            .rounded(theme.metrics.row_radius)
            .bg(selection.opacity(progress))
            .when(!self.selected, |b| b.hover(move |s| s.bg(hover)))
            .child(tile(&name))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
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
            .children(self.trailing)
    }
}
