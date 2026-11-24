//! An item's menu in the list, from a right-click or from the keyboard
//! (Shift-F10 or the Menu key): copy its username, password or code, make
//! it a favourite, move it to a folder, edit or delete it. Moving lists the
//! folders in the same menu, with a way back, as GNOME's menus do.

use gpui::{
    AnyElement, Context, IntoElement, ParentElement as _, Pixels, Point, Styled as _, Window, div,
    point, px,
};
use rbw_desktop_core::{
    Error, FolderId, Item, ItemContent, ItemId, ItemKind, ItemSummary, Result, Secret,
};
use rbw_desktop_ui::{IconName, Opening, menu_item, menu_separator, popover};

use super::{OpenContextMenu, VaultView};
use crate::backend;
use crate::clipboard;
use crate::recent;

/// The height of a row of the list, with the gap under it.
const LIST_ROW: Pixels = px(47.);

pub(super) struct ContextMenu {
    item: ItemSummary,
    position: Point<Pixels>,
    open: bool,
    /// Whether it lists the folders to move the item to.
    folders: bool,
}

/// What can be copied from the menu.
#[derive(Clone, Copy)]
enum Copy {
    Username,
    Password,
    Code,
}

impl VaultView {
    pub(super) fn open_context_menu(
        &mut self,
        id: ItemId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.summary(&id).cloned() else {
            return;
        };
        self.select(Some(id), cx);
        self.context_menu = Some(ContextMenu {
            item,
            position,
            open: true,
            folders: false,
        });
        window.focus(&self.context_menu_focus, cx);
        cx.notify();
    }

