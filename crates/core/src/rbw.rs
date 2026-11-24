//! The backend that drives the `rbw` command line of rbw-ng: one short
//! process per call, `RBW_PROFILE` for the account, JSON in and out. The
//! vault stays unlocked in each profile's `rbw-agent` between calls.

mod icons;
mod json;
mod process;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use icons::IconCache;
use json::{ItemJson, ProfileJson, StatusJson};
pub use process::RBW_ENV;
use process::{Rbw, Wait};

use crate::{
    Account, AccountId, AccountSettings, Backend, CharacterClasses, Characters, Error, Folder,
    FolderId, GeneratorOptions, Item, ItemData, ItemId, ItemSummary, LockState, LoginCode,
    Preferences, Result, Secret, Server, SortOrder, SshAgentStatus, SshService, SshUses,
    VerificationCode, WebsiteIcon, WindowPlace, icon,
};

/// How many recently used items each account keeps.
const RECENT: usize = 20;

/// The rbw-ng release with everything this backend uses.
pub const REQUIRED_RBW: &str = "1.19.0";

/// An rbw on `PATH` older than the one the backend uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutdatedRbw {
    pub path: PathBuf,
    /// Its `--version`: a number for rbw-ng, the whole text for another
    /// program, such as the original rbw.
    pub version: String,
}

pub struct RbwBackend {
    rbw: Rbw,
    outdated: Option<OutdatedRbw>,
    preferences: PathBuf,
    /// Each account's recently used items: ids only.
    recent: PathBuf,
    /// Held while the recent items are read and written back.
    recent_lock: Mutex<()>,
    icons: IconCache,
}

impl RbwBackend {
    /// Finds an `rbw` that is [`REQUIRED_RBW`] or later: the one in
    /// [`RBW_ENV`], else the one on `PATH`, else the one next to this
    /// program.
    pub fn new() -> Result<Self> {
        let (rbw, outdated) = Rbw::find()?;
        Self::with_rbw(rbw, outdated)
    }

    /// Uses the `rbw` at this path, and the `rbw-agent` next to it if there
    /// is one.
    pub fn at(rbw: PathBuf) -> Result<Self> {
        Self::with_rbw(Rbw::checked(rbw)?, None)
    }

    fn with_rbw(rbw: Rbw, outdated: Option<OutdatedRbw>) -> Result<Self> {
        let dirs = app_dirs()?;
        Ok(Self {
            rbw,
            outdated,
            preferences: dirs.config_dir().join("preferences.json"),
            recent: dirs.config_dir().join("recent.json"),
            recent_lock: Mutex::new(()),
            icons: IconCache::new(dirs.cache_dir().join("icons")),
        })
    }

    fn run(&self, account: &AccountId, args: &[&str], wait: Wait) -> Result<()> {
        self.rbw.run(Some(account), args, None, wait)?;
        Ok(())
    }

    fn run_with_secret(
        &self,
        account: &AccountId,
        args: &[&str],
        secret: &Secret,
        wait: Wait,
    ) -> Result<()> {
        self.rbw
            .run(Some(account), args, Some(secret.expose().as_bytes()), wait)?;
        Ok(())
    }

    fn json<T: serde::de::DeserializeOwned>(
        &self,
        account: &AccountId,
        args: &[&str],
        stdin: Option<&[u8]>,
        wait: Wait,
    ) -> Result<T> {
        self.rbw.json(Some(account), args, stdin, wait)
    }

    fn profiles(&self) -> Result<Vec<ProfileJson>> {
        self.rbw
            .json(None, &["profiles", "--json"], None, Wait::Local)
    }

    fn item_json(&self, account: &AccountId, item: &ItemId) -> Result<ItemJson> {
        self.json(account, &["item", "get", "--", &item.0], None, Wait::Local)
    }

    fn save(&self, account: &AccountId, args: &[&str], item: &ItemJson) -> Result<ItemSummary> {
        let input = json::to_stdin(item)?;
        let saved: ItemJson = self.json(account, args, Some(&input), Wait::Network)?;
        saved
            .summary()
            .ok_or_else(|| Error::Other("rbw saved an item of an unknown kind".into()))
    }

