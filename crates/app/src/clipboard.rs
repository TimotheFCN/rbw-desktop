//! Copying to the clipboard. A copied secret is cleared again after the
//! time the preferences allow, when the account it came from locks or signs
//! out, and when the app quits, unless something else was copied since.

use std::time::Duration;

use gpui::{App, ClipboardItem, Global};
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

fn clear(reason: Reason, cx: &mut App) {
    let current = cx.read_from_clipboard().and_then(|item| item.text());
    if cx
        .global_mut::<Clipboard>()
        .pending
        .clears(reason, current.as_deref())
    {
        cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
    }
}

/// Why a secret may go.
#[derive(Clone, Copy, Debug)]
enum Reason<'a> {
    /// The time for the copy of this turn is up.
    Expired(u64),
    Closed(&'a AccountId),
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
            Reason::Quit => true,
        };
        if !concerned {
            return false;
        }
        let held = current == Some(copied.value.expose());
        self.copied = None;
        held
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
