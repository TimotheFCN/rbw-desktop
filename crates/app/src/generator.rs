//! The password generator, the same view wherever it appears: the menu bar
//! panel's Generator tab, the main window's generator popover and, its
//! options only, the item editor. Each change of an option makes a new
//! value at once, which replaces the old one in place; the options are
//! remembered in the preferences for next time.

use std::sync::Arc;

use gpui::base::slider::{SliderEvent, SliderState};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontFeatures, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _, px, relative,
};
use rbw_desktop_core::{CharacterClasses, Characters, GeneratorOptions, Secret, Strength};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, button, card, motion, password_text,
    segmented_control, slider, switch, toggle_row,
};

use crate::backend;
use crate::clipboard;
use crate::preferences::{self, Prefs};

/// What the generator makes, in the order of its segmented control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Password,
    Passphrase,
    Pin,
}

const KINDS: [(Kind, &str); 3] = [
    (Kind::Password, "Password"),
    (Kind::Passphrase, "Passphrase"),
    (Kind::Pin, "PIN"),
];

impl Kind {
    fn index(self) -> usize {
        KINDS
            .iter()
            .position(|(kind, _)| *kind == self)
            .unwrap_or(0)
    }

    /// The shortest, longest and first length offered.
    fn lengths(self) -> (usize, usize, usize) {
        match self {
            Kind::Password => (8, 64, 20),
            Kind::Passphrase => (3, 10, 5),
            Kind::Pin => (4, 12, 6),
        }
    }

    /// What the length counts.
    fn length_label(self) -> &'static str {
        match self {
            Kind::Password => "Length",
            Kind::Passphrase => "Words",
            Kind::Pin => "Digits",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Kind::Password => "password",
            Kind::Passphrase => "passphrase",
            Kind::Pin => "PIN",
        }
    }
}

/// What goes between a passphrase's words, in the order of its choice.
const SEPARATORS: [(Option<char>, &str); 3] =
    [(Some('-'), "Hyphen"), (Some(' '), "Space"), (None, "None")];

/// The generator's choices. Each kind keeps its own length while another
/// kind is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Choices {
    kind: Kind,
    lengths: [usize; 3],
    numbers: bool,
    symbols: bool,
    /// Letters and digits that can't be mistaken for each other; rbw makes
    /// them without capitals or symbols.
    easy: bool,
    separator: Option<char>,
    capitalize: bool,
    /// A digit after one of a passphrase's words.
    number: bool,
}

impl Choices {
    fn from_options(options: GeneratorOptions) -> Self {
        let mut choices = Self {
            kind: Kind::Password,
            lengths: KINDS.map(|(kind, _)| kind.lengths().2),
            numbers: true,
            symbols: true,
            easy: false,
            separator: SEPARATORS[0].0,
            capitalize: false,
            number: false,
        };
        let (kind, length) = match options {
            GeneratorOptions::Password { length, characters } => {
                match characters {
                    Characters::DigitsOnly => return choices.with(Kind::Pin, length),
                    Characters::All => {}
                    Characters::NoSymbols => choices.symbols = false,
                    Characters::Unambiguous => choices.easy = true,
                    // Letters are always in; the switches are the others.
                    Characters::Classes(classes) => {
                        choices.numbers = classes.digits;
                        choices.symbols = classes.symbols;
                    }
                }
                (Kind::Password, length)
            }
            GeneratorOptions::Passphrase {
                words,
                separator,
                capitalize,
                number,
            } => {
                choices.separator = separator;
                choices.capitalize = capitalize;
                choices.number = number;
                (Kind::Passphrase, words)
            }
        };
        choices.with(kind, length)
    }

    /// These choices for `kind`, of `length`, within what it offers.
    fn with(mut self, kind: Kind, length: usize) -> Self {
        let (min, max, _) = kind.lengths();
        self.kind = kind;
        self.lengths[kind.index()] = length.clamp(min, max);
        self
    }
    fn length(&self) -> usize {
        self.lengths[self.kind.index()]
    }

    fn options(&self) -> GeneratorOptions {
        let length = self.length();
        match self.kind {
            Kind::Password => GeneratorOptions::Password {
                length,
                characters: if self.easy {
                    Characters::Unambiguous
                } else {
                    // Each class switched on is in the password, always.
                    Characters::Classes(CharacterClasses {
                        uppercase: true,
                        lowercase: true,
                        digits: self.numbers,
                        symbols: self.symbols,
                        each_class: true,
                    })
                },
            },
            Kind::Passphrase => GeneratorOptions::Passphrase {
                words: length,
                separator: self.separator,
                capitalize: self.capitalize,
                number: self.number,
            },
            Kind::Pin => GeneratorOptions::Password {
                length,
                characters: Characters::DigitsOnly,
            },
        }
    }

