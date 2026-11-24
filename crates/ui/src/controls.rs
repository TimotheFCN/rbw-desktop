//! Choice controls: the segmented control, the switch and the slider.

use gpui::base::{
    Button as BaseButton, Slider as BaseSlider, SliderIndicator, SliderThumb, SliderTrack,
    Switch as BaseSwitch, motion, slider::SliderState,
};
use gpui::{
    App, BoxShadow, ClickEvent, ElementId, Entity, FocusHandle, FontWeight, InteractiveElement,
    IntoElement, ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled,
    Window, div, prelude::FluentBuilder as _, px, relative, white,
};

use crate::focus::control_ring;
use crate::icon::IconName;
use crate::motion as m;
use crate::rows::tile;
use crate::theme::{ActiveTheme, Look};

type SelectHandler = Box<dyn Fn(usize, &mut Window, &mut App)>;

/// A row of mutually exclusive choices, with the selection sliding between
/// them.
#[derive(IntoElement)]
pub struct SegmentedControl {
    id: ElementId,
    labels: Vec<SharedString>,
    /// A glyph over each label, on a tile of its hue.
    glyphs: Vec<(IconName, f32)>,
    selected: usize,
    on_select: Option<SelectHandler>,
}

pub fn segmented_control(
    id: impl Into<ElementId>,
    labels: impl IntoIterator<Item = impl Into<SharedString>>,
    selected: usize,
) -> SegmentedControl {
    SegmentedControl {
        id: id.into(),
        labels: labels.into_iter().map(Into::into).collect(),
        glyphs: vec![],
        selected,
        on_select: None,
    }
}

impl SegmentedControl {
    /// A coloured glyph over each label, such as the kinds of item to make:
    /// larger segments that read as a choice of things.
    pub fn glyphs(mut self, glyphs: impl IntoIterator<Item = (IconName, f32)>) -> Self {
        self.glyphs = glyphs.into_iter().collect();
        self
    }

    pub fn on_select(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for SegmentedControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let count = self.labels.len().max(1) as f32;
        let position = motion::spring(
            (self.id.clone(), "thumb"),
            self.selected as f32,
            m::SELECTION.with_epsilon(0.002),
            window,
            cx,
        );
        // Each segment's own focus, so the arrow keys can move it.
        let focus: Vec<FocusHandle> = (0..self.labels.len())
            .map(|index| {
                window
                    .use_keyed_state((self.id.clone(), format!("focus-{index}")), cx, |_, cx| {
                        cx.focus_handle()
                    })
                    .read(cx)
                    .clone()
            })
            .collect();
        let theme = cx.theme();
        let colors = &theme.colors;
        let macos = theme.look == Look::MacOs;
        let ring = colors.focus_ring;
        let on_select = self.on_select.map(std::rc::Rc::new);
        let id = self.id.clone();
        let (selected, last) = (self.selected, self.labels.len().saturating_sub(1));
        let on_arrow = on_select.clone();
        let arrow_focus = focus.clone();

        div()
            .id(self.id)
            .role(gpui::Role::RadioGroup)
            // The arrow keys move the choice, and the keyboard with it, as
            // in a group of radio buttons.
            .on_key_down(move |event, window, cx| {
                let next = match event.keystroke.key.as_str() {
                    "left" if selected > 0 => selected - 1,
                    "right" if selected < last => selected + 1,
                    _ => return,
                };
                cx.stop_propagation();
                window.focus(&arrow_focus[next], cx);
                if let Some(on_select) = &on_arrow {
                    on_select(next, window, cx);
                }
            })
            .flex()
            .p(px(2.))
            .rounded(if macos { px(9.) } else { px(8.) })
            .bg(colors.fill)
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(relative(position / count))
                            .w(relative(1. / count))
                            .rounded(if macos { px(7.) } else { px(6.) })
                            .bg(colors.segment_selected)
                            .shadow(vec![
                                BoxShadow::new(px(0.), px(1.), gpui::black().opacity(0.16))
                                    .blur_radius(px(3.)),
                            ]),
                    )
                    .children(self.labels.into_iter().enumerate().map(|(index, label)| {
                        let selected = index == self.selected;
                        let glyph = self.glyphs.get(index).copied();
                        let on_select = on_select.clone();
                        BaseButton::new((id.clone(), index.to_string()))
                            .role(gpui::Role::RadioButton)
                            .accessibility_label(label.clone())
                            .selected(selected)
                            // One stop of Tab, at the choice; the arrow keys
                            // move it.
                            .track_focus(&focus[index])
                            .tab_stop(selected)
                            .relative()
                            .flex_1()
                            .py(px(2.5))
                            .when(glyph.is_some(), |d| d.flex_col().gap(px(5.)).py(px(8.)))
                            .rounded(if macos { px(7.) } else { px(6.) })
                            .text_size(px(12.5))
                            .line_height(relative(1.3))
                            // A shadow would show through the clear segment
                            // as a fill: the ring is a border, always there
                            // and clear until the keyboard reaches it.
                            .border_2()
                            .border_color(gpui::transparent_black())
                            .focus_visible(move |s| s.border_color(ring))
                            .when(selected, |d| d.font_weight(FontWeight::SEMIBOLD))
                            .when_some(on_select, |d, on_select| {
                                d.on_click(move |_: &ClickEvent, window, cx| {
                                    on_select(index, window, cx)
                                })
                            })
                            .when_some(glyph, |d, (glyph, hue)| {
                                d.child(tile("").glyph(glyph, hue).size(px(28.)))
                            })
                            .child(label)
                    })),
            )
    }
}

