//! The backend of rbw-desktop. [`Backend`] is everything the interface asks
//! of rbw: accounts, locking, the vault, the generator, settings and the SSH
//! agent. [`RbwBackend`] implements it with rbw-ng's command line, and
//! [`FakeBackend`] with demo data.
//!
//! Every method blocks until it is done, some on the network, so call them
//! off the interface's thread. The vault key stays in `rbw-agent`: items
//! come back with their secrets only from [`Backend::item`], and only for as
//! long as the caller keeps them.

mod account;
mod error;
mod fake;
mod generator;
mod item;
mod rbw;
mod secret;
mod settings;
mod ssh;

pub use account::{
    Account, AccountId, AccountSettings, LockState, LoginCode, Server, TwoStepMethod,
};
pub use error::{Error, Result};
pub use fake::FakeBackend;
pub use generator::{Characters, GeneratorOptions};
pub use item::{
    Card, CustomField, FieldValue, Folder, FolderId, Identity, Item, ItemContent, ItemData, ItemId,
    ItemKind, ItemSummary, Login, SshKey, UriMatch, VerificationCode, Website,
};
pub use rbw::{RBW_ENV, REQUIRED_RBW, RbwBackend};
pub use secret::Secret;
pub use settings::Preferences;
pub use ssh::{SshAgentStatus, SshKeySummary, SshService, SshUses};

pub trait Backend: Send + Sync {
    /// Every account on this computer, signed in or not.
    fn accounts(&self) -> Result<Vec<Account>>;

    /// One account's current state. It is cheap enough to poll every second
    /// or two to notice locking, unlocking and syncs.
    fn account(&self, account: &AccountId) -> Result<Account>;

    /// Creates a signed-out account for an email on a server, without
    /// asking the server anything. [`Backend::sign_in`] then signs it in,
    /// and [`Backend::remove_account`] drops it if that is abandoned.
    fn add_account(&self, server: &Server, email: &str) -> Result<AccountId>;

    /// Signs in with the master password, which also unlocks. Without a
    /// code it can fail with [`Error::TwoStepRequired`] or
    /// [`Error::NewDeviceCodeRequired`]; call it again with the code.
    fn sign_in(
        &self,
        account: &AccountId,
        password: &Secret,
        code: Option<&LoginCode>,
    ) -> Result<()>;

    /// Asks the server to email a two-step login code, for an account that
    /// offers [`TwoStepMethod::Email`].
    fn send_email_code(&self, account: &AccountId, password: &Secret) -> Result<()>;

    /// Signs out and removes the account and its local data from this
    /// computer, for the command line too.
    fn remove_account(&self, account: &AccountId) -> Result<()>;

    fn unlock(&self, account: &AccountId, password: &Secret) -> Result<()>;

    /// Unlocks with Touch ID, when [`Account::touch_id_ready`] says it can.
    fn unlock_with_touch_id(&self, account: &AccountId) -> Result<()>;

    fn lock(&self, account: &AccountId) -> Result<()>;

    /// Syncs the local copy of the vault with the server now. The agent
    /// also syncs on its own.
    fn sync(&self, account: &AccountId) -> Result<()>;

    fn items(&self, account: &AccountId) -> Result<Vec<ItemSummary>>;

    /// The item with its secrets. Fails with [`Error::RepromptRequired`] for
    /// an item that asks for the master password again and isn't unlocked.
    fn item(&self, account: &AccountId, item: &ItemId) -> Result<Item>;

    /// Unlocks an item that asks for the master password again
    /// ([`ItemSummary::reprompt`]), once rbw-agent has checked the password:
    /// [`Backend::item`], [`Backend::verification_code`] and
    /// [`Backend::edit_item`] then work for it until it goes unread for two
    /// minutes or the account locks. Each read keeps it unlocked, so
    /// refreshing its code keeps it open while it is shown. Fails with
    /// [`Error::WrongPassword`].
    fn unlock_item(&self, account: &AccountId, item: &ItemId, password: &Secret) -> Result<()>;

    fn create_item(&self, account: &AccountId, data: &ItemData) -> Result<ItemSummary>;

    /// Replaces the item's data. Fails with [`Error::Conflict`] if the item
    /// changed on the server since the last sync.
    fn edit_item(&self, account: &AccountId, item: &ItemId, data: &ItemData)
    -> Result<ItemSummary>;

    /// Moves the item to the trash.
    fn delete_item(&self, account: &AccountId, item: &ItemId) -> Result<()>;

    /// Marks or unmarks the item as a favourite. It sends no secrets and
    /// works for read-only items too: favourites belong to the account.
    fn set_favorite(&self, account: &AccountId, item: &ItemId, favorite: bool) -> Result<()>;

    /// Moves the item into a folder, or out of any folder with `None`. Like
    /// favourites, folders belong to the account: it sends no secrets and
    /// works for read-only items too. Fails with [`Error::NotFound`] for an
    /// unknown folder.
    fn move_item(
        &self,
        account: &AccountId,
        item: &ItemId,
        folder: Option<&FolderId>,
    ) -> Result<()>;

    fn folders(&self, account: &AccountId) -> Result<Vec<Folder>>;

    fn create_folder(&self, account: &AccountId, name: &str) -> Result<Folder>;

    /// The item's current verification code.
    fn verification_code(&self, account: &AccountId, item: &ItemId) -> Result<VerificationCode>;

    fn generate(&self, options: &GeneratorOptions) -> Result<Secret>;

    fn account_settings(&self, account: &AccountId) -> Result<AccountSettings>;

    fn set_account_settings(&self, account: &AccountId, settings: &AccountSettings) -> Result<()>;

    fn preferences(&self) -> Result<Preferences>;

    fn set_preferences(&self, preferences: &Preferences) -> Result<()>;

    fn ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus>;

    /// Installs the account's agent as a login service that serves its SSH
    /// keys.
    fn install_ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus>;

    fn uninstall_ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus>;

    /// The version of rbw the backend uses, for the About page.
    fn rbw_version(&self) -> Result<String>;
}
