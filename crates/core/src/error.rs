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
    /// The server wants this computer registered with the account's
    /// personal API key before it accepts the master password: ask for the
    /// key, pass it to [`Backend::register_device`](crate::Backend::register_device)
    /// and sign in again.
    #[error("the server wants this device registered with the account's API key")]
    RegistrationRequired,
    #[error("the API key is incorrect")]
    WrongApiKey,
    #[error("none of the account's two-step login methods are supported")]
    TwoStepUnsupported,
    #[error("rbw would have to ask for the master password")]
    PasswordRequired,
    /// The item asks for the master password again before its secrets are
    /// shown or changed: ask for it, pass it to
    /// [`Backend::unlock_item`](crate::Backend::unlock_item) and try again.
    #[error("the item asks for the master password again")]
    RepromptRequired,
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
    /// The server's address leads to something that isn't a Bitwarden
    /// server, such as a web site or a wrong path.
    #[error("the address doesn't lead to a Bitwarden server")]
    WrongServer,
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
