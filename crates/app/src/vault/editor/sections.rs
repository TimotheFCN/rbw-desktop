//! How the editor lays out an item: a header with its tile and name, the
//! choice of kind for a new item, then the fields of its kind in the
//! groups the item view shows them in, its notes, "Add Field", and its
//! folder and whether it is a favourite. Every field has its glyph and a
//! plain label; each group grows and shrinks with what it holds, so the
//! sheet does too, and scrolls inside once it is as tall as the window.

use gpui::base::Button as BaseButton;
use gpui::base::FocusTrapElement as _;
use gpui::base::input::{Input, Textarea, TextareaState};
use gpui::{
    AnyElement, App, Context, ElementId, Entity, FocusHandle, Focusable as _, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _, Window,
    div, point, prelude::FluentBuilder as _, px, relative,
};
use rbw_desktop_core::ItemKind;
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Opening, Popover, TileIcon, button, card, field_error,
    form_row, icon, menu_item, menu_separator, motion, password_text, popover, row_focus_ring,
    scroll_area, segmented_control, switch, tile,
};

use super::form::Key;
use super::{
    CONTEXT, Cancel, EXTRAS, Extra, FieldInput, ItemEditor, KINDS, NEW_FOLDER_CONTEXT, NewField,
    Save, describe, kind_index,
};
use crate::generator;
use crate::icons;

/// A group of rows, named so that it keeps its place and its size from
/// one frame to the next, with the fields in it that take the keyboard,
/// to bring it into view when one does. `None` while it shows nothing,
/// as a group of another kind's.
struct Group {
    id: &'static str,
    element: Option<AnyElement>,
    focus: Vec<FocusHandle>,
}

impl Group {
    fn new(id: &'static str, element: impl IntoElement, focus: Vec<FocusHandle>) -> Self {
        Self {
            id,
            element: Some(element.into_any_element()),
            focus,
        }
    }

    fn empty(id: &'static str) -> Self {
        Self {
            id,
            element: None,
            focus: vec![],
        }
    }
}

/// The custom fields "Add Field" adds: their name, what they are for, and
/// their glyph.
const NEW_FIELDS: [(NewField, &str, &str, IconName); 3] = [
    (
        NewField::Text,
        "Text",
        "Such as a customer number",
        IconName::Text,
    ),
    (
        NewField::Hidden,
        "Hidden Text",
        "Hidden until shown, such as a PIN",
        IconName::Eye,
    ),
    (NewField::Boolean, "Checkbox", "Yes or no", IconName::Check),
];

impl ItemEditor {
    /// A row of one line of text, with the field's glyph and label.
    fn line(&self, key: Key, cx: &App) -> (AnyElement, FocusHandle) {
        let (label, glyph, _) = describe(key);
        let input = self.input(key);
        (
            form_row(glyph, label, Input::new(input), cx),
            input.read(cx).focus_handle(cx),
        )
    }

    /// A card of plain lines.
    fn lines(&self, id: &'static str, keys: &[Key], cx: &App) -> Group {
        let (rows, focus): (Vec<_>, Vec<_>) = keys.iter().map(|key| self.line(*key, cx)).unzip();
        Group::new(id, card().on_surface().rows(rows), focus)
    }