    /// Opens the selected item's menu over its row.
    pub(super) fn open_context_menu_from_keyboard(
        &mut self,
        _: &OpenContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(index), Some(id)) = (self.selected_index(), self.selected.clone()) else {
            return;
        };
        let position = {
            let list = self.list_scroll.0.borrow();
            let bounds = list.base_handle.bounds();
            point(
                bounds.origin.x + px(40.),
                bounds.origin.y + list.base_handle.offset().y + LIST_ROW * (index as f32 + 0.75),
            )
        };
        self.open_context_menu(id, position, window, cx);
    }

    fn close_context_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.context_menu {
            menu.open = false;
        }
        window.focus(&self.list_focus, cx);
        cx.notify();
    }

    /// The item with its secrets, for `then`. An item that asks for the
    /// master password again shows its prompt instead.
    fn with_item(
        &mut self,
        id: ItemId,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, Item, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let account = self.account_id();
        let requested = id.clone();
        backend::request_in(
            window,
            cx,
            move |backend| backend.item(&account, &requested),
            move |this, result, window, cx| match result {
                Ok(item) => then(this, item, window, cx),
                Err(Error::RepromptRequired) => this.ask_password_for(id, cx),
                Err(error) => this.show_error(&error, cx),
            },
        )
        .detach();
    }

    /// Shows the item, with the keyboard in its master password prompt.
    fn ask_password_for(&mut self, id: ItemId, cx: &mut Context<Self>) {
        self.select_by(Some(id), true, cx);
    }

    fn copy(&mut self, what: Copy, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &self.context_menu else {
            return;
        };
        let id = menu.item.id.clone();
        self.close_context_menu(window, cx);
        if let Copy::Code = what {
            let account = self.account_id();
            let requested = id.clone();
            backend::request(
                cx,
                move |backend| backend.verification_code(&account, &requested),
                move |this, result, cx| match result {
                    Ok(code) => this.copied(&id, &code.code, "Verification code copied", cx),
                    Err(Error::RepromptRequired) => this.ask_password_for(id, cx),
                    Err(error) => this.show_error(&error, cx),
                },
            )
            .detach();
            return;
        }
        self.with_item(id, window, cx, move |this, item, _, cx| {
            let ItemContent::Login(login) = &item.data.content else {
                return;
            };
            match what {
                Copy::Username => this.copied(
                    &item.id,
                    &Secret::new(login.username.clone()),
                    "Username copied",
                    cx,
                ),
                Copy::Password => this.copied(&item.id, &login.password, "Password copied", cx),
                Copy::Code => {}
            }
        });
    }

    fn copied(
        &mut self,
        item: &ItemId,
        value: &Secret,
        message: &'static str,
        cx: &mut Context<Self>,
    ) {
        clipboard::copy(value, true, cx);
        recent::used(&self.account.id, item, cx);
        self.toast
            .show(message, |this: &mut Self| &mut this.toast, cx);
    }

    fn toggle_favorite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &self.context_menu else {
            return;
        };
        let (account, id, favorite) =
            (self.account_id(), menu.item.id.clone(), !menu.item.favorite);
        self.close_context_menu(window, cx);
        backend::request(
            cx,
            move |backend| backend.set_favorite(&account, &id, favorite),
            |this, result, cx| this.changed(result, cx),
        )
        .detach();
    }

    fn move_to(&mut self, folder: Option<FolderId>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &self.context_menu else {
            return;
        };
        let (account, id) = (self.account_id(), menu.item.id.clone());
        let unchanged = menu.item.folder == folder;
        self.close_context_menu(window, cx);
        if unchanged {
            return;
        }
        backend::request(
            cx,
            move |backend| backend.move_item(&account, &id, folder.as_ref()),
            |this, result, cx| this.changed(result, cx),
        )
        .detach();
    }

    /// After a change to an item that the list shows, such as a new folder.
    fn changed(&mut self, result: Result<()>, cx: &mut Context<Self>) {
        match result {
            Ok(()) => {
                self.reload(cx);
                self.detail.update(cx, |detail, cx| detail.reload(cx));
            }
            Err(error) => self.show_error(&error, cx),
        }
    }

    fn edit_from_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &self.context_menu else {
            return;
        };
        let id = menu.item.id.clone();
        self.close_context_menu(window, cx);
        self.with_item(id, window, cx, |this, item, window, cx| {
            this.open_editor(item, window, cx)
        });
    }

    fn delete_from_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &self.context_menu else {
            return;
        };
        let (id, name) = (menu.item.id.clone(), menu.item.name.clone());
        self.close_context_menu(window, cx);
        self.delete_item(id, name, window, cx);
    }

    /// Swaps the menu's items for the folders, or back. The item that had
    /// the keyboard goes, so the menu takes it, and the arrow keys start
    /// from its top again.
    fn show_folders(&mut self, show: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.context_menu {
            menu.folders = show;
            window.focus(&self.context_menu_focus, cx);
            cx.notify();
        }
    }

    pub(super) fn render_context_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.context_menu.as_ref()?;
        let item = &menu.item;
        let this = cx.entity().downgrade();
        let popover = popover(
            "item-menu",
            menu.open,
            &self.context_menu_focus,
            item.name.clone(),
        )
        .opening(Opening::At(menu.position))
        .on_dismiss(move |window, cx| {
            this.update(cx, |this, cx| this.close_context_menu(window, cx))
                .ok();
        });
        let rows: Vec<AnyElement> =
            if menu.folders {
                let folders = std::iter::once((None, "No Folder".to_string())).chain(
                    self.folders
                        .iter()
                        .map(|folder| (Some(folder.id.clone()), folder.name.clone())),
                );
                let mut rows = vec![
                    menu_item("back", "Move to Folder")
                        .icon(IconName::Back)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.show_folders(false, window, cx)),
                        )
                        .into_any_element(),
                    menu_separator(cx).into_any_element(),
                ];
                rows.extend(folders.enumerate().map(|(index, (id, name))| {
                    let checked = item.folder == id;
                    menu_item(("folder", index), name)
                        .checked(checked)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.move_to(id.clone(), window, cx)
                        }))
                        .into_any_element()
                }));
                rows
            } else {
                let login = item.kind == ItemKind::Login;
                let mut rows = vec![];
                if login && item.subtitle.is_some() {
                    rows.push(
                        menu_item("copy-username", "Copy Username")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.copy(Copy::Username, window, cx)
                            }))
                            .into_any_element(),
                    );
                }
                if login {
                    rows.push(
                        menu_item("copy-password", "Copy Password")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.copy(Copy::Password, window, cx)
                            }))
                            .into_any_element(),
                    );
                }
                if item.has_code {
                    rows.push(
                        menu_item("copy-code", "Copy Verification Code")
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.copy(Copy::Code, window, cx)
                                }),
                            )
                            .into_any_element(),
                    );
                }
                if !rows.is_empty() {
                    rows.push(menu_separator(cx).into_any_element());
                }
                rows.push(
                    menu_item(
                        "favorite",
                        if item.favorite {
                            "Remove from Favorites"
                        } else {
                            "Add to Favorites"
                        },
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_favorite(window, cx)))
                    .into_any_element(),
                );
                rows.push(
                    menu_item("move", "Move to Folder")
                        .submenu()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.show_folders(true, window, cx)),
                        )
                        .into_any_element(),
                );
                if item.editable {
                    rows.push(menu_separator(cx).into_any_element());
                    rows.push(
                        menu_item("edit", "Edit")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.edit_from_menu(window, cx)),
                            )
                            .into_any_element(),
                    );
                    rows.push(
                        menu_item("delete", "Delete…")
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.delete_from_menu(window, cx)
                                }),
                            )
                            .into_any_element(),
                    );
                }
                rows
            };
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .child(popover.children(rows))
                .into_any_element(),
        )
    }
}