type ToggleHandler = Box<dyn Fn(bool, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    on: bool,
    label: SharedString,
    on_change: Option<ToggleHandler>,
}

/// A switch. `label` names it for assistive technologies; the visible label
/// is the row it sits in.
pub fn switch(id: impl Into<ElementId>, on: bool, label: impl Into<SharedString>) -> Switch {
    Switch {
        id: id.into(),
        on,
        label: label.into(),
        on_change: None,
    }
}

impl Switch {
    pub fn on_change(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let travel = motion::spring(
            (self.id.clone(), "thumb"),
            if self.on { 1. } else { 0. },
            m::SELECTION.with_epsilon(0.002),
            window,
            cx,
        );
        let ring = control_ring(cx);
        let colors = &cx.theme().colors;
        let track = colors
            .fill
            .blend(colors.accent.opacity(travel.clamp(0., 1.)));
        let (width, height, thumb, inset) = (px(36.), px(21.), px(17.), px(2.));

        BaseSwitch::new(self.id)
            .checked(self.on)
            .accessibility_label(self.label)
            .when_some(self.on_change, |s, handler| {
                s.on_change(move |on, _, window, cx| handler(on, window, cx))
            })
            .relative()
            .flex_none()
            .w(width)
            .h(height)
            .rounded_full()
            .bg(track)
            .focus_visible(ring)
            .child(
                div()
                    .absolute()
                    .top(inset)
                    .left(inset + (width - thumb - inset * 2.) * travel)
                    .size(thumb)
                    .rounded_full()
                    .bg(white())
                    .shadow(vec![
                        BoxShadow::new(px(0.), px(1.), gpui::black().opacity(0.25))
                            .blur_radius(px(3.)),
                    ]),
            )
    }
}

/// A horizontal slider over a [`SliderState`] the caller owns and observes.
#[derive(IntoElement)]
pub struct Slider {
    state: Entity<SliderState>,
    label: SharedString,
    focus: Option<FocusHandle>,
}

pub fn slider(state: &Entity<SliderState>, label: impl Into<SharedString>) -> Slider {
    Slider {
        state: state.clone(),
        label: label.into(),
        focus: None,
    }
}

impl Slider {
    /// Lets the keyboard reach the slider with `focus`: the arrow keys then
    /// move it a step, Home and End to its ends.
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }
}

