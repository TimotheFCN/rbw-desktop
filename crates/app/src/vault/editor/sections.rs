//! How the editor lays out an item: the fields of its kind in the groups
//! the item view shows them in, its notes, "Add Field", then its folder.

use gpui::base::Button as BaseButton;
use gpui::base::FocusTrapElement as _;
use gpui::base::input::{Input, Textarea};
use gpui::{
    AnyElement, App, Context, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, relative,
};
use rbw_desktop_core::ItemKind;
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Opening, Popover, button, card, field_error, form_row,
    icon, menu_item, menu_separator, motion, password_text, popover, row_focus_ring,
    segmented_control, sheet_title, switch, toggle_row,
};

use super::form::Key;
use super::{
    CONTEXT, Cancel, EXTRAS, Extra, FieldInput, ItemEditor, KINDS, NEW_FOLDER_CONTEXT, NewField,
    Save,
};
use crate::generator;

/// A group of rows, with the fields in it that take the keyboard, to bring
/// it into view when one does.
struct Group {
    element: AnyElement,
    focus: Vec<FocusHandle>,
}

impl ItemEditor {
    fn line(&self, label: &'static str, key: Key, cx: &App) -> (AnyElement, FocusHandle) {
        let input = self.input(key);
        (
            form_row(label, Input::new(input), cx),
            input.read(cx).focus_handle(cx),
        )
    }

