use std::time::{Duration, SystemTime};

/// An account is an rbw profile on this computer, the `RBW_PROFILE` the
/// command line uses too. The default profile, used when `RBW_PROFILE` is
/// unset, has the empty name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(String);

impl AccountId {
    pub fn new(profile: impl Into<String>) -> Self {
        Self(profile.into())
    }

    /// The value of `RBW_PROFILE` for this account.
    pub fn profile(&self) -> &str {
        &self.0
    }

    pub fn is_default(&self) -> bool {
        self.0.is_empty()
    }

    /// The profile for a new account: its email, as a name that suits a
    /// folder and a service name, made unique among the taken ones.
    pub fn for_email(email: &str, taken: impl Fn(&str) -> bool) -> Self {
        let mut base = String::new();
        for c in email.to_lowercase().chars() {
            if c.is_ascii_alphanumeric() {
                base.push(c);
            } else if !base.is_empty() && !base.ends_with('-') {
                base.push('-');
            }
        }
        let base = match base.trim_end_matches('-') {
            "" => "account",
            base => base,
        };
        let mut name = base.to_string();
        let mut n = 2;
        while taken(&name) {
            name = format!("{base}-{n}");
            n += 1;
        }
        Self(name)
    }
}

impl std::fmt::Display for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_default() {
            f.write_str("(default)")
        } else {
            f.write_str(&self.0)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Server {
    BitwardenCom,
    BitwardenEu,
    /// A self-hosted Bitwarden or Vaultwarden server, by its web vault
    /// address, such as `https://vault.example.com`.
    SelfHosted(String),
}

impl Server {
    /// The address of a website's icon on this server's icon service, for
    /// a website as an item stores it. `None` when it has no host name.
    pub fn icon_url(&self, website: &str) -> Option<String> {
        let host = host(website)?;
        Some(match self {
            Self::BitwardenCom => {
                format!("https://icons.bitwarden.net/{host}/icon.png")
            }
            Self::BitwardenEu => {
                format!("https://icons.bitwarden.eu/{host}/icon.png")
            }
            Self::SelfHosted(url) => {
                format!("{}/icons/{host}/icon.png", url.trim_end_matches('/'))
            }
        })
    }
}

// The host name of a website as people type it: with or without a scheme,
// a port, a path or credentials.
fn host(website: &str) -> Option<&str> {
    let rest = website.trim();
    let rest = rest.split_once("://").map_or(rest, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?;
    let host = if host_port.starts_with('[') {
        host_port.split_inclusive(']').next()?
    } else {
        host_port.split(':').next()?
    };
    let valid = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':'));
    valid.then_some(host)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockState {
    /// The profile exists but has no signed-in account: it needs
    /// [`Backend::sign_in`](crate::Backend::sign_in).
    SignedOut,
    Locked,
    Unlocked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    /// `None` for a profile whose configuration has no email yet.
    pub email: Option<String>,
    pub server: Server,
    pub state: LockState,
    /// Whether Touch ID can unlock the account now. It can once Touch ID
    /// unlock is on and the account was unlocked with its password since
    /// the agent started.
    pub touch_id_ready: bool,
    /// When the local copy of the vault last synced with the server. It
    /// changes on every sync, also the background syncs of the agent, so it
    /// tells when to reload the items.
    pub last_sync: Option<SystemTime>,
}

/// A two-step login method, as the server offers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TwoStepMethod {
    Authenticator,
    Email,
    YubiKey,
}

/// The code that finishes a sign-in after
/// [`Error::TwoStepRequired`](crate::Error::TwoStepRequired) or
/// [`Error::NewDeviceCodeRequired`](crate::Error::NewDeviceCodeRequired).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginCode {
    TwoStep {
        method: TwoStepMethod,
        code: String,
    },
    /// The code the server emailed to verify a new device.
    NewDevice(String),
}

/// The settings rbw keeps for each account, shared with the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountSettings {
    /// How long the vault stays unlocked without being used.
    pub lock_timeout: Duration,
    /// Whether Touch ID can unlock the vault; `None` where Touch ID isn't
    /// available.
    pub touch_id: Option<bool>,
    /// Whether the SSH agent asks before each use of a key.
    pub ssh_confirm: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_for_emails() {
        let taken = |name: &str| name == "sam-example-com";
        assert_eq!(
            AccountId::for_email("Sam+Work@Example.com", taken).profile(),
            "sam-work-example-com"
        );
        assert_eq!(
            AccountId::for_email("sam@example.com", taken).profile(),
            "sam-example-com-2"
        );
        assert_eq!(AccountId::for_email("@", taken).profile(), "account");
    }

    #[test]
    fn icon_urls() {
        let com = Server::BitwardenCom;
        assert_eq!(
            com.icon_url("https://github.com/login").as_deref(),
            Some("https://icons.bitwarden.net/github.com/icon.png")
        );
        assert_eq!(
            com.icon_url("user:pw@example.org:8443").as_deref(),
            Some("https://icons.bitwarden.net/example.org/icon.png")
        );
        let own = Server::SelfHosted("https://vault.example.com/".into());
        assert_eq!(
            own.icon_url("example.com").as_deref(),
            Some("https://vault.example.com/icons/example.com/icon.png")
        );
        assert_eq!(com.icon_url(""), None);
        assert_eq!(com.icon_url("androidapp://com.example app"), None);
    }
}