    fn set_config(&self, account: &AccountId, key: &str, value: &str) -> Result<()> {
        self.run(account, &["config", "set", "--", key, value], Wait::Local)
    }

    /// Every account's recent items, by profile. Call with `recent_lock`
    /// held.
    fn read_recent(&self) -> Result<HashMap<String, Vec<String>>> {
        match std::fs::read_to_string(&self.recent) {
            // a broken file starts the record again
            Ok(text) => Ok(serde_json::from_str(&text).unwrap_or_default()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(error) => Err(Error::Other(format!(
                "can't read the recent items: {error}"
            ))),
        }
    }

    fn change_recent(&self, change: impl FnOnce(&mut HashMap<String, Vec<String>>)) -> Result<()> {
        let _held = self
            .recent_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut recent = self.read_recent()?;
        change(&mut recent);
        write_json(&self.recent, &recent)
            .map_err(|error| Error::Other(format!("can't save the recent items: {error}")))
    }
}

/// Writes `value` whole next to `path`, then moves it over the old file.
fn write_json(path: &Path, value: &impl serde::Serialize) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, path)
}

// The app's folders, named after APP_ID's parts: macOS names them after the
// whole id, Linux after the last part in lower case, rbwdesktop. (rbw's
// profiles take the folders named rbw- and a name, so "rbw-desktop" would
// belong to a profile called desktop, whose rbw purge --all would remove
// them.)
const APP_ID_PARTS: [&str; 3] = ["io.github", "timothefcn", "RbwDesktop"];

fn app_dirs() -> Result<directories::ProjectDirs> {
    let [qualifier, organization, application] = APP_ID_PARTS;
    directories::ProjectDirs::from(qualifier, organization, application)
        .ok_or_else(|| Error::Other("this system has no home folder".into()))
}

/// A folder of the app's for what lasts as long as the login session, such
/// as a socket: `$XDG_RUNTIME_DIR/rbwdesktop` on Linux, its cache folder
/// elsewhere. It is made if missing, for this user only.
pub fn session_dir() -> Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt as _;

    let dirs = app_dirs()?;
    let dir = dirs.runtime_dir().unwrap_or(dirs.cache_dir()).to_path_buf();
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|error| Error::Other(format!("can't make {}: {error}", dir.display())))?;
    Ok(dir)
}