    /// A card of plain lines.
    fn lines(&self, lines: &[(&'static str, Key)], cx: &App) -> Group {
        let (rows, focus): (Vec<_>, Vec<_>) = lines
            .iter()
            .map(|(label, key)| self.line(label, *key, cx))
            .unzip();
        Group {
            element: card().on_surface().rows(rows).into_any_element(),
            focus,
        }
    }

    /// The name, then the fields that say what the item is.
    fn render_main(&self, window: &Window, cx: &mut Context<Self>) -> Group {
        let mut lines = vec![("Name", Key::Name)];
        match self.kind {
            ItemKind::Login => lines.push(("Username", Key::Username)),
            ItemKind::Card => lines.extend([
                ("Cardholder", Key::Cardholder),
                ("Brand", Key::Brand),
                ("Number", Key::Number),
                ("Expires", Key::Expiry),
                ("Security code", Key::SecurityCode),
            ]),
            ItemKind::Identity => {
                let full = self.shows(Extra::TitleAndMiddleName, cx);
                if full {
                    lines.push(("Title", Key::Title));
                }
                lines.push(("First name", Key::FirstName));
                if full {
                    lines.push(("Middle name", Key::MiddleName));
                }
                lines.push(("Last name", Key::LastName));
            }
            ItemKind::SecureNote | ItemKind::SshKey => {}
        }
        let (mut rows, mut focus): (Vec<_>, Vec<_>) = lines
            .iter()
            .map(|(label, key)| self.line(label, *key, cx))
            .unzip();
        if self.kind == ItemKind::Login {
            rows.push(self.render_password(window, cx));
            focus.push(self.input(Key::Password).read(cx).focus_handle(cx));
            if self.shows(Extra::Authenticator, cx) {
                let (row, handle) = self.line("Setup key", Key::Authenticator, cx);
                rows.push(row);
                focus.push(handle);
            }
            let (websites, handles) = self.render_websites(cx);
            rows.extend(websites);
            focus.extend(handles);
        }
        Group {
            element: card().on_surface().rows(rows).into_any_element(),
            focus,
        }
    }

    /// The password, its digits and symbols coloured while it shows and
    /// isn't being edited, with buttons to show it, make another and change
    /// how it is made. The field stays underneath, so it still takes the
    /// keyboard and the click.
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
            .text_size(px(12.))
            .child(
                div()
                    .when(coloured, |d| d.text_color(gpui::transparent_black()))
                    .child(Input::new(input)),
            )
            .when(coloured, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(password_text(&value, cx)),
                )
            });
        let small = |id: &'static str, name: IconName, label: &'static str| {
            button(id)
                .icon(name)
                .style(ButtonStyle::Flat)
                .size(px(26.))
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
                small("generate", IconName::Refresh, "Generate Password")
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
        form_row(
            "Password",
            div()
                .flex_1()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(field)
                .child(buttons),
            cx,
        )
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
                form_row("Website", control, cx)
            })
            .collect();
        (rows, focus)
    }

    /// The custom fields: a name in place of the label, the value, and a
    /// button to remove it. Linked fields aren't shown, and are kept.
    fn render_fields(&self, cx: &mut Context<Self>) -> Option<Group> {
        let mut focus = vec![];
        let rows: Vec<AnyElement> = self
            .fields
            .iter()
            .filter(|row| !matches!(row.value, FieldInput::Linked(_)))
            .map(|row| {
                let id = row.id;
                focus.push(row.name.read(cx).focus_handle(cx));
                let value = match &row.value {
                    FieldInput::Text(input) | FieldInput::Hidden(input) => {
                        focus.push(input.read(cx).focus_handle(cx));
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(input))
                            .into_any_element()
                    }
                    FieldInput::Boolean(on) => div()
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
                    FieldInput::Linked(_) => div().into_any_element(),
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
                form_row(Input::new(&row.name), control, cx)
            })
            .collect();
        (!rows.is_empty()).then(|| Group {
            element: card().on_surface().rows(rows).into_any_element(),
            focus,
        })
    }

    /// The groups an identity has besides its name.
    fn render_identity(&self, cx: &App) -> Vec<Group> {
        let mut groups = vec![self.lines(
            &[
                ("Email", Key::Email),
                ("Phone", Key::Phone),
                ("Username", Key::IdentityUsername),
            ],
            cx,
        )];
        if self.shows(Extra::Address, cx) {
            let mut lines = vec![("Address", Key::Address1), ("Address 2", Key::Address2)];
            if !self.value(Key::Address3, cx).is_empty() {
                lines.push(("Address 3", Key::Address3));
            }
            lines.extend([
                ("City", Key::City),
                ("State", Key::State),
                ("Postal code", Key::PostalCode),
                ("Country", Key::Country),
            ]);
            groups.push(self.lines(&lines, cx));
        }
        if self.shows(Extra::Identification, cx) {
            groups.push(self.lines(
                &[
                    ("Passport", Key::Passport),
                    ("License", Key::License),
                    ("SSN", Key::Ssn),
                ],
                cx,
            ));
        }
        groups
    }

    fn render_notes(&self, cx: &App) -> Group {
        Group {
            element: card()
                .on_surface()
                .row(
                    div()
                        .px(px(13.))
                        .py(px(9.))
                        .line_height(relative(1.35))
                        .child(Textarea::new(&self.notes)),
                )
                .into_any_element(),
            focus: vec![self.notes.read(cx).focus_handle(cx)],
        }
    }

    /// "Add Field", with the groups not shown yet and the custom fields.
    fn render_add(&self, cx: &mut Context<Self>) -> AnyElement {
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
        if self.kind == ItemKind::Login {
            menu = menu.child(
                menu_item("add-website", "Website")
                    .on_click(cx.listener(|this, _, window, cx| this.add_website_row(window, cx))),
            );
        }
        let extras: Vec<Extra> = EXTRAS
            .into_iter()
            .filter(|extra| extra.kind() == self.kind && !self.shows(*extra, cx))
            .collect();
        for extra in extras {
            menu = menu.child(
                menu_item(("add-extra", extra as usize), extra.label()).on_click(
                    cx.listener(move |this, _, window, cx| this.add_extra(extra, window, cx)),
                ),
            );
        }
        if matches!(self.kind, ItemKind::Login | ItemKind::Identity) {
            menu = menu.child(menu_separator(cx));
        }
        for (index, (kind, label)) in [
            (NewField::Text, "Text Field"),
            (NewField::Hidden, "Hidden Field"),
            (NewField::Boolean, "Checkbox"),
        ]
        .into_iter()
        .enumerate()
        {
            menu = menu.child(menu_item(("add-field", index), label).on_click(
                cx.listener(move |this, _, window, cx| this.add_custom_field(kind, window, cx)),
            ));
        }
        div()
            .relative()
            .flex()
            .child(
                button("add-field")
                    .label("Add Field")
                    .icon(IconName::Plus)
                    .style(ButtonStyle::Link)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_add_menu(window, cx))),
            )
            .child(menu)
            .into_any_element()
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
                .on_click(cx.listener(|this, _, window, cx| this.start_new_folder(window, cx))),
        )
    }

    fn render_folder_row(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(input) = &self.new_folder {
            return div()
                .key_context(NEW_FOLDER_CONTEXT)
                .on_action(cx.listener(Self::cancel_new_folder))
                .child(form_row("Folder", Input::new(input), cx))
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

    /// How strong the generated password is, under the fields.
    fn render_generated_note(&self, cx: &App) -> AnyElement {
        let generator = self.generator.read(cx);
        let noun = generator.noun();
        let (dot, note) = match generator.strength() {
            Some(strength) => {
                let (color, word) = generator::strength_look(strength, cx);
                (Some(color), format!("{word} {noun}, generated for you"))
            }
            None => (None, format!("{noun}, generated for you")),
        };
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .mx(px(13.))
            .text_size(px(12.))
            .text_color(cx.theme().colors.text_secondary)
            .children(dot.map(generator::strength_dot))
            .child(note)
            .into_any_element()
    }

    /// The groups of fields, in the order they show.
    fn groups(&self, window: &Window, cx: &mut Context<Self>) -> Vec<Group> {
        let main = self.render_main(window, cx);
        // Another kind shows other fields: they fade up in place.
        let mut groups = vec![Group {
            element: motion::refresh(("item-kind", self.kind as usize), div().child(main.element))
                .into_any_element(),
            focus: main.focus,
        }];
        if self.kind == ItemKind::Login && self.generated {
            groups.push(Group {
                element: self.render_generated_note(cx),
                focus: vec![],
            });
        }
        if self.kind == ItemKind::Identity {
            groups.extend(self.render_identity(cx));
        }
        groups.extend(self.render_fields(cx));
        groups.push(self.render_notes(cx));
        groups.push(Group {
            element: self.render_add(cx),
            focus: vec![],
        });
        let favorite = self.favorite;
        groups.push(Group {
            element: card()
                .on_surface()
                .row(self.render_folder_row(cx))
                .row(toggle_row(
                    "Add to Favorites",
                    switch("favorite", favorite, "Add to Favorites").on_change({
                        let this = cx.entity().downgrade();
                        move |on, _, cx| {
                            this.update(cx, |this, cx| {
                                this.favorite = on;
                                cx.notify();
                            })
                            .ok();
                        }
                    }),
                ))
                .into_any_element(),
            focus: vec![],
        });
        groups
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
        let title = if self.editing.is_some() {
            "Edit Item"
        } else {
            "New Item"
        };
        let selected_kind = KINDS
            .iter()
            .position(|(kind, _)| *kind == self.kind)
            .unwrap_or_default();
        let deletable = self.editing.as_ref().is_some_and(|item| item.editable);
        let dim = cx.theme().colors.text_secondary;
        div()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::save))
            .min_h_0()
            .flex()
            .flex_col()
            .child(sheet_title(title))
            .when(self.editing.is_none(), |d| {
                d.child(
                    div().pb(px(12.)).child(
                        segmented_control("kind", KINDS.map(|(_, label)| label), selected_kind)
                            .on_select({
                                let this = cx.entity().downgrade();
                                move |index, _, cx| {
                                    this.update(cx, |this, cx| this.set_kind(KINDS[index].0, cx))
                                        .ok();
                                }
                            }),
                    ),
                )
            })
            .child(
                div()
                    .id("editor-fields")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .children(
                        groups
                            .into_iter()
                            .map(|group| div().flex_none().child(group.element)),
                    ),
            )
            .when_some(self.reprompt.clone(), |d, reprompt| {
                d.child(
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
                )
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
                            .children(self.problem.map(|problem| field_error(problem, cx))),
                    )
                    .child(button("cancel").label("Cancel").on_click(
                        cx.listener(|this, _, window, cx| this.cancel(&Cancel, window, cx)),
                    ))
                    .child(
                        button("save")
                            .label("Save")
                            .style(ButtonStyle::Primary)
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
