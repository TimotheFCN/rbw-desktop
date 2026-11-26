//! An item's menu in the list, from a right-click or from the keyboard
//! (Shift-F10 or the Menu key): copy its username, password or code, make
//! it a favourite, move it to a folder, from a submenu, edit or delete it.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, IntoElement, ParentElement as _, Pixels, Point, Styled as _, Window, div,
    point, prelude::FluentBuilder as _, px,
};
use rbw_desktop_core::{Error, FolderId, Item, ItemContent, ItemId, ItemKind, ItemSummary, Result};
use rbw_desktop_ui::{Opening, Popover, ROW_HEIGHT, menu_item, menu_separator, motion, popover};

use super::list::ROW_GAP;
use super::{OpenContextMenu, Part, VaultView};
use crate::backend;
use crate::clipboard;
use crate::recent;

pub(super) struct ContextMenu {
    item: Arc<ItemSummary>,
    position: Point<Pixels>,
    open: bool,
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
                bounds.origin.y
                    + list.base_handle.offset().y
                    + (ROW_HEIGHT + ROW_GAP) * (index as f32 + 0.75),
            )
        };
        self.open_context_menu(id, position, window, cx);
    }

    /// Closes the menu, which goes, with its item, once it has left.
    fn close_context_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.context_menu {
            menu.open = false;
        }
        self.context_menu_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(motion::DISMISS).await;
            this.update(cx, |this, cx| {
                if this.context_menu.as_ref().is_some_and(|menu| !menu.open) {
                    this.context_menu = None;
                    cx.notify();
                }
            })
            .ok();
        });
        window.focus(&self.list_focus, cx);
        cx.notify();
    }

    /// The item with its secrets, for `then`. An item that asks for the
    /// master password again shows its prompt instead. A newer request
    /// replaces it.
    pub(super) fn with_item(
        &mut self,
        id: ItemId,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, Item, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let account = self.account_id();
        let requested = id.clone();
        self.menu_task = backend::request_in(
            window,
            cx,
            move |backend| backend.item(&account, &requested),
            move |this, result, window, cx| match result {
                Ok(item) => then(this, item, window, cx),
                Err(Error::RepromptRequired) => this.ask_password_for(id, cx),
                Err(error) => this.show_error(&error, cx),
            },
        );
    }

    /// Shows the item, with the keyboard in its master password prompt.
    pub(super) fn ask_password_for(&mut self, id: ItemId, cx: &mut Context<Self>) {
        self.select_by(Some(id), true, cx);
    }

    fn copy(&mut self, what: Part, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &self.context_menu else {
            return;
        };
        let id = menu.item.id.clone();
        self.close_context_menu(window, cx);
        if let Part::Code = what {
            let account = self.account_id();
            let requested = id.clone();
            // A newer request from the menu replaces it, as with_item's.
            self.menu_task = backend::request(
                cx,
                move |backend| backend.verification_code(&account, &requested),
                move |this, result, cx| match result {
                    Ok(code) => {
                        clipboard::copy_secret(&code.code, Some(&this.account.id), cx);
                        this.copied(&id, cx);
                    }
                    Err(Error::RepromptRequired) => this.ask_password_for(id, cx),
                    Err(error) => this.show_error(&error, cx),
                },
            );
            return;
        }
        self.with_item(id, window, cx, move |this, item, _, cx| {
            let ItemContent::Login(login) = &item.data.content else {
                return;
            };
            match what {
                Part::Username => {
                    clipboard::copy_text(&login.username, cx);
                    this.copied(&item.id, cx);
                }
                Part::Password => {
                    clipboard::copy_secret(&login.password, Some(&this.account.id), cx);
                    this.copied(&item.id, cx);
                }
                Part::Code => {}
            }
        });
    }

    /// Something of an item was copied: its row says so for a moment.
    pub(super) fn copied(&mut self, item: &ItemId, cx: &mut Context<Self>) {
        recent::used(&self.account.id, item, cx);
        self.copied.set(item.clone(), cx, |this| &mut this.copied);
    }

    fn toggle_favorite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &self.context_menu else {
            return;
        };
        let (account, id, favorite) =
            (self.account_id(), menu.item.id.clone(), !menu.item.favorite);
        self.close_context_menu(window, cx);
        backend::request_network(
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
        backend::request_network(
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
                crate::vault_cache::invalidate(&self.account.id, cx);
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
        self.fetch_and_edit(id, window, cx);
    }

    fn delete_from_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &self.context_menu else {
            return;
        };
        let (id, name) = (menu.item.id.clone(), menu.item.name.clone());
        self.close_context_menu(window, cx);
        self.delete_item(id, name, window, cx);
    }

    pub(super) fn render_context_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.context_menu.as_ref()?;
        let (this, view) = (cx.entity().downgrade(), cx.entity());
        let popover = popover(
            "item-menu",
            menu.open,
            &self.context_menu_focus,
            menu.item.name.clone(),
        )
        .opening(Opening::At(menu.position))
        .on_dismiss(move |window, cx| {
            this.update(cx, |this, cx| this.close_context_menu(window, cx))
                .ok();
        })
        .when_shown(move |menu, _, cx| {
            view.update(cx, |this, cx| this.context_menu_items(menu, cx))
        });
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .child(popover)
                .into_any_element(),
        )
    }

    /// The menu's items, made only while it shows.
    fn context_menu_items(&mut self, popover: Popover, cx: &mut Context<Self>) -> Popover {
        let Some(item) = self.context_menu.as_ref().map(|menu| menu.item.clone()) else {
            return popover;
        };
        let folders: Vec<AnyElement> =
            std::iter::once((None, "No Folder".to_string()))
                .chain(
                    self.folders
                        .iter()
                        .map(|folder| (Some(folder.id.clone()), folder.name.clone())),
                )
                .enumerate()
                .map(|(index, (id, name))| {
                    let checked = item.folder == id;
                    menu_item(("folder", index), name)
                        .checked(checked)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.move_to(id.clone(), window, cx)
                        }))
                        .into_any_element()
                })
                .collect();
        let login = item.kind == ItemKind::Login;
        let mut copies = vec![];
        if login && item.subtitle.is_some() {
            copies.push(menu_item("copy-username", "Copy Username").on_click(
                cx.listener(|this, _, window, cx| this.copy(Part::Username, window, cx)),
            ));
        }
        if login {
            copies.push(menu_item("copy-password", "Copy Password").on_click(
                cx.listener(|this, _, window, cx| this.copy(Part::Password, window, cx)),
            ));
        }
        if item.has_code {
            copies
                .push(menu_item("copy-code", "Copy Verification Code").on_click(
                    cx.listener(|this, _, window, cx| this.copy(Part::Code, window, cx)),
                ));
        }
        let has_copies = !copies.is_empty();
        popover
            .children(copies)
            .when(has_copies, |menu| menu.child(menu_separator(cx)))
            .child(
                menu_item(
                    "favorite",
                    if item.favorite {
                        "Remove from Favorites"
                    } else {
                        "Add to Favorites"
                    },
                )
                .on_click(cx.listener(|this, _, window, cx| this.toggle_favorite(window, cx))),
            )
            .submenu(menu_item("move", "Move to Folder"), folders)
            .when(item.editable, |menu| {
                menu.child(menu_separator(cx))
                    .child(menu_item("edit", "Edit").on_click(
                        cx.listener(|this, _, window, cx| this.edit_from_menu(window, cx)),
                    ))
                    .child(menu_item("delete", "Delete…").on_click(
                        cx.listener(|this, _, window, cx| this.delete_from_menu(window, cx)),
                    ))
            })
    }
}
