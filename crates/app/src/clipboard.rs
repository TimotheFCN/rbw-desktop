//! Copying to the clipboard. A copied secret is cleared again after the
//! time the preferences allow, when the account it came from locks or signs
//! out, once no account is unlocked, and when the app quits, unless
//! something else was copied since. On macOS a secret is marked concealed
//! and transient, so that clipboard managers and Universal Clipboard leave
//! it alone.

use std::time::Duration;

use gpui::{App, ClipboardEntry, ClipboardItem, Global};
use rbw_desktop_core::{AccountId, Preferences, Secret};

struct Clipboard {
    /// How long a copied secret stays; `None` leaves it there.
    clear_after: Option<Duration>,
    pending: Pending,
}

impl Global for Clipboard {}

/// Keeps copied secrets for the default time until the preferences arrive
/// ([`crate::preferences`]).
pub fn init(cx: &mut App) {
    cx.set_global(Clipboard {
        clear_after: Preferences::default().clipboard_clear,
        pending: Pending::default(),
    });
    cx.on_app_quit(|cx| {
        clear(Reason::Quit, cx);
        async {}
    })
    .detach();
}

pub fn set_clear_after(clear_after: Option<Duration>, cx: &mut App) {
    cx.global_mut::<Clipboard>().clear_after = clear_after;
}

/// Copies something that isn't secret, such as a username: it stays.
pub fn copy_text(text: &str, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
    cx.global_mut::<Clipboard>().pending.replaced();
}

/// Copies a secret, which goes again after a while, and when `account`
/// locks or signs out.
pub fn copy_secret(value: &Secret, account: Option<&AccountId>, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(value.expose().to_string()));
    #[cfg(target_os = "macos")]
    macos::conceal();
    let clipboard = cx.global_mut::<Clipboard>();
    let turn = clipboard.pending.copied(value, account);
    let Some(delay) = clipboard.clear_after else {
        return;
    };
    cx.spawn(async move |cx| {
        cx.background_executor().timer(delay).await;
        cx.update(|cx| clear(Reason::Expired(turn), cx));
    })
    .detach();
}

/// The account locked or signed out: a secret copied from it goes.
pub fn account_closed(account: &AccountId, cx: &mut App) {
    clear(Reason::Closed(account), cx);
}

/// No account is unlocked any more: any secret goes, also one that came
/// from no account, such as a generated password.
pub fn all_closed(cx: &mut App) {
    clear(Reason::AllClosed, cx);
}

/// Whether a secret the app copied may still be on the clipboard, waiting
/// to go.
pub fn has_pending_secret(cx: &App) -> bool {
    cx.try_global::<Clipboard>()
        .is_some_and(|clipboard| clipboard.pending.copied.is_some())
}

fn clear(reason: Reason, cx: &mut App) {
    let Some(clipboard) = cx.try_global::<Clipboard>() else {
        return;
    };
    // Nothing to read when no secret waits.
    if clipboard.pending.copied.is_none() {
        return;
    }
    let current = cx.read_from_clipboard().and_then(text);
    if cx
        .global_mut::<Clipboard>()
        .pending
        .clears(reason, current.as_ref().map(Secret::expose))
    {
        cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
    }
}

/// The text on the clipboard, which may be the secret, kept so that it is
/// wiped once compared: its parts are moved, not copied, and joined in a
/// string that never grows.
fn text(item: ClipboardItem) -> Option<Secret> {
    let parts: Vec<Secret> = item
        .into_entries()
        .filter_map(|entry| match entry {
            ClipboardEntry::String(string) => Some(Secret::new(string.text)),
            _ => None,
        })
        .collect();
    match parts.len() {
        0 => None,
        1 => parts.into_iter().next(),
        _ => {
            let mut joined = String::with_capacity(parts.iter().map(|p| p.expose().len()).sum());
            for part in &parts {
                joined.push_str(part.expose());
            }
            Some(Secret::new(joined))
        }
    }
}

/// Why a secret may go.
#[derive(Clone, Copy, Debug)]
enum Reason<'a> {
    /// The time for the copy of this turn is up.
    Expired(u64),
    Closed(&'a AccountId),
    /// No account is unlocked any more.
    AllClosed,
    Quit,
}