    /// The widest value these choices can make, which the shown value is
    /// laid over so that a new one never changes the height: every
    /// character takes the same width in the monospaced font, and no word
    /// of rbw's list is longer than nine letters.
    fn widest(&self) -> String {
        match self.kind {
            Kind::Password | Kind::Pin => "0".repeat(self.length()),
            Kind::Passphrase => {
                let separator = self.separator.map(String::from).unwrap_or_default();
                let words = vec!["mmmmmmmmm"; self.length()].join(&separator);
                if self.number { words + "0" } else { words }
            }
        }
    }
}

/// A new value was made, such as for the item editor's password field.
pub struct Generated(pub Secret);

impl EventEmitter<Generated> for Generator {}

pub struct Generator {
    choices: Choices,
    slider: Entity<SliderState>,
    slider_focus: FocusHandle,
    copy_focus: FocusHandle,
    value: Option<Secret>,
    /// Counts the values made, so that each fades up in place of the last.
    values: usize,
    in_flight: bool,
    /// Whether the options changed while a value was on the way.
    again: bool,
    problem: Option<&'static str>,
    copied: bool,
    copied_timer: Task<()>,
    request: Task<()>,
    _slider_events: Subscription,
}

impl Generator {
    /// A generator with the remembered options. `generate_now` makes the
    /// first value at once; without it, the first comes with the first
    /// change, as in the editor of an item that has a password.
    pub fn new(generate_now: bool, cx: &mut Context<Self>) -> Self {
        let choices = Choices::from_options(Prefs::global(cx).read(cx).get().generator);
        let (slider, slider_events) = Self::slider_for(&choices, cx);
        let mut this = Self {
            choices,
            slider,
            slider_focus: cx.focus_handle(),
            copy_focus: cx.focus_handle(),
            value: None,
            values: 0,
            in_flight: false,
            again: false,
            problem: None,
            copied: false,
            copied_timer: Task::ready(()),
            request: Task::ready(()),
            _slider_events: slider_events,
        };
        if generate_now {
            this.generate(cx);
        }
        this
    }

    fn slider_for(
        choices: &Choices,
        cx: &mut Context<Self>,
    ) -> (Entity<SliderState>, Subscription) {
        let (min, max, _) = choices.kind.lengths();
        let length = choices.length();
        let slider = cx.new(|_| {
            SliderState::new()
                .min(min as f32)
                .max(max as f32)
                .step(1.)
                .default_value(length as f32)
        });
        let events = cx.subscribe(&slider, |this, _, event: &SliderEvent, cx| match event {
            SliderEvent::Change(value) => {
                let length = value.end().round() as usize;
                let index = this.choices.kind.index();
                if this.choices.lengths[index] != length {
                    this.choices.lengths[index] = length;
                    this.generate(cx);
                }
            }
            SliderEvent::Release(_) => this.remember(cx),
        });
        (slider, events)
    }

    /// Makes another value with the same options.
    pub fn regenerate(&mut self, cx: &mut Context<Self>) {
        self.generate(cx);
    }

    /// How hard the current options' values are to guess; `None` for a
    /// PIN, which is kept safe by what it unlocks giving few tries rather
    /// than by its length.
    pub fn strength(&self) -> Option<Strength> {
        (self.choices.kind != Kind::Pin).then(|| self.choices.options().strength())
    }

