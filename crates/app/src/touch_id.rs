//! When the lock screen asks for Touch ID by itself. Touch ID can't be read
//! without the system's prompt, so for a finger on the sensor to unlock,
//! the lock screen starts that prompt as soon as it shows in the window the
//! person is using, as other password managers do. It never starts it over
//! another app, as when the lock timeout locks a window in the background,
//! nor right after the person locked the account here, nor again after
//! they canceled it, until the window comes forward again.

/// Whether the lock screen may start a Touch ID unlock by itself. Each lock
/// screen has its own, which a new account shown starts afresh.
#[derive(Debug)]
pub struct AutoUnlock {
    /// Nothing starts until the window is activated again: after the
    /// person locked here, or once Touch ID was asked for.
    waiting: bool,
    /// An unlock runs, with Touch ID or the password.
    busy: bool,
    /// The window went to the background by the person's doing, rather
    /// than because Touch ID's prompt took the keyboard from it.
    away: bool,
}

impl AutoUnlock {
    /// For a lock screen that shows because the person locked the account
    /// in this window (`locked_here`), or for another reason, such as the
    /// app starting, the lock timeout or another account chosen.
    pub fn new(locked_here: bool) -> Self {
        Self {
            waiting: locked_here,
            busy: false,
            away: false,
        }
    }

    /// The window went to the background.
    pub fn deactivated(&mut self) {
        // While an unlock runs, Touch ID's prompt took the keyboard: its
        // going isn't the person coming back.
        self.away = !self.busy;
    }

    /// The window came forward again: Touch ID may be asked for again.
    pub fn activated(&mut self) {
        if std::mem::take(&mut self.away) {
            self.waiting = false;
        }
    }

    /// An unlock starts; `touch_id` says it is with Touch ID.
    pub fn started(&mut self, touch_id: bool) {
        self.busy = true;
        self.waiting |= touch_id;
    }

    /// The unlock failed or was canceled.
    pub fn ended(&mut self) {
        self.busy = false;
    }

    /// Whether to start Touch ID now: it can unlock the account (`ready`),
    /// the window is the active one, and nothing above holds it back.
    pub fn should_start(&self, ready: bool, active: bool) -> bool {
        ready && active && !self.waiting && !self.busy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_starts_in_the_window_in_use() {
        // The app starts, or another account is chosen, in the window in
        // use: at once.
        let auto = AutoUnlock::new(false);
        assert!(auto.should_start(true, true));
        // Not without Touch ID ready.
        assert!(!auto.should_start(false, true));
    }

    #[test]
    fn a_lock_in_the_background_waits_for_the_window() {
        // The lock timeout locked a window behind another app.
        let mut auto = AutoUnlock::new(false);
        assert!(!auto.should_start(true, false));
        auto.activated();
        assert!(auto.should_start(true, true));
    }

    #[test]
    fn a_lock_here_waits_until_the_window_comes_back() {
        let mut auto = AutoUnlock::new(true);
        assert!(!auto.should_start(true, true));
        // Clicking in the window changes nothing; leaving and coming back
        // does.
        auto.activated();
        assert!(!auto.should_start(true, true));
        auto.deactivated();
        auto.activated();
        assert!(auto.should_start(true, true));
    }

    #[test]
    fn a_cancel_waits_until_the_window_comes_back() {
        let mut auto = AutoUnlock::new(false);
        auto.started(true);
        // The system's prompt takes the keyboard, and gives it back once
        // canceled: that isn't the person coming back.
        auto.deactivated();
        assert!(!auto.should_start(true, false));
        auto.ended();
        auto.activated();
        assert!(!auto.should_start(true, true));
        // The person goes to another app and comes back.
        auto.deactivated();
        auto.activated();
        assert!(auto.should_start(true, true));
    }

    #[test]
    fn nothing_starts_while_an_unlock_runs() {
        let mut auto = AutoUnlock::new(false);
        auto.started(false);
        assert!(!auto.should_start(true, true));
        // A wrong password: Touch ID may still start.
        auto.ended();
        assert!(auto.should_start(true, true));
    }

    #[test]
    fn another_account_starts_afresh() {
        let mut auto = AutoUnlock::new(false);
        auto.started(true);
        auto.ended();
        assert!(!auto.should_start(true, true));
        // Choosing another locked account shows a new lock screen.
        let other = AutoUnlock::new(false);
        assert!(other.should_start(true, true));
    }
}
