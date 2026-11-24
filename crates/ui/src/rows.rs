//! Rows of the sidebar and the item list, and the letter tile that stands for
//! an item.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::base::Button as BaseButton;
use gpui::base::motion;
use gpui::{
    AnyElement, App, Bounds, ClickEvent, ContentMask, ElementId, Entity, FontWeight, Hsla,
    InteractiveElement, IntoElement, ObjectFit, ParentElement, Pixels, RenderImage, RenderOnce,
    SharedString, Styled, StyledImage, Window, canvas, div, img, linear_color_stop,
    linear_gradient, point, prelude::FluentBuilder as _, px, relative, size,
};

use crate::card::copied_badge;
use crate::focus::{inner_ring, row_ring};
use crate::icon::{IconName, icon};
use crate::motion as m;
use crate::theme::{ActiveTheme, Look, mix};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A selection highlight that slides between rows, such as a list's or a
/// sidebar's: what a row draws of it, and how much of the row it covers.
///
/// The highlight is drawn by the rows it overlaps, each clipping it to
/// itself, so that it can move across rows that are drawn on their own,
/// as a list's are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Highlight {
    /// Where the highlight's top is, from the row's top.
    offset: Pixels,
    /// How far down from its top the row draws it: to the next row, so
    /// that the gap between two rows shows no seam.
    reach: Pixels,
}

impl Highlight {
    /// The highlight `rows` rows from this row, a fraction while it moves,
    /// in a list whose rows are `pitch` apart; `None` once it is a row or
    /// more away. [`motion::slide`](crate::motion::slide) gives where it
    /// is.
    pub fn at(rows: f32, pitch: Pixels) -> Option<Self> {
        (rows.abs() < 1.).then(|| Self {
            offset: pitch * rows,
            reach: pitch,
        })
    }

    /// On the row, where a selected row has it without a list to slide in.
    fn here(height: Pixels) -> Self {
        Self {
            offset: px(0.),
            reach: height,
        }
    }

    /// How far a row `height` tall reads as selected, 0 to 1: its text
    /// takes the selection's colours once the highlight covers most of it,
    /// over a short way rather than through every shade in between, which
    /// would read poorly on either fill.
    fn selected(self, height: Pixels) -> f32 {
        let cover = 1. - (self.offset / height).abs();
        ((cover - 0.35) / 0.3).clamp(0., 1.)
    }

    /// The highlight, painted behind the row's content and clipped to the
    /// row, which must be `relative`: painted rather than laid out, so that
    /// the row keeps its size wherever the highlight is.
    fn render(self, radius: Pixels, fill: Hsla) -> impl IntoElement {
        canvas(
            |_, _, _| {},
            move |row: Bounds<Pixels>, _, window, _| {
                let at = Bounds::new(row.origin + point(px(0.), self.offset), row.size);
                let clip = Bounds::new(row.origin, size(row.size.width, self.reach));
                window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                    window.paint_quad(gpui::fill(at, fill).corner_radii(radius));
                });
            },
        )
        .absolute()
        .inset_0()
    }
}

/// Where the rows of a [`selection_track`] are, kept while they show.
#[derive(Default)]
pub struct SelectionTrack {
    /// Each row's top and height in the window, as drawn in this frame.
    drawing: HashMap<ElementId, (Pixels, Pixels)>,
    /// The same, as drawn in the frame before, which the highlight of this
    /// one was placed from.
    drawn: HashMap<ElementId, (Pixels, Pixels)>,
    /// Each row's top from the topmost row's, height and distance to the
    /// next row when it follows closely, as drawn last: the same however
    /// the rows scroll.
    rows: HashMap<ElementId, (Pixels, Pixels, Pixels)>,
    /// Where the highlight is, from the topmost row's top.
    at: Option<Pixels>,
}

impl SelectionTrack {
    /// The highlight as the row `id` draws it.
    fn highlight(&self, id: &ElementId) -> Option<Highlight> {
        let (top, height, reach) = *self.rows.get(id)?;
        let offset = self.at? - top;
        (offset.abs() < height).then_some(Highlight { offset, reach })
    }
}

