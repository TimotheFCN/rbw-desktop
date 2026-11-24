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
    pub fingerprint: String,
}