#[derive(serde::Deserialize)]
struct ConfigJson {
    lock_timeout: u64,
    #[serde(default)]
    touch_id_unlock: bool,
    #[serde(default)]
    ssh_confirm: bool,
    #[serde(default)]
    touch_id_ssh: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PreferencesJson {
    // seconds; null leaves the clipboard alone
    clipboard_clear: Option<u64>,
    website_icons: bool,
    // these came later: older files take the defaults
    #[serde(default)]
    generator: Option<GeneratorJson>,
    #[serde(default)]
    menu_bar: Option<bool>,
    #[serde(default)]
    list_share: Option<f32>,
    #[serde(default)]
    sort: Option<SortJson>,
    #[serde(default)]
    window: Option<WindowJson>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct WindowJson {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    #[serde(default)]
    maximized: bool,
    #[serde(default)]
    display: Option<String>,
}

impl From<WindowJson> for WindowPlace {
    fn from(saved: WindowJson) -> Self {
        Self {
            x: saved.x,
            y: saved.y,
            width: saved.width,
            height: saved.height,
            maximized: saved.maximized,
            display: saved.display,
        }
    }
}

impl From<WindowPlace> for WindowJson {
    fn from(place: WindowPlace) -> Self {
        Self {
            x: place.x,
            y: place.y,
            width: place.width,
            height: place.height,
            maximized: place.maximized,
            display: place.display,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum SortJson {
    Name,
    Modified,
    RecentlyUsed,
}

impl From<SortOrder> for SortJson {
    fn from(order: SortOrder) -> Self {
        match order {
            SortOrder::Name => SortJson::Name,
            SortOrder::Modified => SortJson::Modified,
            SortOrder::RecentlyUsed => SortJson::RecentlyUsed,
        }
    }
}

impl From<SortJson> for SortOrder {
    fn from(order: SortJson) -> Self {
        match order {
            SortJson::Name => SortOrder::Name,
            SortJson::Modified => SortOrder::Modified,
            SortJson::RecentlyUsed => SortOrder::RecentlyUsed,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum GeneratorJson {
    Password {
        length: usize,
        characters: CharactersJson,
    },
    Passphrase {
        words: usize,
        // these came later: older files take rbw's own
        #[serde(default = "space")]
        separator: Option<char>,
        #[serde(default)]
        capitalize: bool,
        #[serde(default)]
        number: bool,
    },
}

fn space() -> Option<char> {
    Some(' ')
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum CharactersJson {
    All,
    NoSymbols,
    DigitsOnly,
    Unambiguous,
    Classes {
        uppercase: bool,
        lowercase: bool,
        digits: bool,
        symbols: bool,
        each_class: bool,
    },
}

impl From<GeneratorOptions> for GeneratorJson {
    fn from(options: GeneratorOptions) -> Self {
        match options {
            GeneratorOptions::Password { length, characters } => GeneratorJson::Password {
                length,
                characters: match characters {
                    Characters::All => CharactersJson::All,
                    Characters::NoSymbols => CharactersJson::NoSymbols,
                    Characters::DigitsOnly => CharactersJson::DigitsOnly,
                    Characters::Unambiguous => CharactersJson::Unambiguous,
                    Characters::Classes(classes) => CharactersJson::Classes {
                        uppercase: classes.uppercase,
                        lowercase: classes.lowercase,
                        digits: classes.digits,
                        symbols: classes.symbols,
                        each_class: classes.each_class,
                    },
                },
            },
            GeneratorOptions::Passphrase {
                words,
                separator,
                capitalize,
                number,
            } => GeneratorJson::Passphrase {
                words,
                separator,
                capitalize,
                number,
            },
        }
    }
}

impl From<GeneratorJson> for GeneratorOptions {
    fn from(json: GeneratorJson) -> Self {
        match json {
            GeneratorJson::Password { length, characters } => GeneratorOptions::Password {
                length,
                characters: match characters {
                    CharactersJson::All => Characters::All,
                    CharactersJson::NoSymbols => Characters::NoSymbols,
                    CharactersJson::DigitsOnly => Characters::DigitsOnly,
                    CharactersJson::Unambiguous => Characters::Unambiguous,
                    CharactersJson::Classes {
                        uppercase,
                        lowercase,
                        digits,
                        symbols,
                        each_class,
                    } => Characters::Classes(CharacterClasses {
                        uppercase,
                        lowercase,
                        digits,
                        symbols,
                        each_class,
                    }),
                },
            },
            GeneratorJson::Passphrase {
                words,
                separator,
                capitalize,
                number,
            } => GeneratorOptions::Passphrase {
                words,
                separator,
                capitalize,
                number,
            },
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SshStatusJson {
    supported: bool,
    installed: bool,
    other_profiles: Vec<Option<String>>,
    running: bool,
    socket: PathBuf,
    ssh: SshJson,
}

#[derive(serde::Deserialize)]
struct SshJson {
    agent: String,
    socket: Option<PathBuf>,
}

impl Backend for RbwBackend {
    fn accounts(&self) -> Result<Vec<Account>> {
        self.profiles()?
            .into_iter()
            .map(|profile| self.account(&AccountId::new(profile.profile.unwrap_or_default())))
            .collect()
    }

    fn account(&self, account: &AccountId) -> Result<Account> {
        let status: StatusJson = self.json(account, &["status", "--json"], None, Wait::Local)?;
        Ok(Account {
            id: account.clone(),
            email: status.user_email,
            server: json::server(status.server_url.as_deref()),
            state: match status.status.as_str() {
                "unlocked" => LockState::Unlocked,
                "locked" => LockState::Locked,
                _ => LockState::SignedOut,
            },
            touch_id_ready: status.touch_id,
            last_sync: status.last_sync.as_deref().and_then(json::parse_time),
        })
    }

    fn add_account(&self, server: &Server, email: &str) -> Result<AccountId> {
        let email = email.trim();
        let profiles = self.profiles()?;
        if let Some(existing) = profiles.iter().find(|profile| {
            profile
                .user_email
                .as_deref()
                .is_some_and(|known| known.eq_ignore_ascii_case(email))
                && json::server(profile.server_url.as_deref()) == *server
        }) {
            return Err(Error::AccountExists(AccountId::new(
                existing.profile.clone().unwrap_or_default(),
            )));
        }
        // the first account is the default profile, which the command line
        // uses without RBW_PROFILE
        let account = if profiles.iter().any(|profile| profile.profile.is_none()) {
            AccountId::for_email(email, |name| {
                profiles
                    .iter()
                    .any(|profile| profile.profile.as_deref() == Some(name))
            })
        } else {
            AccountId::default()
        };
        self.set_config(&account, "email", email)?;
        if let Some(base_url) = json::base_url(server) {
            self.set_config(&account, "base_url", base_url)?;
        }
        Ok(account)
    }

    fn sign_in(
        &self,
        account: &AccountId,
        password: &Secret,
        code: Option<&LoginCode>,
    ) -> Result<()> {
        let method;
        let mut args = vec!["login", "--password-stdin"];
        // in the environment, which only this user can read, not in the
        // arguments, which every user sees
        let code = match code {
            Some(LoginCode::TwoStep {
                method: chosen,
                code,
            }) => {
                method = process::method_id(*chosen).to_string();
                args.extend(["--method", &method]);
                Some(code)
            }
            Some(LoginCode::NewDevice(code)) => Some(code),
            None => None,
        };
        if code.is_some() {
            args.push("--code-env");
        }
        let env: Vec<_> = code
            .map(|code| ("RBW_CODE", code.as_str()))
            .into_iter()
            .collect();
        self.rbw.run_with_env(
            Some(account),
            &args,
            Some(password.expose().as_bytes()),
            &env,
            Wait::Network,
        )?;
        Ok(())
    }

    fn register_device(
        &self,
        account: &AccountId,
        client_id: &str,
        client_secret: &Secret,
    ) -> Result<()> {
        let env = [
            ("RBW_CLIENT_ID", client_id),
            ("RBW_CLIENT_SECRET", client_secret.expose()),
        ];
        match self.rbw.run_with_env(
            Some(account),
            &["register", "--api-key-env"],
            None,
            &env,
            Wait::Network,
        ) {
            Ok(_) => Ok(()),
            Err(failure) if failure.error == Error::WrongPassword => Err(Error::WrongApiKey),
            Err(failure) => Err(failure.error),
        }
    }

    fn send_email_code(&self, account: &AccountId, password: &Secret) -> Result<()> {
        let args = ["login", "--password-stdin", "--method", "1", "--code", ""];
        match self.rbw.run(
            Some(account),
            &args,
            Some(password.expose().as_bytes()),
            Wait::Network,
        ) {
            Err(failure) if failure.code.as_deref() == Some("two_step_code_sent") => Ok(()),
            Err(failure) => Err(failure.error),
            Ok(_) => Err(Error::Other("rbw logged in without a code".into())),
        }
    }

    fn remove_account(&self, account: &AccountId) -> Result<()> {
        self.run(account, &["purge", "--all"], Wait::Local)?;
        // neither matters once the account has gone, so failing to forget
        // them doesn't fail the removal
        let _ = self.icons.forget(account);
        let _ = self.change_recent(|recent| {
            recent.remove(account.profile());
        });
        Ok(())
    }

    fn unlock(&self, account: &AccountId, password: &Secret) -> Result<()> {
        self.run_with_secret(
            account,
            &["unlock", "--password-stdin"],
            password,
            Wait::Local,
        )
    }

    fn unlock_with_touch_id(&self, account: &AccountId) -> Result<()> {
        // Touch ID waits for the person
        self.run(account, &["unlock", "--touch-id"], Wait::Network)
    }

    fn lock(&self, account: &AccountId) -> Result<()> {
        self.run(account, &["lock"], Wait::Local)
    }

    fn sync(&self, account: &AccountId) -> Result<()> {
        self.run(account, &["sync"], Wait::Network)
    }

    fn items(&self, account: &AccountId) -> Result<Vec<ItemSummary>> {
        let items: Vec<ItemJson> = self.json(account, &["item", "list"], None, Wait::Local)?;
        Ok(items.into_iter().filter_map(ItemJson::summary).collect())
    }

    fn item(&self, account: &AccountId, item: &ItemId) -> Result<Item> {
        self.item_json(account, item)?
            .item()
            .ok_or_else(|| Error::Other("the item is of a kind this version doesn't know".into()))
    }

    fn unlock_item(&self, account: &AccountId, item: &ItemId, password: &Secret) -> Result<()> {
        self.run_with_secret(
            account,
            &["item", "unlock", "--password-stdin", "--", &item.0],
            password,
            Wait::Local,
        )
    }

    fn create_item(&self, account: &AccountId, data: &ItemData) -> Result<ItemSummary> {
        self.save(account, &["item", "create"], &ItemJson::from_data(data))
    }

    fn edit_item(
        &self,
        account: &AccountId,
        item: &ItemId,
        data: &ItemData,
    ) -> Result<ItemSummary> {
        // the edit names the organization the item is in, which only the
        // list knows without fetching the item's secrets
        let current = self
            .items(account)?
            .into_iter()
            .find(|summary| summary.id == *item)
            .ok_or(Error::NotFound)?;
        if !current.editable {
            return Err(Error::ReadOnly);
        }
        let mut edited = ItemJson::from_data(data);
        edited.id = Some(item.0.clone());
        edited.organization_id = current.organization;
        self.save(account, &["item", "edit", "--", &item.0], &edited)
    }

    fn delete_item(&self, account: &AccountId, item: &ItemId) -> Result<()> {
        self.run(account, &["item", "delete", "--", &item.0], Wait::Network)
    }

    fn set_favorite(&self, account: &AccountId, item: &ItemId, favorite: bool) -> Result<()> {
        let favorite = if favorite { "true" } else { "false" };
        self.json::<serde::de::IgnoredAny>(
            account,
            &["item", "favorite", "--", &item.0, favorite],
            None,
            Wait::Network,
        )
        .map(drop)
    }

    fn move_item(
        &self,
        account: &AccountId,
        item: &ItemId,
        folder: Option<&FolderId>,
    ) -> Result<()> {
        let mut args = vec!["item", "move", "--", &item.0];
        args.extend(folder.map(|folder| folder.0.as_str()));
        self.json::<serde::de::IgnoredAny>(account, &args, None, Wait::Network)
            .map(drop)
    }

    fn folders(&self, account: &AccountId) -> Result<Vec<Folder>> {
        let folders: Vec<json::FolderJson> =
            self.json(account, &["folder", "list"], None, Wait::Local)?;
        Ok(folders.into_iter().map(Folder::from).collect())
    }

    fn create_folder(&self, account: &AccountId, name: &str) -> Result<Folder> {
        let folder: json::FolderJson = self.json(
            account,
            &["folder", "create", "--", name],
            None,
            Wait::Network,
        )?;
        Ok(folder.into())
    }

    fn verification_code(&self, account: &AccountId, item: &ItemId) -> Result<VerificationCode> {
        let code: json::CodeJson =
            self.json(account, &["item", "code", "--", &item.0], None, Wait::Local)?;
        Ok(code.into())
    }

    fn website_icon(&self, account: &AccountId, item: &ItemSummary) -> Result<Option<WebsiteIcon>> {
        if !self.preferences()?.website_icons {
            return Ok(None);
        }
        let Some(host) = icon::icon_host(item) else {
            return Ok(None);
        };
        if let Some(cached) = self.icons.get(account, &host) {
            return Ok(cached);
        }
        let icon = match self
            .rbw
            .run(Some(account), &["icon", "--", &host], None, Wait::Network)
        {
            // a web page, an SVG or an oversized image counts as no icon
            Ok(bytes) => WebsiteIcon::from_server(&bytes),
            Err(failure) if failure.error == Error::NotFound => None,
            Err(failure) => return Err(failure.error),
        };
        self.icons.put(account, &host, icon.as_ref());
        Ok(icon)
    }

    fn cached_website_icon(
        &self,
        account: &AccountId,
        item: &ItemSummary,
    ) -> Result<Option<Option<WebsiteIcon>>> {
        if !self.preferences()?.website_icons {
            return Ok(Some(None));
        }
        Ok(match icon::icon_host(item) {
            Some(host) => self.icons.get(account, &host),
            None => Some(None),
        })
    }

    fn generate(&self, options: &GeneratorOptions) -> Result<Secret> {
        if !options.makes_anything() {
            return Err(Error::Other("these options make nothing".into()));
        }
        let flags = |flags: &[(bool, &str)]| {
            flags
                .iter()
                .filter(|(set, _)| *set)
                .map(|(_, flag)| flag.to_string())
                .collect::<Vec<_>>()
        };
        let (length, flags) = match *options {
            GeneratorOptions::Password { length, characters } => (
                length,
                match characters {
                    Characters::All => vec![],
                    Characters::NoSymbols => flags(&[(true, "--no-symbols")]),
                    Characters::DigitsOnly => flags(&[(true, "--only-numbers")]),
                    Characters::Unambiguous => flags(&[(true, "--nonconfusables")]),
                    Characters::Classes(classes) => flags(&[
                        (!classes.uppercase, "--no-uppercase"),
                        (!classes.lowercase, "--no-lowercase"),
                        (!classes.digits, "--no-numbers"),
                        (!classes.symbols, "--no-symbols"),
                        (classes.each_class, "--each-class"),
                    ]),
                },
            ),
            GeneratorOptions::Passphrase {
                words,
                separator,
                capitalize,
                number,
            } => {
                let mut args = flags(&[
                    (true, "--diceware"),
                    (capitalize, "--capitalize"),
                    (number, "--include-number"),
                ]);
                // rbw separates the words with a space unless told otherwise
                if separator != Some(' ') {
                    args.push("--separator".to_string());
                    args.push(separator.map(String::from).unwrap_or_default());
                }
                (words, args)
            }
        };
        let length = length.to_string();
        let mut args = vec!["generate", &length];
        args.extend(flags.iter().map(String::as_str));
        let output = self.rbw.run(None, &args, None, Wait::Local)?;
        let text = std::str::from_utf8(&output)
            .map_err(|_| Error::Other("rbw generated something that isn't text".into()))?;
        Ok(Secret::new(text.trim_end_matches('\n')))
    }

    fn account_settings(&self, account: &AccountId) -> Result<AccountSettings> {
        let config: ConfigJson = self.json(account, &["config", "show"], None, Wait::Local)?;
        Ok(AccountSettings {
            lock_timeout: Duration::from_secs(config.lock_timeout),
            touch_id: cfg!(target_os = "macos").then_some(config.touch_id_unlock),
            ssh_confirm: config.ssh_confirm || config.touch_id_ssh,
        })
    }

    fn set_account_settings(&self, account: &AccountId, settings: &AccountSettings) -> Result<()> {
        let current = self.account_settings(account)?;
        let seconds = settings.lock_timeout.as_secs();
        if seconds == 0 {
            return Err(Error::Other(
                "the lock timeout must be at least a second".into(),
            ));
        }
        if seconds != current.lock_timeout.as_secs() {
            self.set_config(account, "lock_timeout", &seconds.to_string())?;
        }
        if settings.touch_id != current.touch_id {
            let enabled = settings.touch_id.ok_or(Error::TouchIdUnavailable)?;
            self.set_config(account, "touch_id_unlock", &enabled.to_string())?;
        }
        if settings.ssh_confirm != current.ssh_confirm {
            self.set_config(account, "ssh_confirm", &settings.ssh_confirm.to_string())?;
        }
        Ok(())
    }

    fn preferences(&self) -> Result<Preferences> {
        let text = match std::fs::read_to_string(&self.preferences) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Preferences::default());
            }
            Err(error) => return Err(Error::Other(format!("can't read the preferences: {error}"))),
        };
        // preferences from a newer or broken file fall back to the defaults
        let defaults = Preferences::default();
        Ok(serde_json::from_str::<PreferencesJson>(&text)
            .map(|saved| Preferences {
                clipboard_clear: saved.clipboard_clear.map(Duration::from_secs),
                website_icons: saved.website_icons,
                generator: saved.generator.map_or(defaults.generator, Into::into),
                menu_bar: saved.menu_bar.unwrap_or(defaults.menu_bar),
                list_share: saved.list_share,
                sort: saved.sort.map_or(defaults.sort, Into::into),
                window: saved.window.map(Into::into),
            })
            .unwrap_or(defaults))
    }

    fn set_preferences(&self, preferences: &Preferences) -> Result<()> {
        let saved = PreferencesJson {
            clipboard_clear: preferences.clipboard_clear.map(|clear| clear.as_secs()),
            website_icons: preferences.website_icons,
            generator: Some(preferences.generator.into()),
            menu_bar: Some(preferences.menu_bar),
            list_share: preferences.list_share,
            sort: Some(preferences.sort.into()),
            window: preferences.window.clone().map(Into::into),
        };
        write_json(&self.preferences, &saved)
            .map_err(|error| Error::Other(format!("can't save the preferences: {error}")))?;
        if !preferences.website_icons {
            self.icons
                .clear()
                .map_err(|error| Error::Other(format!("can't forget the icons: {error}")))?;
        }
        Ok(())
    }

    fn recent_items(&self, account: &AccountId) -> Result<Vec<ItemId>> {
        let _held = self
            .recent_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut recent = self.read_recent()?;
        Ok(recent
            .remove(account.profile())
            .unwrap_or_default()
            .into_iter()
            .map(ItemId)
            .collect())
    }

    fn item_used(&self, account: &AccountId, item: &ItemId) -> Result<()> {
        self.change_recent(|recent| {
            let items = recent.entry(account.profile().to_string()).or_default();
            items.retain(|known| *known != item.0);
            items.insert(0, item.0.clone());
            items.truncate(RECENT);
        })
    }

    fn ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus> {
        let status: SshStatusJson = self.json(
            account,
            &["ssh-agent", "status", "--json"],
            None,
            Wait::Local,
        )?;
        let keys = match self.json::<Vec<ItemJson>>(account, &["item", "list"], None, Wait::Local) {
            Ok(items) => Some(items.iter().filter_map(ItemJson::ssh_key).collect()),
            Err(Error::Locked | Error::NotSignedIn | Error::PasswordRequired) => None,
            Err(error) => return Err(error),
        };
        let service = if !status.supported {
            SshService::Unsupported
        } else if status.installed {
            SshService::Installed
        } else if let Some(other) = status.other_profiles.into_iter().next() {
            SshService::InstalledForOther(AccountId::new(other.unwrap_or_default()))
        } else {
            SshService::NotInstalled
        };
        let ssh_uses = match (status.ssh.agent.as_str(), status.ssh.socket) {
            ("rbw", _) => SshUses::Rbw,
            ("other", Some(socket)) => SshUses::Other(socket),
            ("none", _) => SshUses::None,
            _ => SshUses::Unknown,
        };
        Ok(SshAgentStatus {
            service,
            running: status.running,
            socket: status.socket,
            ssh_uses,
            keys,
        })
    }

    fn install_ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus> {
        if self.rbw.agent_is_transient() {
            return Err(Error::AppNotInstalled);
        }
        match self.ssh_agent(account)?.service {
            SshService::Unsupported => return Err(Error::SshAgentUnsupported),
            SshService::InstalledForOther(other) => {
                return Err(Error::SshAgentInstalledForOther(other));
            }
            SshService::Installed | SshService::NotInstalled => {}
        }
        self.run(account, &["ssh-agent", "install"], Wait::Local)?;
        self.ssh_agent(account)
    }

    fn uninstall_ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus> {
        if self.ssh_agent(account)?.service == SshService::Installed {
            self.run(account, &["ssh-agent", "uninstall"], Wait::Local)?;
        }
        self.ssh_agent(account)
    }

    fn rbw_version(&self) -> Result<String> {
        Ok(self.rbw.version()?.text)
    }

    fn outdated_rbw(&self) -> Option<OutdatedRbw> {
        self.outdated.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folders_are_named_after_the_app_id() {
        assert_eq!(APP_ID_PARTS.join("."), crate::APP_ID);
        // directories' name on Linux
        let linux = APP_ID_PARTS[2].to_lowercase();
        assert_eq!(linux, "rbwdesktop");
        assert!(!linux.starts_with("rbw-"));
    }

    #[test]
    fn the_windows_place_comes_back_as_it_was_saved() {
        let place = WindowPlace {
            x: -1280.,
            y: 24.5,
            width: 900.,
            height: 600.,
            maximized: false,
            display: Some("37D8832A-2D66-02CA-B9F7-8F30A301B230".into()),
        };
        let json = serde_json::to_string(&WindowJson::from(place.clone())).unwrap();
        let back: WindowJson = serde_json::from_str(&json).unwrap();
        assert_eq!(WindowPlace::from(back), place);
        // saved before the display was
        let before: WindowJson =
            serde_json::from_str(r#"{"x": 0, "y": 0, "width": 900, "height": 600}"#).unwrap();
        assert_eq!(WindowPlace::from(before).display, None);
        let older: PreferencesJson =
            serde_json::from_str(r#"{"clipboard_clear": 30, "website_icons": true}"#).unwrap();
        assert!(older.window.is_none());
    }

    #[test]
    fn preferences_from_before_the_generator_take_its_defaults() {
        let saved: PreferencesJson =
            serde_json::from_str(r#"{"clipboard_clear": 30, "website_icons": false}"#).unwrap();
        assert!(saved.generator.is_none() && saved.menu_bar.is_none());
        for characters in [
            Characters::Unambiguous,
            Characters::Classes(CharacterClasses {
                uppercase: true,
                lowercase: false,
                digits: false,
                symbols: true,
                each_class: true,
            }),
        ] {
            let options = GeneratorOptions::Password {
                length: 12,
                characters,
            };
            let json = serde_json::to_string(&GeneratorJson::from(options)).unwrap();
            let back: GeneratorJson = serde_json::from_str(&json).unwrap();
            assert_eq!(GeneratorOptions::from(back), options);
        }
    }

    #[test]
    fn passphrases_keep_their_options_and_older_ones_take_rbws() {
        let older: GeneratorJson =
            serde_json::from_str(r#"{"kind": "passphrase", "words": 6}"#).unwrap();
        assert_eq!(
            GeneratorOptions::from(older),
            GeneratorOptions::passphrase(6)
        );
        for separator in [None, Some('-')] {
            let options = GeneratorOptions::Passphrase {
                words: 5,
                separator,
                capitalize: true,
                number: true,
            };
            let json = serde_json::to_string(&GeneratorJson::from(options)).unwrap();
            let back: GeneratorJson = serde_json::from_str(&json).unwrap();
            assert_eq!(GeneratorOptions::from(back), options);
        }
    }

    #[test]
    fn the_apps_folders_are_no_rbw_profiles() {
        let dirs = app_dirs().unwrap();
        for dir in [dirs.config_dir(), dirs.cache_dir()] {
            let name = dir.file_name().unwrap().to_str().unwrap();
            assert!(name != "rbw" && !name.starts_with("rbw-"), "{name}");
        }
    }
}
