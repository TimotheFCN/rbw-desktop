use crate::{AccountId, TwoStepMethod};

/// What went wrong, as a value the interface can act on. The `Display`
/// text is for logs, not for people.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("the account is not signed in")]
    NotSignedIn,
    #[error("the vault is locked")]
    Locked,
    #[error("the master password is incorrect")]
    WrongPassword,
    #[error("a two-step login code is required")]
    TwoStepRequired { methods: Vec<TwoStepMethod> },
    #[error("a new device verification code is required")]
    NewDeviceCodeRequired,
    #[error("the code is incorrect")]
    WrongCode,
    #[error("none of the account's two-step login methods are supported")]
    TwoStepUnsupported,
    #[error("Touch ID was canceled")]
    TouchIdCanceled,
    #[error("Touch ID can't unlock the account now")]
    TouchIdUnavailable,
    #[error("no such account")]
    AccountNotFound,
    #[error("an account for this email and server exists: {0}")]
    AccountExists(AccountId),
    #[error("no such item or folder")]
    NotFound,
    #[error("the item changed on the server; sync and try again")]
    Conflict,
    #[error("this item can't be changed")]
    ReadOnly,
    #[error("the item isn't valid: {0}")]
    InvalidItem(String),
    #[error("the SSH agent of {0} is installed; only one can be")]
    SshAgentInstalledForOther(AccountId),
    #[error("the SSH agent service isn't supported on this system")]
    SshAgentUnsupported,
    #[error("the server can't be reached")]
    Offline,
    #[error("the server refused the request: {0}")]
    Server(String),
    #[error("rbw took too long to answer")]
    Timeout,
    #[error("rbw is not installed")]
    RbwNotFound,
    #[error("rbw {found} is too old; rbw-desktop needs {required} or later")]
    RbwTooOld { found: String, required: String },
    #[error("{0}")]
    Other(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