impl RenderOnce for Slider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = &cx.theme().colors;
        let value = self.state.read(cx).percentage().end;
        let ring = colors.focus_ring;
        let focus_visible = self
            .focus
            .as_ref()
            .is_some_and(|focus| focus.is_focused(window) && window.last_input_was_keyboard());
        BaseSlider::new(&self.state)
            .accessibility_label(self.label)
            .when_some(self.focus, |slider, focus| slider.track_focus(&focus))
            .flex_1()
            .child(
                SliderTrack::new(&self.state)
                    .flex()
                    .items_center()
                    .h(px(20.))
                    .w_full()
                    .child(
                        // The indicator's bounds are what values map onto; the
                        // fill and the thumb sit inside it at the value.
                        SliderIndicator::new(&self.state)
                            .relative()
                            .w_full()
                            .h(px(4.))
                            .rounded(px(2.))
                            .bg(colors.fill)
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .h_full()
                                    .w(relative(value))
                                    .rounded(px(2.))
                                    .bg(colors.accent),
                            )
                            .child(
                                SliderThumb::new(&self.state)
                                    .absolute()
                                    .top(px(-8.))
                                    .left(relative(value))
                                    .ml(px(-10.))
                                    .size(px(20.))
                                    .rounded_full()
                                    .bg(white())
                                    .shadow(if focus_visible {
                                        vec![
                                            BoxShadow::new(px(0.), px(0.), ring)
                                                .spread_radius(px(3.)),
                                        ]
                                    } else {
                                        vec![
                                            BoxShadow::new(
                                                px(0.),
                                                px(1.),
                                                gpui::black().opacity(0.3),
                                            )
                                            .blur_radius(px(4.)),
                                            BoxShadow::new(
                                                px(0.),
                                                px(0.),
                                                gpui::black().opacity(0.08),
                                            )
                                            .spread_radius(px(0.5)),
                                        ]
                                    }),
                            ),
                    ),
            )
    }
}

/// A small ring that empties as time runs out, such as a verification code's
/// remaining validity. `fraction` is what remains, 0 to 1.
pub fn progress_ring(fraction: f32, cx: &App) -> impl IntoElement {
    let colors = &cx.theme().colors;
    let (track, fill) = (colors.fill, colors.accent);
    let fraction = fraction.clamp(0., 1.);
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let width = px(2.4);
            let radius = bounds.size.width.min(bounds.size.height) / 2. - width / 2.;
            let center = bounds.center();
            if let Some(path) = arc(center, radius, width, 0., 0.9999) {
                window.paint_path(path, track);
            }
            // What remains runs clockwise from twelve o'clock.
            if fraction > 0.
                && let Some(path) = arc(center, radius, width, 0., fraction)
            {
                window.paint_path(path, fill);
            }
        },
    )
    .size(px(16.))
    .flex_none()
}

/// An arc going round while something is under way, in `color`.
pub fn spinner(
    id: impl Into<ElementId>,
    size: gpui::Pixels,
    color: gpui::Hsla,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Canvas<()> {
    let turn = m::spin(id, window, cx);
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let width = px(1.8);
            let radius = bounds.size.width.min(bounds.size.height) / 2. - width / 2.;
            if let Some(path) = arc(bounds.center(), radius, width, turn, turn + 0.7) {
                window.paint_path(path, color);
            }
        },
    )
    .size(size)
    .flex_none()
}

/// A stroked arc around `center`, between two turns measured clockwise from
/// twelve o'clock.
fn arc(
    center: gpui::Point<gpui::Pixels>,
    radius: gpui::Pixels,
    width: gpui::Pixels,
    from: f32,
    to: f32,
) -> Option<gpui::Path> {
    let at = |turn: f32| {
        let angle = std::f32::consts::TAU * turn - std::f32::consts::FRAC_PI_2;
        gpui::point(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        )
    };
    let mut path = gpui::PathBuilder::stroke(width);
    path.move_to(at(from));
    // Arcs of at most half a turn keep the large-arc flag simple.
    let mut start = from;
    while start < to {
        let end = (start + 0.5).min(to);
        path.arc_to(gpui::point(radius, radius), px(0.), false, true, at(end));
        start = end;
    }
    path.build().ok()
}
