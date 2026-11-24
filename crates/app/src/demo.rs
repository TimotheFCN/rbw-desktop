//! The demo backend the app runs on until it uses rbw, set up by
//! `RBW_DESKTOP_DEMO`: a comma-separated list of
//!
//! * `first-run`: no accounts, as on a computer where rbw was never set up;
//! * `unlocked`: the first demo account unlocked, to start in the vault;
//! * `signed-out`: a third account that is signed out;
//! * `touch-id`: Touch ID ready to unlock the first account;
//! * `slow`: every call takes two seconds, to see what waits;
//! * `offline`: calls that need the server fail.
//!
//! Without it, both demo accounts start locked.

use std::time::Duration;

use rbw_desktop_core::{Backend as _, FakeBackend, Server};

pub fn backend() -> FakeBackend {
    let demo = std::env::var("RBW_DESKTOP_DEMO").unwrap_or_default();
    let wants = |word: &str| demo.split(',').any(|w| w.trim() == word);
    let fake = if wants("first-run") {
        FakeBackend::empty()
    } else {
        FakeBackend::new()
    };
    let first = fake
        .accounts()
        .ok()
        .and_then(|accounts| accounts.into_iter().next())
        .map(|account| account.id);
    if wants("touch-id") {
        fake.set_touch_id(true);
        // Touch ID unlocks an account once its password has unlocked it.
        if let Some(id) = &first {
            let unlocked = fake.unlock(id, &FakeBackend::PASSWORD.into());
            if unlocked.and_then(|()| fake.lock(id)).is_err() {
                log::error!("the demo's Touch ID setup failed");
            }
        }
    }
    if wants("unlocked")
        && let Some(id) = &first
    {
        fake.unlock_now(id);
    }
    if wants("signed-out")
        && fake
            .add_account(&Server::BitwardenCom, "sam@example.com")
            .is_err()
    {
        log::error!("the demo's signed-out account wasn't added");
    }
    if wants("offline") {
        fake.set_offline(true);
    }
    if wants("slow") {
        fake.set_latency(Duration::from_secs(2));
    }
    fake
}