    /// What the item is: its tile, its name as a large title, and what is
    /// being done to it. The tile is the website's icon once its address
    /// is one rbw keeps an icon for, else the letter of its name, or the
    /// glyph of its kind while it has none.
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let name_input = self.input(Key::Name);
        let name = name_input.read(cx).value().to_string();
        // A website still being typed isn't looked up.
        let website = self
            .websites
            .first()
            .filter(|row| !row.input.read(cx).focus_handle(cx).is_focused(window))
            .map(|row| row.input.read(cx).value().trim().to_string())
            .filter(|website| self.kind == ItemKind::Login && !website.is_empty());
        let image = website.and_then(|website| icons::kept_icon(&self.account, &website, cx));
        let (_, kind_label, glyph, hue) = KINDS[kind_index(self.kind)];
        let tile = match image {
            Some(image) => tile(&name)
                .size(px(52.))
                .icon("editor-tile", TileIcon::Image(image))
                .into_any_element(),
            None if name.trim().is_empty() => motion::replace(
                ("editor-glyph", self.kind as usize),
                div().child(tile("").glyph(glyph, hue).size(px(52.))),
            )
            .into_any_element(),
            None => tile(&name).size(px(52.)).into_any_element(),
        };
        let shake = motion::shake("editor-name", self.name_missing, window, cx);
        let theme = cx.theme();
        let colors = &theme.colors;
        let (dim, fill, small) = (
            colors.text_secondary,
            colors.fill,
            theme.metrics.small_text_size,
        );
        let doing = match &self.editing {
            Some(_) => format!("Edit {kind_label}"),
            None => format!("New {kind_label}"),
        };
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .pb(px(14.))
            .child(tile)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(small)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(dim)
                            .child(doing),
                    )
                    .child(
                        div()
                            .relative()
                            .left(shake)
                            .px(px(8.))
                            .py(px(3.))
                            .ml(px(-8.))
                            .rounded(px(8.))
                            .bg(fill)
                            .text_size(px(20.))
                            .font_weight(FontWeight::BOLD)
                            .child(Input::new(name_input)),
                    )
                    // Said only once Save found it empty.
                    .when(self.name_missing > 0, |d| {
                        d.child(field_error("Required: give it a name to save it.", cx))
                    }),
            )
            .into_any_element()
    }

    /// The fields that say what the item is.
    fn render_main(&self, window: &Window, cx: &mut Context<Self>) -> Group {
        let mut keys = vec![];
        match self.kind {
            ItemKind::Login => keys.push(Key::Username),
            ItemKind::Card => keys.extend([
                Key::Cardholder,
                Key::Brand,
                Key::Number,
                Key::Expiry,
                Key::SecurityCode,
            ]),
            ItemKind::Identity => {
                let full = self.shows(Extra::TitleAndMiddleName, cx);
                if full {
                    keys.push(Key::Title);
                }
                keys.push(Key::FirstName);
                if full {
                    keys.push(Key::MiddleName);
                }
                keys.push(Key::LastName);
            }
            ItemKind::SshKey if self.editing.is_none() => {
                return Group::new(
                    "main",
                    self.text_card(IconName::Terminal, "Private key", &self.private_key, cx),
                    vec![self.private_key.read(cx).focus_handle(cx)],
                );
            }
            ItemKind::SecureNote | ItemKind::SshKey => return Group::empty("main"),
        }
        let (mut rows, mut focus): (Vec<_>, Vec<_>) =
            keys.iter().map(|key| self.line(*key, cx)).unzip();
        if self.kind == ItemKind::Login {
            rows.push(self.render_password(window, cx));
            focus.push(self.input(Key::Password).read(cx).focus_handle(cx));
            if self.shows(Extra::Authenticator, cx) {
                let (row, handle) = self.line(Key::Authenticator, cx);
                rows.push(row);
                focus.push(handle);
            }
            let (websites, handles) = self.render_websites(cx);
            rows.extend(websites);
            focus.extend(handles);
        }
        // Another kind's fields fade up in place as the group springs to
        // their height.
        Group::new(
            "main",
            motion::replace(
                ("item-kind", self.kind as usize),
                div().child(card().on_surface().rows(rows)),
            ),
            focus,
        )
    }

    /// The password, its digits and symbols coloured while it shows and
    /// isn't being edited, with buttons to show it, make another and change
    /// how it is made, and under it how strong a generated one is. The
    /// field stays underneath, so it still takes the keyboard and the
    /// click. A new value arrives in place.
    fn render_password(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let input = self.input(Key::Password);
        let editing = input.read(cx).focus_handle(cx).is_focused(window);
        let value = input.read(cx).value();
        let coloured = !editing && self.password_visible();
        let theme = cx.theme();
        let mono = theme.mono_font_family.clone();
        let accent = theme.colors.accent_text;
        let field = div()
            .flex_1()
            .min_w_0()
            .relative()
            .font_family(mono)
            .text_size(px(12.5))
            .child(
                div()
                    .when(coloured, |d| d.text_color(gpui::transparent_black()))
                    .child(Input::new(input)),
            )
            .when(coloured, |d| {
                d.child(
                    motion::replace(
                        ("generated", self.generations),
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(password_text(&value, cx)),
                    )
                    .into_any_element(),
                )
            });
        let small = |id: &'static str, name: IconName, label: &'static str| {
            button(id)
                .icon(name)
                .style(ButtonStyle::Flat)
                .size(px(28.))
                .accessibility_label(label)
        };
        let this = cx.entity().downgrade();
        let options = popover(
            "generator-options",
            self.generator_open,
            &self.generator_focus,
            "Password Options",
        )
        .dialog()
        .opening(Opening::BelowEnd)
        .on_dismiss(move |window, cx| {
            this.update(cx, |this, cx| this.close_generator(window, cx))
                .ok();
        })
        .child(
            div().w(generator::POPOVER_WIDTH).child(
                self.generator
                    .update(cx, |generator, cx| generator.render_options(cx)),
            ),
        );
        let buttons = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(
                div()
                    .when(self.password_shown, |d| d.text_color(accent))
                    .child(
                        small(
                            "show-password",
                            IconName::Eye,
                            if self.password_shown {
                                "Hide Password"
                            } else {
                                "Show Password"
                            },
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.toggle_password(window, cx)),
                        ),
                    ),
            )
            .child(
                small("generate", IconName::Refresh, "Generate Another")
                    .on_click(cx.listener(|this, _, _, cx| this.regenerate(cx))),
            )
            .child(
                div()
                    .relative()
                    .child(
                        small(
                            "generator-options-button",
                            IconName::Sliders,
                            "Password Options",
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.toggle_generator(window, cx)),
                        ),
                    )
                    .child(options),
            );
        let row = form_row(
            IconName::Key,
            "Password",
            div()
                .flex_1()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(field)
                .child(buttons),
            cx,
        );
        div()
            .flex()
            .flex_col()
            .child(row)
            .when(self.generated, |d| d.child(self.render_strength(cx)))
            .into_any_element()
    }

    /// How strong a generated password is, from the randomness its options
    /// hold. One typed gets no score: nothing about it can be known for
    /// sure.
    fn render_strength(&self, cx: &App) -> AnyElement {
        let generator = self.generator.read(cx);
        let (dot, line) = match (generator.strength(), generator.entropy()) {
            (Some(strength), Some(bits)) => {
                let (color, word) = generator::strength_look(strength, cx);
                (Some(color), format!("{word}: {bits:.0} bits of randomness"))
            }
            _ => (None, format!("Generated {}", generator.noun())),
        };
        // Under the field, past the glyph and the label.
        let indent = px(14. + 16. + 10.)
            + match cx.theme().look {
                rbw_desktop_ui::Look::MacOs => px(100. + 10.),
                rbw_desktop_ui::Look::Gnome => px(110. + 10.),
            };
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(indent)
            .pr(px(14.))
            .pb(px(10.))
            .mt(px(-6.))
            .text_size(cx.theme().metrics.small_text_size)
            .text_color(cx.theme().colors.text_secondary)
            .children(dot.map(generator::strength_dot))
            .child(line)
            .into_any_element()
    }

    /// A login's websites, each with a button to remove it when there are
    /// several.
    fn render_websites(&self, cx: &mut Context<Self>) -> (Vec<AnyElement>, Vec<FocusHandle>) {
        let several = self.websites.len() > 1;
        let mut focus = vec![];
        let rows: Vec<AnyElement> = self
            .websites
            .iter()
            .map(|row| {
                focus.push(row.input.read(cx).focus_handle(cx));
                let id = row.id;
                let control = div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().flex_1().min_w_0().child(Input::new(&row.input)))
                    .when(several, |d| {
                        d.child(
                            remove_button(("remove-website", id), "Remove Website").on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.remove_website(id, window, cx)
                                }),
                            ),
                        )
                    });
                form_row(IconName::Globe, "Website", control, cx)
            })
            .collect();
        (rows, focus)
    }

    /// The custom fields: their name in place of the label, the value, and
    /// a button to remove it. Linked fields aren't shown, and are kept.
    fn render_fields(&self, cx: &mut Context<Self>) -> Group {
        let mut focus = vec![];
        let rows: Vec<AnyElement> = self
            .fields
            .iter()
            .filter(|row| !matches!(row.value, FieldInput::Linked(_)))
            .map(|row| {
                let id = row.id;
                focus.push(row.name.read(cx).focus_handle(cx));
                let (glyph, value) = match &row.value {
                    FieldInput::Text(input) | FieldInput::Hidden(input) => {
                        focus.push(input.read(cx).focus_handle(cx));
                        let glyph = if matches!(row.value, FieldInput::Hidden(_)) {
                            IconName::Eye
                        } else {
                            IconName::Text
                        };
                        (
                            glyph,
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(input))
                                .into_any_element(),
                        )
                    }
                    FieldInput::Boolean(on) => (
                        IconName::Check,
                        div()
                            .flex_1()
                            .flex()
                            .justify_end()
                            .child(switch(("field-switch", id), *on, "Checked").on_change({
                                let this = cx.entity().downgrade();
                                move |on, _, cx| {
                                    this.update(cx, |this, cx| this.set_checkbox(id, on, cx))
                                        .ok();
                                }
                            }))
                            .into_any_element(),
                    ),
                    FieldInput::Linked(_) => (IconName::Text, div().into_any_element()),
                };
                let control = div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(value)
                    .child(
                        remove_button(("remove-field", id), "Remove Field").on_click(cx.listener(
                            move |this, _, window, cx| this.remove_field(id, window, cx),
                        )),
                    );
                form_row(glyph, Input::new(&row.name), control, cx)
            })
            .collect();
        if rows.is_empty() {
            return Group::empty("fields");
        }
        Group::new("fields", card().on_surface().rows(rows), focus)
    }

    /// The groups an identity has besides its name.
    fn render_identity(&self, cx: &App) -> [Group; 3] {
        if self.kind != ItemKind::Identity {
            return [
                Group::empty("contact"),
                Group::empty("address"),
                Group::empty("documents"),
            ];
        }
        let contact = self.lines(
            "contact",
            &[Key::Email, Key::Phone, Key::IdentityUsername],
            cx,
        );
        let address = if self.shows(Extra::Address, cx) {
            let mut keys = vec![Key::Address1, Key::Address2];
            if !self.value(Key::Address3, cx).is_empty() {
                keys.push(Key::Address3);
            }
            keys.extend([Key::City, Key::State, Key::PostalCode, Key::Country]);
            self.lines("address", &keys, cx)
        } else {
            Group::empty("address")
        };
        let documents = if self.shows(Extra::Identification, cx) {
            self.lines("documents", &[Key::Passport, Key::License, Key::Ssn], cx)
        } else {
            Group::empty("documents")
        };
        [contact, address, documents]
    }

    /// A card holding text of several lines, under its glyph and label.
    fn text_card(
        &self,
        glyph: IconName,
        label: &'static str,
        state: &Entity<TextareaState>,
        cx: &App,
    ) -> AnyElement {
        let dim = cx.theme().colors.text_secondary;
        card()
            .on_surface()
            .row(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .px(px(14.))
                    .py(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .text_color(dim)
                            .child(icon(glyph))
                            .child(label),
                    )
                    .child(
                        div()
                            .line_height(relative(1.35))
                            .child(Textarea::new(state)),
                    ),
            )
            .into_any_element()
    }

    fn render_notes(&self, cx: &App) -> Group {
        Group::new(
            "notes",
            self.text_card(IconName::Note, "Notes", &self.notes, cx),
            vec![self.notes.read(cx).focus_handle(cx)],
        )
    }

    /// "Add Field": the groups not shown yet and the custom fields, each
    /// with its glyph and what it is for.
    fn render_add(&self, cx: &mut Context<Self>) -> Group {
        let this = cx.entity().downgrade();
        let mut menu = popover(
            "add-menu",
            self.add_menu_open,
            &self.add_menu_focus,
            "Add Field",
        )
        .opening(Opening::Above)
        .on_dismiss(move |window, cx| {
            this.update(cx, |this, cx| this.close_add_menu(window, cx))
                .ok();
        });
        let mut parts = 0;
        if self.kind == ItemKind::Login {
            parts += 1;
            menu = menu.child(
                menu_item("add-website", "Website")
                    .icon(IconName::Globe)
                    .detail("Another address for this login")
                    .on_click(cx.listener(|this, _, window, cx| this.add_website_row(window, cx))),
            );
        }
        let extras: Vec<Extra> = EXTRAS
            .into_iter()
            .filter(|extra| extra.kind() == self.kind && !self.shows(*extra, cx))
            .collect();
        for extra in extras {
            parts += 1;
            let (label, detail, glyph) = extra.describe();
            menu = menu.child(
                menu_item(("add-extra", extra as usize), label)
                    .icon(glyph)
                    .detail(detail)
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.add_extra(extra, window, cx)),
                    ),
            );
        }
        if parts > 0 {
            menu = menu.child(menu_separator(cx));
        }
        for (index, (kind, label, detail, glyph)) in NEW_FIELDS.into_iter().enumerate() {
            menu = menu.child(
                menu_item(("add-field", index), label)
                    .icon(glyph)
                    .detail(detail)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.add_custom_field(kind, window, cx)
                    })),
            );
        }
        Group::new(
            "add",
            div()
                .relative()
                .flex()
                .child(
                    button("add-field")
                        .label("Add Field")
                        .icon(IconName::Plus)
                        .trailing_icon(IconName::Down)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.toggle_add_menu(window, cx)),
                        ),
                )
                .child(menu),
            vec![],
        )
    }

    fn render_folder_menu(&self, cx: &mut Context<Self>) -> Popover {
        let choices = std::iter::once((None, "No Folder".to_string())).chain(
            self.folders
                .iter()
                .map(|folder| (Some(folder.id.clone()), folder.name.clone())),
        );
        let this = cx.entity().downgrade();
        popover(
            "folder-menu",
            self.folder_menu_open,
            &self.folder_menu_focus,
            "Folder",
        )
        .on_dismiss(move |window, cx| {
            this.update(cx, |this, cx| this.close_folder_menu(window, cx))
                .ok();
        })
        .children(choices.enumerate().map(|(index, (id, name))| {
            let checked = id == self.folder;
            menu_item(("folder", index), name)
                .checked(checked)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.folder = id.clone();
                    this.close_folder_menu(window, cx);
                }))
        }))
        .child(menu_separator(cx))
        .child(
            menu_item("new-folder", "New Folder…")
                .icon(IconName::Plus)
                .on_click(cx.listener(|this, _, window, cx| this.start_new_folder(window, cx))),
        )
    }

    fn render_folder_row(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(input) = &self.new_folder {
            return div()
                .key_context(NEW_FOLDER_CONTEXT)
                .on_action(cx.listener(Self::cancel_new_folder))
                .child(form_row(IconName::Folder, "Folder", Input::new(input), cx))
                .into_any_element();
        }
        let name = self.folder_name();
        let theme = cx.theme();
        let (hover, faint) = (theme.colors.hover, theme.colors.text_tertiary);
        BaseButton::new("folder")
            .accessibility_label(format!("Folder, {name}"))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_folder_menu(window, cx)))
            .w_full()
            .justify_start()
            .hover(move |s| s.bg(hover))
            .focus_visible(row_focus_ring(cx))
            .child(form_row(
                IconName::Folder,
                "Folder",
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .line_height(relative(1.35))
                    // The menu opens under the value, like a pop-up button's.
                    .child(
                        div()
                            .flex_1()
                            .relative()
                            .child(name)
                            .child(self.render_folder_menu(cx)),
                    )
                    .child(icon(IconName::ChevronsUpDown).text_color(faint)),
                cx,
            ))
            .into_any_element()
    }

    /// Where the item is kept: its folder, and whether it is a favourite.
    fn render_place(&self, cx: &mut Context<Self>) -> Group {
        let favorite = self.favorite;
        Group::new(
            "place",
            card()
                .on_surface()
                .row(self.render_folder_row(cx))
                .row(form_row(
                    IconName::Star,
                    "Favorite",
                    div().flex_1().flex().justify_end().child(
                        switch("favorite", favorite, "Favorite").on_change({
                            let this = cx.entity().downgrade();
                            move |on, _, cx| {
                                this.update(cx, |this, cx| {
                                    this.favorite = on;
                                    cx.notify();
                                })
                                .ok();
                            }
                        }),
                    ),
                    cx,
                )),
            vec![],
        )
    }

    /// The groups, always the same ones in the same order, empty when they
    /// show nothing, so that each keeps its size from one frame to the
    /// next and springs to a new one.
    fn groups(&self, window: &Window, cx: &mut Context<Self>) -> Vec<Group> {
        let mut groups = vec![self.render_main(window, cx)];
        groups.extend(self.render_identity(cx));
        groups.push(self.render_fields(cx));
        groups.push(self.render_notes(cx));
        groups.push(self.render_add(cx));
        groups.push(self.render_place(cx));
        groups
    }

    /// The choice of a new item's kind, as tiles of their colours.
    fn render_kinds(&self, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity().downgrade();
        div()
            .pb(px(14.))
            .child(
                segmented_control(
                    "kind",
                    KINDS.map(|(_, label, ..)| label),
                    kind_index(self.kind),
                )
                .glyphs(KINDS.map(|(_, _, glyph, hue)| (glyph, hue)))
                .on_select(move |index, window, cx| {
                    this.update(cx, |this, cx| this.set_kind(KINDS[index].0, window, cx))
                        .ok();
                }),
            )
            .into_any_element()
    }

    pub(super) fn render_sheet(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let groups = self.groups(window, cx);
        // A field taking the keyboard below the fold brings its group up.
        let focused = groups
            .iter()
            .position(|group| group.focus.iter().any(|focus| focus.is_focused(window)));
        if let Some(index) = focused
            && focused != self.revealed_group
        {
            self.scroll.scroll_to_item(index);
        }
        self.revealed_group = focused;
        let header = self.render_header(window, cx);
        let kinds = self.editing.is_none().then(|| self.render_kinds(cx));
        let deletable = self.editing.as_ref().is_some_and(|item| item.editable);
        let dim = cx.theme().colors.text_secondary;
        div()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::save))
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .children(kinds)
            .child(
                scroll_area("editor-fields")
                    .track_scroll(&self.scroll)
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    // Room at the sides for the focus rings, which the
                    // scrolling would clip.
                    .mx(px(-4.))
                    .px(px(4.))
                    .children(groups.into_iter().map(|group| {
                        let content = group
                            .element
                            .map(|element| div().pb(px(10.)).child(element));
                        div().flex_none().child(motion::grow(
                            ElementId::Name(format!("editor-group-{}", group.id).into()),
                            div().children(content),
                        ))
                    })),
            )
            .when_some(self.reprompt.clone(), |d, reprompt| {
                d.child(motion::enter(
                    "editor-reprompt",
                    div()
                        .mt(px(14.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(
                            div()
                                .mx(px(13.))
                                .text_size(px(12.))
                                .text_color(dim)
                                .child("Enter your master password to save this item."),
                        )
                        .child(reprompt),
                    point(px(0.), px(6.)),
                ))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pt(px(14.))
                    .when(deletable, |d| {
                        d.child(
                            button("delete").label("Delete…").on_click(
                                cx.listener(|this, _, window, cx| this.delete(window, cx)),
                            ),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .px(px(4.))
                            .children(self.problem.map(|problem| field_error(problem, cx))),
                    )
                    .child(button("cancel").label("Cancel").on_click(
                        cx.listener(|this, _, window, cx| this.cancel(&Cancel, window, cx)),
                    ))
                    .child(
                        button("save")
                            .label("Save")
                            .style(ButtonStyle::Primary)
                            .shortcut(crate::secondary!("s"))
                            .busy(self.saving)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save(&Save, window, cx)),
                            ),
                    ),
            )
            .focus_trap("item-editor", &self.focus_handle)
    }
}

fn remove_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
) -> rbw_desktop_ui::Button {
    button(id)
        .icon(IconName::Minus)
        .style(ButtonStyle::Flat)
        .size(px(22.))
        .accessibility_label(label)
}