/// The sliding selection of rows of different places and gaps, such as a
/// sidebar's with its headings: the highlight slides to the row `selected`,
/// on [`motion::slide`](crate::motion::slide), from where the rows were
/// drawn last, which they record as they are drawn. Make it before the
/// rows, and give it to each with [`NavRow::track`].
pub fn selection_track(
    id: impl Into<ElementId>,
    selected: Option<&ElementId>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SelectionTrack> {
    let id = id.into();
    let track = window.use_keyed_state(id.clone(), cx, |_, _| SelectionTrack::default());
    let target = track.update(cx, |track, _| {
        track.drawn = std::mem::take(&mut track.drawing);
        let drawn = &track.drawn;
        let first = drawn
            .values()
            .map(|(top, _)| *top)
            .reduce(|first, top| if top < first { top } else { first });
        if let Some(first) = first {
            let tops: Vec<Pixels> = drawn.values().map(|(top, _)| *top).collect();
            track.rows = drawn
                .iter()
                .map(|(id, &(top, height))| {
                    // A row a few points under it closes the gap.
                    let next = tops
                        .iter()
                        .copied()
                        .filter(|next| *next > top)
                        .reduce(|next, other| if other < next { other } else { next });
                    let reach = next
                        .map(|next| next - top)
                        .filter(|reach| *reach <= height + px(4.))
                        .unwrap_or(height);
                    (id.clone(), (top - first, height, reach))
                })
                .collect();
        }
        selected
            .and_then(|selected| track.rows.get(selected))
            .map(|(top, height, _)| (*top, *height))
    });
    let at = target.map(|(top, height)| {
        m::slide((id, "highlight"), top / height, false, None, window, cx) * height
    });
    track.update(cx, |track, _| track.at = at);
    track
}

/// Records a row's place in its track as it is drawn. A row drawn
/// elsewhere than in the frame before, as when a heading comes in above
/// it, draws again: the highlight was placed from where it was.
fn track_row(track: &Entity<SelectionTrack>, id: ElementId) -> impl IntoElement {
    let track = track.clone();
    canvas(
        move |bounds, window, cx| {
            let place = (bounds.top(), bounds.size.height);
            let moved = track.update(cx, |track, _| {
                let moved = track.drawn.get(&id) != Some(&place);
                track.drawing.insert(id, place);
                moved
            });
            if moved {
                window.request_animation_frame();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

/// A sidebar destination: an icon, a label, and the selection behind them.
#[derive(IntoElement)]
pub struct NavRow {
    id: ElementId,
    icon: IconName,
    label: SharedString,
    selected: bool,
    track: Option<Entity<SelectionTrack>>,
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
        track: None,
        on_click: None,
    }
}

impl NavRow {
    /// Slides the selection to this row from the one before, along
    /// `track`, which all the rows it moves between share.
    pub fn track(mut self, track: &Entity<SelectionTrack>) -> Self {
        self.track = Some(track.clone());
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

impl RenderOnce for NavRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        // Without a track, or before it knows where the rows are, the
        // highlight is on the selected row.
        let tracked = self
            .track
            .as_ref()
            .map(|track| track.read(cx))
            .filter(|track| track.at.is_some());
        let highlight = match tracked {
            Some(track) => track.highlight(&self.id),
            None => self
                .selected
                .then(|| Highlight::here(cx.theme().metrics.nav_row_height)),
        };
        let theme = cx.theme();
        let colors = &theme.colors;
        let metrics = theme.metrics;
        let icon_color = match theme.look {
            Look::MacOs => colors.accent_text,
            Look::Gnome => colors.text,
        };
        let fill = colors.sidebar_selection;
        let hover = colors.hover;
        let ring = row_ring(colors.focus_ring);
        // A Mac's sidebar says which row is chosen in bold, GNOME's in a
        // heavier weight.
        let weight = match theme.look {
            Look::MacOs => FontWeight::BOLD,
            Look::Gnome => FontWeight::SEMIBOLD,
        };
        let (height, radius) = (metrics.nav_row_height, metrics.nav_row_radius);
        let id = self.id.clone();
        BaseButton::new(self.id)
            .selected(self.selected)
            .accessibility_label(self.label.clone())
            .when_some(self.on_click, |b, handler| b.on_click(handler))
            .focus_visible(ring)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(9.))
            .h(height)
            .px(px(8.))
            .rounded(radius)
            .when(!self.selected && theme.look == Look::Gnome, |b| {
                b.hover(move |s| s.bg(hover))
            })
            .when(self.selected, |b| b.font_weight(weight))
            .when_some(highlight, |b, highlight| {
                b.child(highlight.render(radius, fill))
            })
            .when_some(self.track, |b, track| b.child(track_row(&track, id)))
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

/// What a tile shows of a website's icon.
#[derive(Clone)]
pub enum TileIcon {
    /// Being looked for: the tile stays neutral, so that a letter in a
    /// colour doesn't flash before the icon.
    Pending,
    Image(Arc<RenderImage>),
    /// There is none: the tile shows its letter in its colour.
    Letter,
}

/// The coloured rounded square with an item's initial, or the website's
/// icon in its place.
#[derive(IntoElement)]
pub struct Tile {
    letter: SharedString,
    hue: f32,
    size: Pixels,
    icon: Option<(ElementId, TileIcon)>,
    glyph: Option<IconName>,
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
        glyph: None,
    }
}

impl Tile {
    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    /// Shows a glyph in place of the letter, on a tile of `hue`, such as an
    /// item's kind while it has no name yet.
    pub fn glyph(mut self, glyph: IconName, hue: f32) -> Self {
        self.glyph = Some(glyph);
        self.hue = hue;
        self
    }

    /// Shows a website's icon, or a neutral square while it is looked for.
    /// What arrives fades in; what was known when the tile first showed is
    /// there at once. `id` names the tile.
    pub fn icon(mut self, id: impl Into<ElementId>, icon: TileIcon) -> Self {
        self.icon = Some((id.into(), icon));
        self
    }
}

impl RenderOnce for Tile {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (lettered, image) = match self.icon {
            None => (1., None),
            Some((id, icon)) => {
                let mut appear = |channel: &'static str, shown: bool| {
                    motion::transition(
                        (id.clone(), channel),
                        if shown { 1. } else { 0. },
                        m::fade(m::ARRIVE),
                        window,
                        cx,
                    )
                };
                let lettered = appear("letter", matches!(icon, TileIcon::Letter));
                let shown = appear("icon", matches!(icon, TileIcon::Image(_)));
                let image = match icon {
                    TileIcon::Image(image) if shown > 0. => Some((image, shown)),
                    _ => None,
                };
                (lettered, image)
            }
        };
        let theme = cx.theme();
        let (light, dark) = theme.colors.tile(self.hue);
        let neutral = theme.colors.fill;
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
                linear_color_stop(mix(neutral, light, lettered), 0.),
                linear_color_stop(mix(neutral, dark, lettered), 1.),
            ))
            .text_color(gpui::white().opacity(lettered))
            .text_size(self.size * 0.44)
            .font_weight(FontWeight::SEMIBOLD)
            .map(|d| match self.glyph {
                Some(glyph) => d.child(icon(glyph).size(self.size * 0.52)),
                None => d.child(self.letter),
            })
            .when_some(image, |d, (image, shown)| {
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
    icon: TileIcon,
    trailing: Option<AnyElement>,
    copied: bool,
    compact: bool,
    /// Whether the row is a stop of Tab, rather than reached with the
    /// arrow keys through its list.
    tab_stop: bool,
    /// Whether its list has the keyboard, which it shows for it.
    focused: bool,
    /// The list's selection, where it is drawn on this row.
    highlight: Option<Highlight>,
    /// Whether the selection is on this row without a list to slide in.
    here: bool,
    /// Whether the pointer is on the row, as its list tells.
    hovered: bool,
    accessibility_label: Option<SharedString>,
    on_click: Option<ClickHandler>,
}

/// The height of a row of the item list.
pub const ROW_HEIGHT: Pixels = px(46.);
/// The height of a row of a panel's list, [`ListRow::compact`].
pub const COMPACT_ROW_HEIGHT: Pixels = px(40.);
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
        icon: TileIcon::Letter,
        trailing: None,
        copied: false,
        compact: false,
        tab_stop: true,
        focused: false,
        highlight: None,
        here: selected,
        hovered: false,
        accessibility_label: None,
        on_click: None,
    }
}

impl ListRow {
    /// A row of a panel or popover, as in the menu bar panel: a smaller
    /// tile, and a neutral highlight in place of the accent, as a menu's.
    /// As in a menu, the selection follows the pointer, so the row shows
    /// no hover of its own, and it never takes the keyboard from the field
    /// that moves through its list.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// The website's icon to show in the tile.
    pub fn icon(mut self, icon: TileIcon) -> Self {
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

    /// Draws the list's selection where it is, as it slides between rows,
    /// rather than on this row when it is selected.
    pub fn highlight(mut self, highlight: Option<Highlight>) -> Self {
        self.highlight = highlight;
        self.here = false;
        self
    }

    /// The pointer is on the row: it shows a hover unless it is selected.
    /// The list tells, from [`follow_pointer`](crate::follow_pointer),
    /// rather than the row from GPUI's hover, which lags behind a list
    /// that scrolls.
    pub fn hovered(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
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
        let height = if self.compact {
            COMPACT_ROW_HEIGHT
        } else {
            ROW_HEIGHT
        };
        let highlight = self
            .highlight
            .or(self.here.then(|| Highlight::here(height)));
        let progress = highlight.map_or(0., |highlight| highlight.selected(height));
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
        let radius = theme.metrics.row_radius;
        let name = self.name;
        let tile = tile(&name)
            .size(if self.compact { px(28.) } else { px(32.) })
            .icon((self.id.clone(), "tile"), self.icon);
        BaseButton::new(self.id)
            .selected(self.selected)
            .accessibility_label(self.accessibility_label.unwrap_or_else(|| name.clone()))
            .when_some(self.on_click, |b, handler| b.on_click(handler))
            .tab_stop(self.tab_stop)
            .focusable(!self.compact)
            .focus_visible(row_ring(ring_color))
            .when(list_focused && self.selected, |b| {
                b.shadow(inner_ring(ring_color))
            })
            .relative()
            .flex()
            .items_center()
            .gap(px(10.))
            .h(height)
            .px(if self.compact { px(7.) } else { px(9.) })
            .rounded(radius)
            .when(self.hovered && !self.selected && !self.compact, |b| {
                b.bg(hover)
            })
            .when_some(highlight, |b, highlight| {
                b.child(highlight.render(radius, selection))
            })
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