/// The secret the app put on the clipboard last, until it goes.
#[derive(Default)]
struct Pending {
    copied: Option<Copied>,
    /// Counts copies, so that only the latest one's time clears it.
    turns: u64,
}

struct Copied {
    value: Secret,
    account: Option<AccountId>,
    turn: u64,
}

impl Pending {
    /// A secret was copied; its turn says when its time is up.
    fn copied(&mut self, value: &Secret, account: Option<&AccountId>) -> u64 {
        self.turns += 1;
        self.copied = Some(Copied {
            value: value.clone(),
            account: account.cloned(),
            turn: self.turns,
        });
        self.turns
    }

    /// Something else was copied in its place.
    fn replaced(&mut self) {
        self.copied = None;
    }

    /// Whether the clipboard, holding `current`, is to be emptied: it still
    /// holds the secret, and `reason` concerns it. Either way a secret that
    /// `reason` concerns is forgotten.
    fn clears(&mut self, reason: Reason, current: Option<&str>) -> bool {
        let Some(copied) = &self.copied else {
            return false;
        };
        let concerned = match reason {
            Reason::Expired(turn) => copied.turn == turn,
            Reason::Closed(account) => copied.account.as_ref() == Some(account),
            Reason::AllClosed | Reason::Quit => true,
        };
        if !concerned {
            return false;
        }
        let held = current == Some(copied.value.expose());
        self.copied = None;
        held
    }
}

/// Marks what is on the pasteboard as a secret, by the markers of
/// nspasteboard.org: clipboard managers don't record it, and it doesn't go
/// to the person's other devices.
#[cfg(target_os = "macos")]
mod macos {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::{NSData, NSString};

    const MARKERS: [&str; 2] = [
        "org.nspasteboard.ConcealedType",
        "org.nspasteboard.TransientType",
    ];

    /// Adds the markers to what GPUI just wrote, which is on the general
    /// pasteboard's first item.
    pub fn conceal() {
        let pasteboard = NSPasteboard::generalPasteboard();
        let empty = NSData::new();
        for marker in MARKERS {
            let marker = NSString::from_str(marker);
            if !pasteboard.setData_forType(Some(&empty), &marker) {
                log::info!("marking a copied secret as concealed failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copying_again_waits_for_the_new_time() {
        let mut pending = Pending::default();
        let first = pending.copied(&"hunter2".into(), None);
        let second = pending.copied(&"hunter2".into(), None);
        assert!(!pending.clears(Reason::Expired(first), Some("hunter2")));
        assert!(pending.clears(Reason::Expired(second), Some("hunter2")));
    }

    #[test]
    fn locking_clears_what_the_account_copied() {
        let (personal, work) = (AccountId::new("personal"), AccountId::new("work"));
        let mut pending = Pending::default();
        pending.copied(&"hunter2".into(), Some(&personal));
        assert!(!pending.clears(Reason::Closed(&work), Some("hunter2")));
        assert!(pending.clears(Reason::Closed(&personal), Some("hunter2")));
        assert!(!pending.clears(Reason::Quit, Some("hunter2")));
    }

    #[test]
    fn the_last_lock_clears_a_generated_secret() {
        let mut pending = Pending::default();
        pending.copied(&"hunter2".into(), None);
        assert!(!pending.clears(Reason::Closed(&AccountId::new("work")), Some("hunter2")));
        assert!(pending.clears(Reason::AllClosed, Some("hunter2")));
    }

    #[test]
    fn the_clipboard_text_is_its_strings_joined() {
        let item = ClipboardItem::new_string("hunter2".into());
        assert_eq!(text(item).as_ref().map(Secret::expose), Some("hunter2"));
    }

    #[test]
    fn quitting_clears_a_secret() {
        let mut pending = Pending::default();
        pending.copied(&"hunter2".into(), None);
        assert!(pending.clears(Reason::Quit, Some("hunter2")));
    }

    #[test]
    fn what_was_copied_since_stays() {
        let mut pending = Pending::default();
        let turn = pending.copied(&"hunter2".into(), None);
        assert!(!pending.clears(Reason::Expired(turn), Some("something else")));
        pending.copied(&"hunter2".into(), None);
        pending.replaced();
        assert!(!pending.clears(Reason::Quit, Some("hunter2")));
    }
}