    /// "password", "passphrase" or "PIN".
    pub fn noun(&self) -> &'static str {
        self.choices.kind.noun()
    }

    fn generate(&mut self, cx: &mut Context<Self>) {
        if self.in_flight {
            self.again = true;
            return;
        }
        self.in_flight = true;
        let options = self.choices.options();
        self.request = backend::request(
            cx,
            move |backend| backend.generate(&options),
            |this, result, cx| {
                this.in_flight = false;
                match result {
                    Ok(value) => {
                        this.values += 1;
                        this.problem = None;
                        this.copied = false;
                        this.value = Some(value.clone());
                        cx.emit(Generated(value));
                    }
                    Err(error) => this.problem = Some(backend::describe(&error)),
                }
                if std::mem::take(&mut this.again) {
                    this.generate(cx);
                }
                cx.notify();
            },
        );
    }

    fn remember(&self, cx: &mut App) {
        let options = self.choices.options();
        preferences::update(cx, move |preferences| preferences.generator = options);
    }

    /// Applies a change of the choices: a new value now, remembered.
    fn change(&mut self, change: impl FnOnce(&mut Choices), cx: &mut Context<Self>) {
        let kind = self.choices.kind;
        change(&mut self.choices);
        if self.choices.kind != kind {
            let (slider, events) = Self::slider_for(&self.choices, cx);
            self.slider = slider;
            self._slider_events = events;
        }
        self.generate(cx);
        self.remember(cx);
        cx.notify();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let Some(value) = &self.value else { return };
        clipboard::copy_secret(value, None, cx);
        self.copied = true;
        self.copied_timer = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(motion::CONFIRMATION_HOLD)
                .await;
            this.update(cx, |this, cx| {
                this.copied = false;
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    /// The value, large, over the room the widest value would take, with
    /// how strong it is under it. Each new value fades up in place of the
    /// last; a quick run of them, as the length slides, stays faint until
    /// it rests. When the room changes, as with the length, it grows or
    /// shrinks to it rather than jump.
    fn render_value(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let mono = theme.mono_font_family.clone();
        let dim = colors.text_secondary;
        let danger = colors.danger;
        let shown = match (&self.value, self.problem) {
            (_, Some(problem)) => div()
                .text_color(danger)
                .font_family(theme.font_family.clone())
                .text_size(px(13.))
                .child(problem)
                .into_any_element(),
            (Some(value), None) => password_text(value.expose(), cx).into_any_element(),
            (None, None) => div().into_any_element(),
        };
        let strength = self.strength().map(|strength| {
            let (color, word) = strength_look(strength, cx);
            div()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .mt(px(8.))
                .text_size(px(12.))
                .text_color(dim)
                .child(strength_dot(color))
                .child(word)
        });
        let label = format!("Generated {}", self.noun());
        div()
            .id("generated")
            .role(gpui::Role::Label)
            .aria_label(label)
            .w_full()
            .px(px(12.))
            .py(px(16.))
            .flex()
            .flex_col()
            .child(motion::grow(
                "generated-room",
                div()
                    .relative()
                    .w_full()
                    .text_center()
                    .font_family(mono)
                    .text_size(px(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .line_height(relative(1.5))
                    .child(div().invisible().child(self.choices.widest()))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(motion::replace(
                                ("generated-value", self.values),
                                div().w_full().child(shown),
                            )),
                    ),
            ))
            .children(strength)
            .into_any_element()
    }

    /// The kind, the length and, for passwords, which characters.
    pub fn render_options(&self, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity().downgrade();
        let kinds = segmented_control("generator-kind", KINDS.map(|(_, label)| label), {
            self.choices.kind.index()
        })
        .on_select(move |index, _, cx| {
            this.update(cx, |this, cx| {
                this.change(|choices| choices.kind = KINDS[index].0, cx)
            })
            .ok();
        });
        let tabular = FontFeatures(Arc::new(vec![("tnum".into(), 1)]));
        let length = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .min_h(px(38.))
            .px(px(13.))
            .child(self.choices.kind.length_label())
            .child(
                div().flex_1().px(px(4.)).flex().child(
                    slider(&self.slider, self.choices.kind.length_label())
                        .track_focus(&self.slider_focus),
                ),
            )
            .child(
                div()
                    .w(px(20.))
                    .text_right()
                    .font_weight(FontWeight::SEMIBOLD)
                    .font_features(tabular)
                    .child(self.choices.length().to_string()),
            );
        let mut rows = vec![
            div().p(px(6.)).child(kinds).into_any_element(),
            length.into_any_element(),
        ];
        let choices = self.choices;
        match choices.kind {
            // Easy to read takes its own digits and no symbols.
            Kind::Password => rows.extend([
                self.toggle("Numbers", choices.numbers || choices.easy, cx, |c, on| {
                    c.numbers = on;
                    c.easy &= on;
                }),
                self.toggle("Symbols", choices.symbols && !choices.easy, cx, |c, on| {
                    c.symbols = on;
                    c.easy &= !on;
                }),
                self.toggle("Easy to read", choices.easy, cx, |c, on| c.easy = on),
            ]),
            Kind::Passphrase => rows.extend([
                self.toggle("Capitalize", choices.capitalize, cx, |c, on| {
                    c.capitalize = on
                }),
                self.toggle("Include a number", choices.number, cx, |c, on| {
                    c.number = on
                }),
                self.render_separator(cx),
            ]),
            Kind::Pin => {}
        }
        card().on_surface().rows(rows).into_any_element()
    }

    /// A switch for one choice.
    fn toggle(
        &self,
        label: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        change: fn(&mut Choices, bool),
    ) -> AnyElement {
        let this = cx.entity().downgrade();
        toggle_row(
            label,
            switch(label, on, label).on_change(move |on, _, cx| {
                this.update(cx, |this, cx| {
                    this.change(|choices| change(choices, on), cx)
                })
                .ok();
            }),
        )
    }

    /// What goes between a passphrase's words.
    fn render_separator(&self, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity().downgrade();
        let selected = SEPARATORS
            .iter()
            .position(|(separator, _)| *separator == self.choices.separator)
            .unwrap_or(0);
        toggle_row(
            "Separator",
            div().w(px(184.)).child(
                segmented_control("separator", SEPARATORS.map(|(_, label)| label), selected)
                    .on_select(move |index, _, cx| {
                        this.update(cx, |this, cx| {
                            this.change(|choices| choices.separator = SEPARATORS[index].0, cx)
                        })
                        .ok();
                    }),
            ),
        )
    }
}

/// The Copy button takes the keyboard, so that Return copies.
impl Focusable for Generator {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.copy_focus.clone()
    }
}

/// The generator's width in a popover.
pub const POPOVER_WIDTH: Pixels = px(282.);

/// The dot before the word that says how strong a value is.
pub fn strength_dot(color: Hsla) -> gpui::Div {
    div().size(px(7.)).rounded_full().bg(color)
}

/// The colour of the dot and the word that say how strong a value is.
pub fn strength_look(strength: Strength, cx: &App) -> (Hsla, &'static str) {
    let colors = &cx.theme().colors;
    match strength {
        Strength::Weak => (colors.danger, "Weak"),
        Strength::Fair => (colors.warning, "Fair"),
        Strength::Strong => (colors.success, "Strong"),
    }
}

impl Render for Generator {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let copy_label = format!("Copy {}", self.noun());
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(card().on_surface().row(self.render_value(cx)))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        div().flex_1().child(
                            button("copy-generated")
                                .label(if self.copied { "Copied" } else { "Copy" })
                                .when(self.copied, |b| b.icon(IconName::Check))
                                .accessibility_label(copy_label)
                                .style(ButtonStyle::Primary)
                                .wide()
                                .disabled(self.value.is_none())
                                .track_focus(&self.copy_focus)
                                .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
                        ),
                    )
                    .child(
                        button("regenerate")
                            .icon(IconName::Refresh)
                            .accessibility_label("Generate Another")
                            .on_click(cx.listener(|this, _, _, cx| this.regenerate(cx))),
                    ),
            )
            .child(self.render_options(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn password(length: usize, numbers: bool, symbols: bool) -> GeneratorOptions {
        GeneratorOptions::Password {
            length,
            characters: Characters::Classes(CharacterClasses {
                uppercase: true,
                lowercase: true,
                digits: numbers,
                symbols,
                each_class: true,
            }),
        }
    }

    #[test]
    fn choices_round_trip_and_clamp() {
        for options in [
            password(20, true, true),
            password(16, false, true),
            password(30, true, false),
            password(12, false, false),
            GeneratorOptions::Passphrase {
                words: 6,
                separator: None,
                capitalize: true,
                number: true,
            },
            GeneratorOptions::Password {
                length: 6,
                characters: Characters::DigitsOnly,
            },
            GeneratorOptions::Password {
                length: 12,
                characters: Characters::Unambiguous,
            },
        ] {
            assert_eq!(Choices::from_options(options).options(), options);
        }
        let long = Choices::from_options(GeneratorOptions::passphrase(99));
        assert_eq!(
            long.options(),
            GeneratorOptions::Passphrase {
                words: 10,
                separator: Some(' '),
                capitalize: false,
                number: false,
            }
        );
    }

    #[test]
    fn older_options_keep_their_characters_and_hold_each() {
        let all = GeneratorOptions::Password {
            length: 20,
            characters: Characters::All,
        };
        assert_eq!(
            Choices::from_options(all).options(),
            password(20, true, true)
        );
        let no_symbols = GeneratorOptions::Password {
            length: 20,
            characters: Characters::NoSymbols,
        };
        assert_eq!(
            Choices::from_options(no_symbols).options(),
            password(20, true, false)
        );
    }

    #[test]
    fn every_choice_makes_something() {
        for numbers in [false, true] {
            for symbols in [false, true] {
                for easy in [false, true] {
                    let (min, max, _) = Kind::Password.lengths();
                    for length in [min, max] {
                        let mut choices = Choices::from_options(password(length, numbers, symbols));
                        choices.easy = easy;
                        assert!(choices.options().makes_anything(), "{choices:?}");
                    }
                }
            }
        }
        for (separator, _) in SEPARATORS {
            let mut choices = Choices::from_options(GeneratorOptions::passphrase(3));
            choices.separator = separator;
            assert!(choices.options().makes_anything());
        }
    }

    #[test]
    fn each_kind_keeps_its_length() {
        let mut choices = Choices::from_options(password(32, true, true));
        choices.kind = Kind::Passphrase;
        assert_eq!(choices.length(), 5);
        choices.kind = Kind::Password;
        assert_eq!(choices.length(), 32);
    }
}
