//! Grouped cards: rounded groups of rows split by inset hairlines, for an
//! item's fields and for forms.

use gpui::base::Button as BaseButton;
use gpui::{
    AnyElement, App, BoxShadow, ClickEvent, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use smallvec::SmallVec;

use crate::icon::{IconName, icon};
use crate::motion as m;
use crate::theme::{ActiveTheme, Look};

#[derive(IntoElement)]
pub struct Card {
    rows: SmallVec<[AnyElement; 4]>,
    /// Cards inside sheets and popovers sit on a translucent surface.
    on_surface: bool,
}

pub fn card() -> Card {
    Card {
        rows: SmallVec::new(),
        on_surface: false,
    }
}

impl Card {
    pub fn row(mut self, row: impl IntoElement) -> Self {
        self.rows.push(row.into_any_element());
        self
    }

    pub fn rows(mut self, rows: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.rows
            .extend(rows.into_iter().map(IntoElement::into_any_element));
        self
    }

    pub fn on_surface(mut self) -> Self {
        self.on_surface = true;
        self
    }
}

impl RenderOnce for Card {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let separator = colors.separator;
        div()
            .flex()
            .flex_col()
            .rounded(theme.metrics.card_radius)
            .overflow_hidden()
            .bg(if self.on_surface {
                colors.frost_card
            } else {
                colors.card
            })
            .when_some(colors.card_border, |d, border| {
                d.border_1().border_color(border).shadow(vec![
                    BoxShadow::new(px(0.), px(1.), gpui::black().opacity(0.04)).blur_radius(px(2.)),
                ])
            })
            .children(self.rows.into_iter().enumerate().map(|(index, row)| {
                div()
                    .relative()
                    .when(index > 0, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top_0()
                                .left(px(13.))
                                .right_0()
                                .h(px(0.5))
                                .bg(separator),
                        )
                    })
                    .child(row)
            }))
    }
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// One field of an item: a small label over its value, with actions at the
/// end. Clicking the row copies the value; [`FieldRow::copied`] then swaps the
/// actions for a short "Copied" confirmation. A row without actions shows a
/// copy icon while hovered, so that it reads as clickable.
#[derive(IntoElement)]
pub struct FieldRow {
    id: ElementId,
    label: SharedString,
    value: AnyElement,
    actions: SmallVec<[AnyElement; 2]>,
    copied: bool,
    on_click: Option<ClickHandler>,
}

pub fn field_row(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    value: impl IntoElement,
) -> FieldRow {
    FieldRow {
        id: id.into(),
        label: label.into(),
        value: value.into_any_element(),
        actions: SmallVec::new(),
        copied: false,
        on_click: None,
    }
}

impl FieldRow {
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

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

impl RenderOnce for FieldRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let confirmation = m::presence((self.id.clone(), "copied"), self.copied, window, cx);
        let theme = cx.theme();
        let colors = &theme.colors;
        let hover = colors.hover;
        let clickable = self.on_click.is_some();
        let group: SharedString = format!("field-row-{}", self.id).into();
        let trailing = if confirmation.should_render() {
            copied_badge(cx)
                .opacity(confirmation.progress)
                .into_any_element()
        } else if self.actions.is_empty() && clickable {
            div()
                .invisible()
                .group_hover(group.clone(), |s| s.visible())
                .text_color(colors.text_tertiary)
                .child(icon(IconName::Copy))
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .text_color(colors.text_tertiary)
                .children(self.actions)
                .into_any_element()
        };
        BaseButton::new(self.id)
            .group(group)
            .accessibility_label(self.label.clone())
            .when_some(self.on_click, |b, handler| b.on_click(handler))
            .flex()
            .items_center()
            .gap(px(10.))
            .min_h(px(40.))
            .px(px(13.))
            .py(px(9.))
            .when(clickable, |b| b.hover(move |s| s.bg(hover)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(theme.metrics.small_text_size)
                            .text_color(colors.text_secondary)
                            .mb(px(1.))
                            .child(self.label),
                    )
                    .child(self.value),
            )
            .child(trailing)
    }
}

/// The green tick and "Copied" that confirm a copy.
pub fn copied_badge(cx: &App) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().colors.success)
        .child(icon(IconName::Check))
        .child("Copied")
}

/// A row of a form: a fixed-width label and the control it names.
pub fn form_row(label: impl Into<SharedString>, control: impl IntoElement, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let label_width = match theme.look {
        Look::MacOs => px(76.),
        Look::Gnome => px(84.),
    };
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .min_h(px(38.))
        .px(px(13.))
        .child(
            div()
                .w(label_width)
                .flex_none()
                .text_color(theme.colors.text_secondary)
                .child(label.into()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .child(control),
        )
        .into_any_element()
}

/// A form row whose label is the whole row and whose control sits at its end,
/// such as a switch.
pub fn toggle_row(label: impl Into<SharedString>, control: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .min_h(px(38.))
        .px(px(13.))
        .child(div().flex_1().child(label.into()))
        .child(control)
        .into_any_element()
}
