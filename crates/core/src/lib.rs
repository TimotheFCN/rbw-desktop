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
mod icon;
mod item;
mod rbw;
mod secret;
mod settings;
mod ssh;

/// The app's id, wherever a system names it: the macOS bundle identifier
/// and folders, the Linux desktop file, window class and icon, and the
/// menu bar item.
pub const APP_ID: &str = "io.github.timothefcn.RbwDesktop";

/// The app's name, as people see it: in the Dock and the menu bar, GNOME's
/// overview, its menus and its windows.
pub const APP_NAME: &str = "Fastwarden";

pub use account::{
    Account, AccountId, AccountSettings, LockState, LoginCode, Server, TwoStepMethod,
};
pub use error::{Error, Result};
pub use fake::{FakeBackend, Operation};
pub use generator::{CharacterClasses, Characters, GeneratorOptions, Strength};
pub use icon::{IconFormat, WebsiteIcon};
pub use item::{
    Card, CustomField, FieldValue, Folder, FolderId, Identity, Item, ItemContent, ItemData, ItemId,
    ItemKind, ItemSummary, Login, Revision, SshKey, UriMatch, VerificationCode, Website,
};
pub use rbw::{OutdatedRbw, RBW_ENV, REQUIRED_RBW, RbwBackend, session_dir};
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
    /// [`Error::NewDeviceCodeRequired`]; call it again with the code. It
    /// fails with [`Error::RegistrationRequired`] when the server wants
    /// [`Backend::register_device`] first, and with [`Error::WrongServer`]
    /// when the account's server address leads to no Bitwarden server.
    fn sign_in(
        &self,
        account: &AccountId,
        password: &Secret,
        code: Option<&LoginCode>,
    ) -> Result<()>;

    /// Registers this computer with the server for a signed-out account,
    /// with the account's personal API key: its client id (`user.` and a
    /// number) and secret, which the web vault shows under Settings,
    /// Security, Keys. Some servers ask for it before they accept the master
    /// password ([`Error::RegistrationRequired`]); on bitwarden.com it also
    /// spares the emailed new device code. Then call
    /// [`Backend::sign_in`] again. Fails with [`Error::WrongApiKey`].
    fn register_device(
        &self,
        account: &AccountId,
        client_id: &str,
        client_secret: &Secret,
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
    /// changed on the server since `data` was read ([`ItemData::revision`]),
    /// or since the last sync for data without a revision.
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

    /// The icon of a login's website, from the icon service of the
    /// account's server, to show instead of the item's letter. `None` when
    /// [`Preferences::website_icons`] is off, which also fetches nothing,
    /// when the item has no website that gets an icon, or when the service
    /// has none. Icons are kept on disk for 30 days, and misses for 3, so
    /// after the first time this answers from the disk at once; the first
    /// time it waits on the server, up to 20 seconds. Like every method it
    /// blocks, so show the letter until it answers. Errors, such as
    /// [`Error::Offline`], aren't kept: a later call tries again.
    fn website_icon(&self, account: &AccountId, item: &ItemSummary) -> Result<Option<WebsiteIcon>>;

    /// The items of the account used last in the app on this computer,
    /// most recent first, as [`Backend::item_used`] noted them: their ids
    /// only, kept with the preferences. Items deleted since are still
    /// listed.
    fn recent_items(&self, account: &AccountId) -> Result<Vec<ItemId>>;

    /// Notes that something of the item was just copied or the item was
    /// opened, for [`Backend::recent_items`]. Removing the account forgets
    /// its items.
    fn item_used(&self, account: &AccountId, item: &ItemId) -> Result<()>;

    /// A new password or passphrase. It needs no account, and works while
    /// every account is locked. Options that make nothing fail
    /// ([`GeneratorOptions::makes_anything`]).
    fn generate(&self, options: &GeneratorOptions) -> Result<Secret>;

    fn account_settings(&self, account: &AccountId) -> Result<AccountSettings>;

    fn set_account_settings(&self, account: &AccountId, settings: &AccountSettings) -> Result<()>;

    fn preferences(&self) -> Result<Preferences>;

    fn set_preferences(&self, preferences: &Preferences) -> Result<()>;

    fn ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus>;

    /// Installs the account's agent as a login service that serves its SSH
    /// keys. It leaves the account as it is: an agent that runs keeps it
    /// unlocked, and serves the SSH keys, until it stops, and the service
    /// takes over then.
    fn install_ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus>;

    /// Removes the account's service. On Linux the account stays as it is
    /// until its agent stops; on macOS removing the service stops its agent,
    /// which locks the account.
    fn uninstall_ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus>;

    /// The version of rbw the backend uses, for the About page.
    fn rbw_version(&self) -> Result<String>;

    /// An older rbw on `PATH`, when the backend uses the one bundled with
    /// the app instead. rbw restarts an agent of another version, so the
    /// command line and the app would each restart the account's agent,
    /// which locks it, whenever the other used it: ask the person to update
    /// the one on `PATH`.
    fn outdated_rbw(&self) -> Option<OutdatedRbw> {
        None
    }
}
