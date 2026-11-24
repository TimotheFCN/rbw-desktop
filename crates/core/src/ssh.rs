use std::path::PathBuf;

use crate::{AccountId, ItemId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshAgentStatus {
    pub service: SshService,
    /// Whether the account's agent is running now. Once installed, the
    /// service starts it when a program connects.
    pub running: bool,
    /// The socket that serves the account's SSH keys.
    pub socket: PathBuf,
    /// The agent `ssh` uses by default.
    pub ssh_uses: SshUses,
    /// The keys the agent serves; `None` while the vault is locked.
    pub keys: Option<Vec<SshKeySummary>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshService {
    /// This system has no service manager rbw supports.
    Unsupported,
    NotInstalled,
    Installed,
    /// Another account's agent is installed. Only one can be, so it has to
    /// be uninstalled first.
    InstalledForOther(AccountId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshUses {
    /// This account's agent.
    Rbw,
    /// Another agent, such as the system's own, at this socket.
    Other(PathBuf),
    None,
    /// `ssh` couldn't say.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshKeySummary {
    pub item: ItemId,
    pub name: String,
    /// The kind of key, as people name it: "Ed25519", "RSA", "ECDSA", or
    /// empty when it is one this version doesn't know.
    pub key_type: String,
    pub fingerprint: String,
    /// In OpenSSH's format, as servers and websites ask for it.
    pub public_key: String,
}

/// The kind of key a public key in OpenSSH's format holds, from its first
/// word, such as `ssh-ed25519`.
pub(crate) fn key_type(public_key: &str) -> String {
    let algorithm = public_key.split_whitespace().next().unwrap_or_default();
    let name = match algorithm {
        "ssh-ed25519" => "Ed25519",
        "sk-ssh-ed25519@openssh.com" => "Ed25519 security key",
        "ssh-rsa" => "RSA",
        "ecdsa-sha2-nistp256" | "ecdsa-sha2-nistp384" | "ecdsa-sha2-nistp521" => "ECDSA",
        "sk-ecdsa-sha2-nistp256@openssh.com" => "ECDSA security key",
        "ssh-dss" => "DSA",
        _ => "",
    };
    name.to_string()
}

/// What the item list says of an SSH key, such as "Ed25519 key"; `None`
/// for a kind of key this version doesn't know.
pub(crate) fn key_subtitle(public_key: &str) -> Option<String> {
    let kind = key_type(public_key);
    match kind.as_str() {
        "" => None,
        kind if kind.ends_with(" key") => Some(kind.to_string()),
        kind => Some(format!("{kind} key")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn key_subtitles() {
        assert_eq!(
            super::key_subtitle("ssh-ed25519 AAAA").as_deref(),
            Some("Ed25519 key")
        );
        assert_eq!(
            super::key_subtitle("sk-ssh-ed25519@openssh.com AAAA").as_deref(),
            Some("Ed25519 security key")
        );
        assert_eq!(super::key_subtitle("ssh-future AAAA"), None);
    }

    #[test]
    fn key_types() {
        assert_eq!(super::key_type("ssh-ed25519 AAAAC3Nz me@host"), "Ed25519");
        assert_eq!(super::key_type("ecdsa-sha2-nistp384 AAAA"), "ECDSA");
        assert_eq!(super::key_type("ssh-rsa AAAAB3"), "RSA");
        assert_eq!(super::key_type(""), "");
    }
}
