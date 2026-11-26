//! What a view just copied, which says "Copied" for a moment
//! ([`motion::CONFIRMATION_HOLD`]) and then goes back to how it was.

use gpui::{Context, Task};
use rbw_desktop_ui::motion;

/// The thing just copied in a view, such as a field or a key, until its
/// moment is over. Copying another starts the moment again.
pub struct CopiedSlot<T> {
    copied: Option<T>,
    timer: Task<()>,
}

impl<T> Default for CopiedSlot<T> {
    fn default() -> Self {
        Self {
            copied: None,
            timer: Task::ready(()),
        }
    }
}

impl<T: PartialEq + Clone + 'static> CopiedSlot<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `what` was just copied.
    pub fn is(&self, what: &T) -> bool {
        self.copied.as_ref() == Some(what)
    }

    /// Notes that `what` was copied, and redraws the view, and again once
    /// the moment is over. `slot` finds this slot in the view.
    pub fn set<V: 'static>(
        &mut self,
        what: T,
        cx: &mut Context<V>,
        slot: fn(&mut V) -> &mut CopiedSlot<T>,
    ) {
        self.copied = Some(what.clone());
        self.timer = cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(motion::CONFIRMATION_HOLD)
                .await;
            view.update(cx, |view, cx| {
                let slot = slot(view);
                if slot.copied.as_ref() == Some(&what) {
                    slot.copied = None;
                    cx.notify();
                }
            })
            .ok();
        });
        cx.notify();
    }

    /// Ends the moment now, as when the value copied is replaced.
    pub fn clear(&mut self) {
        self.copied = None;
        self.timer = Task::ready(());
    }
}
