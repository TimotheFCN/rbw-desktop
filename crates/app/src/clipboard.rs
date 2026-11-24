//! Copying to the clipboard. A copied secret is cleared again after the
//! time the preferences allow, unless something else was copied since.

use std::time::Duration;

use gpui::{App, ClipboardItem, Global};
use rbw_desktop_core::{Preferences, Secret};

/// How long a copied secret stays on the clipboard; `None` leaves it there.
pub struct ClearAfter(pub Option<Duration>);

impl Global for ClearAfter {}

/// Keeps copied secrets for the default time until the preferences arrive
/// ([`crate::preferences`]).
pub fn init(cx: &mut App) {
    cx.set_global(ClearAfter(Preferences::default().clipboard_clear));
}

pub fn copy(value: &Secret, secret: bool, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(value.expose().to_string()));
    let Some(delay) = secret.then(|| cx.global::<ClearAfter>().0).flatten() else {
        return;
    };
    let copied = value.clone();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(delay).await;
        cx.update(|cx| {
            let unchanged = cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .is_some_and(|text| text == copied.expose());
            if unchanged {
                cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
            }
        });
    })
    .detach();
}
