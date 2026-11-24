//! The app's record of each account's recently used items, which the menu
//! bar panel lists: an item counts as used when something of it is copied,
//! when it is opened with a click, its website opened, or it is saved.

use gpui::App;
use rbw_desktop_core::{AccountId, ItemId};

use crate::backend;

pub fn used(account: &AccountId, item: &ItemId, cx: &mut App) {
    let (account, item) = (account.clone(), item.clone());
    backend::forget(cx, move |backend| backend.item_used(&account, &item));
}
