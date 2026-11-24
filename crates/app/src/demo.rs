//! The demo backend, which the app runs on instead of rbw when
//! `RBW_DESKTOP_DEMO` is set, even empty: for screenshots, and to try the
//! interface without an account. Its value is a comma-separated list of
//!
//! * `first-run`: no accounts, as on a computer where rbw was never set up;
//! * `unlocked`: the first demo account unlocked, to start in the vault;
//! * `signed-out`: a third account that is signed out;
//! * `touch-id`: Touch ID ready to unlock the first account;
//! * `slow`: every call takes two seconds, to see what waits;
//! * `offline`: calls that need the server fail;
//! * `no-rbw`, `old-rbw`: rbw missing, or too old; `outdated-rbw`: the app
//!   uses its own rbw, the one on `PATH` being too old;
//! * `ssh-off`: no SSH agent set up; `ssh-unsupported`: none can be;
//!   `ssh-elsewhere`: set up, but `ssh` uses another agent; `ssh-fails`:
//!   setting it up fails; `ssh-slow`: setting it up takes a while;
//! * `no-icons`: no website has an icon;
//! * `big`: 5,000 more logins in the first account, whose websites' icons
//!   take 300 ms each, as from the network;
//! * `panel`: the menu bar item's panel opens alone, as if the item was
//!   clicked, for systems without one;
//! * `settings`: the Settings window opens too, over the main window.
//!
//! Without a word, both demo accounts start locked.

use std::time::Duration;

use rbw_desktop_core::{
    Backend as _, Error, FakeBackend, Operation, OutdatedRbw, Preferences, REQUIRED_RBW, Result,
    Server, SshUses,
};

/// The words of `RBW_DESKTOP_DEMO`, or `None` to run on rbw.
pub fn words() -> Option<String> {
    std::env::var("RBW_DESKTOP_DEMO").ok()
}

/// Whether the demo runs and asks for `word`.
pub fn wants(word: &str) -> bool {
    words().is_some_and(|words| words.split(',').any(|w| w.trim() == word))
}

pub fn backend(demo: &str) -> Result<FakeBackend> {
    let wants = |word: &str| demo.split(',').any(|w| w.trim() == word);
    if wants("no-rbw") {
        return Err(Error::RbwNotFound);
    }
    if wants("old-rbw") {
        return Err(Error::RbwTooOld {
            found: "1.13.2".to_string(),
            required: REQUIRED_RBW.to_string(),
        });
    }
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
    // Before unlocking: on macOS, removing the agent locks the vault.
    if wants("ssh-off")
        && let Some(id) = &first
        && fake.uninstall_ssh_agent(id).is_err()
    {
        log::error!("the demo's SSH agent wasn't removed");
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
    if wants("big")
        && let Some(id) = &first
    {
        fake.add_logins(id, 5000);
        fake.set_latency_of(Operation::WebsiteIcon, Duration::from_millis(300));
    }
    if wants("outdated-rbw") {
        fake.set_outdated_rbw(Some(OutdatedRbw {
            path: "/usr/bin/rbw".into(),
            version: "1.13.2".into(),
        }));
    }
    if wants("offline") {
        fake.set_offline(true);
    }
    if wants("slow") {
        fake.set_latency(Duration::from_secs(2));
    }
    if wants("ssh-unsupported") {
        fake.set_ssh_supported(false);
    }
    if wants("ssh-elsewhere") {
        fake.set_ssh_uses(Some(SshUses::Other("/run/user/1000/keyring/ssh".into())));
    }
    if wants("ssh-fails") {
        fake.fail(
            Operation::InstallSshAgent,
            Error::Other("systemctl failed".into()),
        );
    }
    if wants("ssh-slow") {
        fake.set_latency_of(Operation::InstallSshAgent, Duration::from_secs(3));
    }
    if wants("no-icons")
        && fake
            .set_preferences(&Preferences {
                website_icons: false,
                ..Preferences::default()
            })
            .is_err()
    {
        log::error!("the demo's website icons weren't turned off");
    }
    Ok(fake)
}
