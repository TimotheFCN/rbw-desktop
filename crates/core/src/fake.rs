//! A backend with demo data, for building the interface, tests, screenshots
//! and a demo mode. It behaves like the real one: accounts lock after their
//! timeout, sign-in asks for codes, and edits, favourites and folders stick
//! for as long as the backend lives.

mod demo;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rand::Rng as _;
use rand::seq::IndexedRandom as _;

use crate::{
    Account, AccountId, AccountSettings, Backend, Characters, Error, Folder, FolderId,
    GeneratorOptions, Item, ItemContent, ItemData, ItemId, ItemSummary, LockState, LoginCode,
    Preferences, Result, Secret, Server, SshAgentStatus, SshKeySummary, SshService, SshUses,
    TwoStepMethod, VerificationCode,
};

pub struct FakeBackend {
    state: Mutex<State>,
}

struct State {
    accounts: Vec<FakeAccount>,
    preferences: Preferences,
    latency: Duration,
    fail_next: Option<Error>,
    offline: bool,
    touch_id: bool,
    // the account whose SSH agent is installed
    ssh_agent: Option<AccountId>,
    next_id: u64,
}

struct FakeAccount {
    id: AccountId,
    email: String,
    server: Server,
    state: LockState,
    // set once the account was unlocked with its password
    touch_id_ready: bool,
    last_use: Instant,
    last_sync: Option<SystemTime>,
    settings: AccountSettings,
    items: Vec<Item>,
    folders: Vec<Folder>,
    // items that ask for the password again, unlocked, with their last read
    unlocked_items: HashMap<ItemId, Instant>,
}

// How long an unlocked item that asks for the password again stays
// unlocked without being read, as in rbw-agent.
const ITEM_UNLOCK_IDLE: Duration = Duration::from_secs(120);

impl FakeBackend {
    /// The master password of every demo account.
    pub const PASSWORD: &str = "demo";
    /// The two-step and new device code the demo accepts. Signing in asks
    /// for a two-step code, or a new device code when the email has
    /// `+device@`.
    pub const CODE: &str = "123456";

    /// Two demo accounts, both locked: a personal one on bitwarden.com
    /// with about forty items, and a work one on a self-hosted server.
    pub fn new() -> Self {
        let touch_id = cfg!(target_os = "macos");
        let accounts = demo::accounts()
            .into_iter()
            .map(|demo| FakeAccount {
                id: AccountId::new(demo.profile),
                email: demo.email.to_string(),
                server: demo.server,
                state: LockState::Locked,
                touch_id_ready: false,
                last_use: Instant::now(),
                last_sync: Some(SystemTime::now() - Duration::from_secs(600)),
                settings: AccountSettings {
                    lock_timeout: Duration::from_secs(3600),
                    touch_id: touch_id.then_some(true),
                    ssh_confirm: false,
                },
                items: demo.items,
                folders: demo.folders,
                unlocked_items: HashMap::new(),
            })
            .collect();
        Self {
            state: Mutex::new(State {
                accounts,
                preferences: Preferences::default(),
                latency: Duration::ZERO,
                fail_next: None,
                offline: false,
                touch_id,
                ssh_agent: Some(AccountId::new(demo::PERSONAL)),
                next_id: 1,
            }),
        }
    }

    /// No accounts, as on a computer where rbw was never set up.
    pub fn empty() -> Self {
        let fake = Self::new();
        fake.state().accounts.clear();
        fake.state().ssh_agent = None;
        fake
    }

    /// Makes every call take this long, as rbw and the network do.
    pub fn set_latency(&self, latency: Duration) {
        self.state().latency = latency;
    }

    /// Makes the next call fail with this error, whatever it is.
    pub fn fail_next(&self, error: Error) {
        self.state().fail_next = Some(error);
    }

    /// While offline, calls that need the server fail with
    /// [`Error::Offline`]; the local copy of the vault still reads.
    pub fn set_offline(&self, offline: bool) {
        self.state().offline = offline;
    }

    /// Whether this computer has Touch ID. By default only on macOS.
    pub fn set_touch_id(&self, available: bool) {
        let mut state = self.state();
        state.touch_id = available;
        for account in &mut state.accounts {
            account.settings.touch_id = available.then_some(true);
        }
    }

    /// Unlocks an account without a password, to start the interface in
    /// the vault.
    pub fn unlock_now(&self, account: &AccountId) {
        if let Ok(account) = self.state().account_mut(account) {
            account.state = LockState::Unlocked;
            account.last_use = Instant::now();
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    // Waits out the latency, then hands out the state, or the error that
    // fail_next asked for.
    fn call(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        let latency = self.state().latency;
        if !latency.is_zero() {
            std::thread::sleep(latency);
        }
        let mut state = self.state();
        state.lock_idle_accounts();
        match state.fail_next.take() {
            Some(error) => Err(error),
            None => Ok(state),
        }
    }

    fn online(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        let state = self.call()?;
        if state.offline {
            return Err(Error::Offline);
        }
        Ok(state)
    }
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    fn lock_idle_accounts(&mut self) {
        for account in &mut self.accounts {
            if account.state == LockState::Unlocked
                && account.last_use.elapsed() >= account.settings.lock_timeout
            {
                account.lock();
            }
        }
    }

    fn account_mut(&mut self, id: &AccountId) -> Result<&mut FakeAccount> {
        self.accounts
            .iter_mut()
            .find(|account| account.id == *id)
            .ok_or(Error::AccountNotFound)
    }

    // The account, which must be unlocked; using it keeps it unlocked.
    fn unlocked(&mut self, id: &AccountId) -> Result<&mut FakeAccount> {
        let account = self.account_mut(id)?;
        match account.state {
            LockState::SignedOut => Err(Error::NotSignedIn),
            LockState::Locked => Err(Error::Locked),
            LockState::Unlocked => {
                account.last_use = Instant::now();
                Ok(account)
            }
        }
    }

    fn new_id(&mut self) -> String {
        self.next_id += 1;
        format!("f0000000-0000-4000-8000-{:012x}", self.next_id)
    }
}

impl FakeAccount {
    fn public(&self, touch_id: bool) -> Account {
        Account {
            id: self.id.clone(),
            email: Some(self.email.clone()),
            server: self.server.clone(),
            state: self.state,
            touch_id_ready: touch_id
                && self.touch_id_ready
                && self.settings.touch_id == Some(true)
                && self.state == LockState::Locked,
            last_sync: self.last_sync,
        }
    }

    fn item_mut(&mut self, id: &ItemId) -> Result<&mut Item> {
        self.items
            .iter_mut()
            .find(|item| item.id == *id)
            .ok_or(Error::NotFound)
    }

    fn lock(&mut self) {
        self.state = LockState::Locked;
        self.unlocked_items.clear();
    }

    // The item, with its secrets: one that asks for the password again
    // must be unlocked, and reading it keeps it unlocked.
    fn readable_item_mut(&mut self, id: &ItemId) -> Result<&mut Item> {
        let reprompt = self.item_mut(id)?.reprompt;
        if reprompt {
            match self.unlocked_items.get_mut(id) {
                Some(last_read) if last_read.elapsed() < ITEM_UNLOCK_IDLE => {
                    *last_read = Instant::now();
                }
                _ => return Err(Error::RepromptRequired),
            }
        }
        self.item_mut(id)
    }

    fn editable_item_mut(&mut self, id: &ItemId) -> Result<&mut Item> {
        let item = self.item_mut(id)?;
        if !item.editable {
            return Err(Error::ReadOnly);
        }
        Ok(item)
    }

    fn check(&self, data: &ItemData) -> Result<()> {
        if data.name.trim().is_empty() {
            return Err(Error::InvalidItem("an item needs a name".into()));
        }
        if let Some(folder) = &data.folder
            && !self.folders.iter().any(|known| known.id == *folder)
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    fn sign_in(&mut self, password: &Secret, code: Option<&LoginCode>) -> Result<()> {
        if self.state != LockState::SignedOut {
            return self.unlock(password);
        }
        if password.expose() != FakeBackend::PASSWORD {
            return Err(Error::WrongPassword);
        }
        let new_device = self.email.contains("+device@");
        match code {
            None if new_device => return Err(Error::NewDeviceCodeRequired),
            None => {
                return Err(Error::TwoStepRequired {
                    methods: vec![TwoStepMethod::Authenticator, TwoStepMethod::Email],
                });
            }
            Some(LoginCode::NewDevice(code) | LoginCode::TwoStep { code, .. })
                if code.trim() != FakeBackend::CODE =>
            {
                return Err(Error::WrongCode);
            }
            Some(LoginCode::TwoStep { .. }) if new_device => {
                return Err(Error::NewDeviceCodeRequired);
            }
            Some(_) => {}
        }
        self.state = LockState::Unlocked;
        self.touch_id_ready = true;
        self.last_use = Instant::now();
        self.last_sync = Some(SystemTime::now());
        Ok(())
    }

    fn unlock(&mut self, password: &Secret) -> Result<()> {
        match self.state {
            LockState::SignedOut => Err(Error::NotSignedIn),
            _ if password.expose() != FakeBackend::PASSWORD => Err(Error::WrongPassword),
            _ => {
                self.state = LockState::Unlocked;
                self.touch_id_ready = true;
                self.last_use = Instant::now();
                Ok(())
            }
        }
    }
}

fn summary(item: &Item) -> ItemSummary {
    let (subtitle, websites, has_code) = match &item.data.content {
        ItemContent::Login(login) => (
            Some(login.username.clone()),
            login.websites.iter().map(|site| site.uri.clone()).collect(),
            !login.totp.is_empty(),
        ),
        ItemContent::SecureNote => (None, vec![], false),
        ItemContent::Card(card) => (Some(card.brand.clone()), vec![], false),
        ItemContent::Identity(identity) => (Some(identity.full_name()), vec![], false),
        ItemContent::SshKey(key) => (Some(key.fingerprint.clone()), vec![], false),
    };
    ItemSummary {
        id: item.id.clone(),
        kind: item.kind(),
        name: item.data.name.clone(),
        subtitle: subtitle.filter(|subtitle| !subtitle.is_empty()),
        websites,
        folder: item.data.folder.clone(),
        organization: item.organization.clone(),
        favorite: item.favorite,
        has_code,
        editable: item.editable,
        reprompt: item.reprompt,
    }
}

// Saves what the real backend derives from a private key.
fn complete(mut data: ItemData) -> ItemData {
    if let ItemContent::SshKey(key) = &mut data.content
        && key.public_key.is_empty()
    {
        let (public_key, fingerprint) = demo::ssh_public_key(key.private_key.expose());
        key.public_key = public_key;
        key.fingerprint = fingerprint;
    }
    data
}

// A believable six-digit code that changes every period, from the secret.
fn totp(secret: &str, now: SystemTime) -> VerificationCode {
    let period = 30;
    let seconds = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let step = seconds / period;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in secret.bytes().chain(step.to_le_bytes()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    VerificationCode {
        code: Secret::new(format!("{:06}", hash % 1_000_000)),
        period: Duration::from_secs(period),
        expires_at: UNIX_EPOCH + Duration::from_secs((step + 1) * period),
    }
}

fn generate(options: &GeneratorOptions) -> Result<Secret> {
    const SYMBOLS: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";
    const DIGITS: &str = "0123456789";
    const LETTERS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const UNAMBIGUOUS: &str = "34678abcdefhjkmnpqrtuwxy";
    let mut rng = rand::rng();
    let value = match *options {
        GeneratorOptions::Password { length, characters } => {
            let alphabet: Vec<char> = match characters {
                Characters::All => [SYMBOLS, DIGITS, LETTERS].concat(),
                Characters::NoSymbols => [DIGITS, LETTERS].concat(),
                Characters::DigitsOnly => DIGITS.to_string(),
                Characters::Unambiguous => UNAMBIGUOUS.to_string(),
            }
            .chars()
            .collect();
            (0..length)
                .map(|_| alphabet[rng.random_range(0..alphabet.len())])
                .collect()
        }
        GeneratorOptions::Passphrase { words } => (0..words)
            .filter_map(|_| demo::WORDS.choose(&mut rng).copied())
            .collect::<Vec<_>>()
            .join(" "),
    };
    if value.is_empty() {
        return Err(Error::Other("nothing to generate".into()));
    }
    Ok(Secret::new(value))
}

impl Backend for FakeBackend {
    fn accounts(&self) -> Result<Vec<Account>> {
        let state = self.call()?;
        Ok(state
            .accounts
            .iter()
            .map(|account| account.public(state.touch_id))
            .collect())
    }

    fn account(&self, account: &AccountId) -> Result<Account> {
        let mut state = self.call()?;
        let touch_id = state.touch_id;
        Ok(state.account_mut(account)?.public(touch_id))
    }

    fn add_account(&self, server: &Server, email: &str) -> Result<AccountId> {
        let mut state = self.call()?;
        let email = email.trim();
        if let Some(existing) = state
            .accounts
            .iter()
            .find(|account| account.email.eq_ignore_ascii_case(email) && account.server == *server)
        {
            return Err(Error::AccountExists(existing.id.clone()));
        }
        let id = if state.accounts.iter().any(|account| account.id.is_default()) {
            AccountId::for_email(email, |name| {
                state
                    .accounts
                    .iter()
                    .any(|account| account.id.profile() == name)
            })
        } else {
            AccountId::default()
        };
        let touch_id = state.touch_id;
        state.accounts.push(FakeAccount {
            id: id.clone(),
            email: email.to_string(),
            server: server.clone(),
            state: LockState::SignedOut,
            touch_id_ready: false,
            last_use: Instant::now(),
            last_sync: None,
            settings: AccountSettings {
                lock_timeout: Duration::from_secs(3600),
                touch_id: touch_id.then_some(false),
                ssh_confirm: false,
            },
            items: vec![],
            folders: vec![],
            unlocked_items: HashMap::new(),
        });
        Ok(id)
    }

    fn sign_in(
        &self,
        account: &AccountId,
        password: &Secret,
        code: Option<&LoginCode>,
    ) -> Result<()> {
        self.online()?.account_mut(account)?.sign_in(password, code)
    }

    fn send_email_code(&self, account: &AccountId, password: &Secret) -> Result<()> {
        let mut state = self.online()?;
        let account = state.account_mut(account)?;
        if account.state != LockState::SignedOut {
            return Err(Error::Other("already signed in".into()));
        }
        if password.expose() != Self::PASSWORD {
            return Err(Error::WrongPassword);
        }
        Ok(())
    }

    fn remove_account(&self, account: &AccountId) -> Result<()> {
        let mut state = self.call()?;
        state.account_mut(account)?;
        state.accounts.retain(|known| known.id != *account);
        if state.ssh_agent.as_ref() == Some(account) {
            state.ssh_agent = None;
        }
        Ok(())
    }

    fn unlock(&self, account: &AccountId, password: &Secret) -> Result<()> {
        self.call()?.account_mut(account)?.unlock(password)
    }

    fn unlock_with_touch_id(&self, account: &AccountId) -> Result<()> {
        let mut state = self.call()?;
        let touch_id = state.touch_id;
        let account = state.account_mut(account)?;
        if account.state == LockState::Unlocked {
            return Ok(());
        }
        if !account.public(touch_id).touch_id_ready {
            return Err(Error::TouchIdUnavailable);
        }
        account.state = LockState::Unlocked;
        account.last_use = Instant::now();
        Ok(())
    }

    fn lock(&self, account: &AccountId) -> Result<()> {
        let mut state = self.call()?;
        let account = state.account_mut(account)?;
        if account.state == LockState::Unlocked {
            account.lock();
        }
        Ok(())
    }

    fn sync(&self, account: &AccountId) -> Result<()> {
        self.online()?.unlocked(account)?.last_sync = Some(SystemTime::now());
        Ok(())
    }

    fn items(&self, account: &AccountId) -> Result<Vec<ItemSummary>> {
        Ok(self
            .call()?
            .unlocked(account)?
            .items
            .iter()
            .map(summary)
            .collect())
    }

    fn item(&self, account: &AccountId, item: &ItemId) -> Result<Item> {
        Ok(self
            .call()?
            .unlocked(account)?
            .readable_item_mut(item)?
            .clone())
    }

    fn unlock_item(&self, account: &AccountId, item: &ItemId, password: &Secret) -> Result<()> {
        let mut state = self.call()?;
        let account = state.unlocked(account)?;
        account.item_mut(item)?;
        if password.expose() != Self::PASSWORD {
            return Err(Error::WrongPassword);
        }
        account.unlocked_items.insert(item.clone(), Instant::now());
        Ok(())
    }

    fn create_item(&self, account: &AccountId, data: &ItemData) -> Result<ItemSummary> {
        let mut state = self.online()?;
        let id = ItemId(state.new_id());
        let account = state.unlocked(account)?;
        account.check(data)?;
        let item = Item {
            id,
            data: complete(data.clone()),
            organization: None,
            favorite: false,
            editable: true,
            reprompt: false,
        };
        let summary = summary(&item);
        account.items.push(item);
        account.last_sync = Some(SystemTime::now());
        Ok(summary)
    }

    fn edit_item(
        &self,
        account: &AccountId,
        item: &ItemId,
        data: &ItemData,
    ) -> Result<ItemSummary> {
        let mut state = self.online()?;
        let account = state.unlocked(account)?;
        account.check(data)?;
        account.readable_item_mut(item)?;
        let item = account.editable_item_mut(item)?;
        if item.kind() != data.content.kind() {
            return Err(Error::InvalidItem("an item's kind can't change".into()));
        }
        item.data = complete(data.clone());
        let summary = summary(item);
        account.last_sync = Some(SystemTime::now());
        Ok(summary)
    }

    fn delete_item(&self, account: &AccountId, item: &ItemId) -> Result<()> {
        let mut state = self.online()?;
        let account = state.unlocked(account)?;
        account.editable_item_mut(item)?;
        account.items.retain(|known| known.id != *item);
        account.last_sync = Some(SystemTime::now());
        Ok(())
    }

    fn set_favorite(&self, account: &AccountId, item: &ItemId, favorite: bool) -> Result<()> {
        let mut state = self.online()?;
        let account = state.unlocked(account)?;
        account.item_mut(item)?.favorite = favorite;
        account.last_sync = Some(SystemTime::now());
        Ok(())
    }

    fn move_item(
        &self,
        account: &AccountId,
        item: &ItemId,
        folder: Option<&FolderId>,
    ) -> Result<()> {
        let mut state = self.online()?;
        let account = state.unlocked(account)?;
        if let Some(folder) = folder
            && !account.folders.iter().any(|known| known.id == *folder)
        {
            return Err(Error::NotFound);
        }
        account.item_mut(item)?.data.folder = folder.cloned();
        account.last_sync = Some(SystemTime::now());
        Ok(())
    }

    fn folders(&self, account: &AccountId) -> Result<Vec<Folder>> {
        Ok(self.call()?.unlocked(account)?.folders.clone())
    }

    fn create_folder(&self, account: &AccountId, name: &str) -> Result<Folder> {
        let mut state = self.online()?;
        let id = FolderId(state.new_id());
        let account = state.unlocked(account)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::InvalidItem("a folder needs a name".into()));
        }
        let folder = Folder {
            id,
            name: name.to_string(),
        };
        account.folders.push(folder.clone());
        account.last_sync = Some(SystemTime::now());
        Ok(folder)
    }

    fn verification_code(&self, account: &AccountId, item: &ItemId) -> Result<VerificationCode> {
        let mut state = self.call()?;
        let item = state.unlocked(account)?.readable_item_mut(item)?;
        match &item.data.content {
            ItemContent::Login(login) if !login.totp.is_empty() => {
                Ok(totp(login.totp.expose(), SystemTime::now()))
            }
            _ => Err(Error::NotFound),
        }
    }

    fn generate(&self, options: &GeneratorOptions) -> Result<Secret> {
        drop(self.call()?);
        generate(options)
    }

    fn account_settings(&self, account: &AccountId) -> Result<AccountSettings> {
        Ok(self.call()?.account_mut(account)?.settings.clone())
    }

    fn set_account_settings(&self, account: &AccountId, settings: &AccountSettings) -> Result<()> {
        let mut state = self.call()?;
        let touch_id = state.touch_id;
        let account = state.account_mut(account)?;
        if settings.lock_timeout.is_zero() {
            return Err(Error::Other("the lock timeout must be positive".into()));
        }
        if settings.touch_id.is_some() && !touch_id {
            return Err(Error::TouchIdUnavailable);
        }
        if settings.touch_id != Some(true) {
            account.touch_id_ready = false;
        }
        account.settings = settings.clone();
        Ok(())
    }

    fn preferences(&self) -> Result<Preferences> {
        Ok(self.call()?.preferences.clone())
    }

    fn set_preferences(&self, preferences: &Preferences) -> Result<()> {
        self.call()?.preferences = preferences.clone();
        Ok(())
    }

    fn ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus> {
        let mut state = self.call()?;
        state.ssh_status(account)
    }

    fn install_ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus> {
        let mut state = self.call()?;
        state.account_mut(account)?;
        match &state.ssh_agent {
            Some(other) if other != account => {
                return Err(Error::SshAgentInstalledForOther(other.clone()));
            }
            _ => state.ssh_agent = Some(account.clone()),
        }
        state.ssh_status(account)
    }

    fn uninstall_ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus> {
        let mut state = self.call()?;
        if state.ssh_agent.as_ref() == Some(account) {
            state.ssh_agent = None;
        }
        state.ssh_status(account)
    }

    fn rbw_version(&self) -> Result<String> {
        drop(self.call()?);
        Ok("demo".to_string())
    }
}

impl State {
    fn ssh_status(&mut self, id: &AccountId) -> Result<SshAgentStatus> {
        let installed = self.ssh_agent.clone();
        let account = self.account_mut(id)?;
        let keys = (account.state == LockState::Unlocked).then(|| {
            account
                .items
                .iter()
                .filter_map(|item| match &item.data.content {
                    ItemContent::SshKey(key) => Some(SshKeySummary {
                        item: item.id.clone(),
                        name: item.data.name.clone(),
                        fingerprint: key.fingerprint.clone(),
                    }),
                    _ => None,
                })
                .collect()
        });
        let socket = PathBuf::from(format!(
            "/run/user/1000/rbw-{}/ssh-agent-socket",
            id.profile()
        ));
        let (service, running, ssh_uses) = match installed {
            Some(owner) if owner == *id => (SshService::Installed, true, SshUses::Rbw),
            Some(owner) => (
                SshService::InstalledForOther(owner),
                false,
                SshUses::Other(PathBuf::from("/run/user/1000/rbw-other/ssh-agent-socket")),
            ),
            None => (
                SshService::NotInstalled,
                account.state == LockState::Unlocked,
                SshUses::Other(PathBuf::from("/run/user/1000/keyring/ssh")),
            ),
        };
        Ok(SshAgentStatus {
            service,
            running,
            socket,
            ssh_uses,
            keys,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FieldValue, ItemKind, Login};

    fn personal() -> AccountId {
        AccountId::new(demo::PERSONAL)
    }

    fn unlocked() -> FakeBackend {
        let fake = FakeBackend::new();
        fake.unlock(&personal(), &FakeBackend::PASSWORD.into())
            .unwrap();
        fake
    }

    #[test]
    fn demo_data_is_believable() {
        let fake = unlocked();
        let items = fake.items(&personal()).unwrap();
        assert!((35..=45).contains(&items.len()), "{}", items.len());
        for kind in [
            ItemKind::Login,
            ItemKind::SecureNote,
            ItemKind::Card,
            ItemKind::Identity,
            ItemKind::SshKey,
        ] {
            assert!(items.iter().any(|item| item.kind == kind), "{kind:?}");
        }
        assert!(items.iter().any(|item| !item.editable));
        assert!(items.iter().any(|item| item.has_code));
        assert!(items.iter().any(|item| item.favorite));
        let folders = fake.folders(&personal()).unwrap();
        for item in &items {
            if let Some(folder) = &item.folder {
                assert!(folders.iter().any(|known| known.id == *folder));
            }
        }
        assert_eq!(fake.accounts().unwrap().len(), 2);
    }

    #[test]
    fn locking() {
        let fake = FakeBackend::new();
        let id = personal();
        assert_eq!(fake.items(&id), Err(Error::Locked));
        assert_eq!(fake.unlock(&id, &"nope".into()), Err(Error::WrongPassword));
        fake.unlock(&id, &FakeBackend::PASSWORD.into()).unwrap();
        assert_eq!(fake.account(&id).unwrap().state, LockState::Unlocked);
        fake.lock(&id).unwrap();
        assert_eq!(fake.account(&id).unwrap().state, LockState::Locked);

        fake.set_touch_id(false);
        let mut settings = fake.account_settings(&id).unwrap();
        assert_eq!(settings.touch_id, None);
        settings.lock_timeout = Duration::from_millis(20);
        fake.set_account_settings(&id, &settings).unwrap();
        fake.unlock(&id, &FakeBackend::PASSWORD.into()).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(fake.account(&id).unwrap().state, LockState::Locked);
    }

    #[test]
    fn touch_id_after_a_password_unlock() {
        let fake = FakeBackend::new();
        fake.set_touch_id(true);
        let id = personal();
        assert!(!fake.account(&id).unwrap().touch_id_ready);
        assert_eq!(
            fake.unlock_with_touch_id(&id),
            Err(Error::TouchIdUnavailable)
        );
        fake.unlock(&id, &FakeBackend::PASSWORD.into()).unwrap();
        fake.lock(&id).unwrap();
        assert!(fake.account(&id).unwrap().touch_id_ready);
        fake.unlock_with_touch_id(&id).unwrap();
        assert_eq!(fake.account(&id).unwrap().state, LockState::Unlocked);
    }

    #[test]
    fn adding_an_account_asks_for_a_code() {
        let fake = FakeBackend::empty();
        let id = fake
            .add_account(&Server::BitwardenEu, "sam@example.com")
            .unwrap();
        assert!(id.is_default());
        assert_eq!(fake.account(&id).unwrap().state, LockState::SignedOut);
        let password = Secret::from(FakeBackend::PASSWORD);
        let Err(Error::TwoStepRequired { methods }) = fake.sign_in(&id, &password, None) else {
            panic!("no two-step login");
        };
        assert!(methods.contains(&TwoStepMethod::Email));
        fake.send_email_code(&id, &password).unwrap();
        let wrong = LoginCode::TwoStep {
            method: TwoStepMethod::Email,
            code: "000000".into(),
        };
        assert_eq!(
            fake.sign_in(&id, &password, Some(&wrong)),
            Err(Error::WrongCode)
        );
        let right = LoginCode::TwoStep {
            method: TwoStepMethod::Email,
            code: FakeBackend::CODE.into(),
        };
        fake.sign_in(&id, &password, Some(&right)).unwrap();
        assert_eq!(fake.account(&id).unwrap().state, LockState::Unlocked);
        assert!(fake.items(&id).unwrap().is_empty());

        let other = fake
            .add_account(&Server::BitwardenCom, "sam+device@example.com")
            .unwrap();
        assert_eq!(other.profile(), "sam-device-example-com");
        assert_eq!(
            fake.sign_in(&other, &password, None),
            Err(Error::NewDeviceCodeRequired)
        );
        assert_eq!(
            fake.add_account(&Server::BitwardenEu, "SAM@example.com"),
            Err(Error::AccountExists(id.clone()))
        );
        fake.remove_account(&id).unwrap();
        assert_eq!(fake.account(&id), Err(Error::AccountNotFound));
    }

    #[test]
    fn editing_items() {
        let fake = unlocked();
        let id = personal();
        let mut data = ItemData::new(ItemKind::Login);
        data.name = "Example".into();
        data.fields.push(crate::CustomField {
            name: "PIN".into(),
            value: FieldValue::Hidden("1234".into()),
        });
        data.content = ItemContent::Login(Login {
            username: "me".into(),
            password: "pw".into(),
            ..Login::default()
        });
        let created = fake.create_item(&id, &data).unwrap();
        assert_eq!(created.subtitle.as_deref(), Some("me"));
        assert_eq!(fake.item(&id, &created.id).unwrap().data, data);

        data.name = "Renamed".into();
        fake.edit_item(&id, &created.id, &data).unwrap();
        fake.set_favorite(&id, &created.id, true).unwrap();
        let item = fake.item(&id, &created.id).unwrap();
        assert_eq!(item.data.name, "Renamed");
        assert!(item.favorite);

        data.content = ItemContent::SecureNote;
        assert!(matches!(
            fake.edit_item(&id, &created.id, &data),
            Err(Error::InvalidItem(_))
        ));
        fake.delete_item(&id, &created.id).unwrap();
        assert_eq!(fake.item(&id, &created.id), Err(Error::NotFound));

        let read_only = fake
            .items(&id)
            .unwrap()
            .into_iter()
            .find(|item| !item.editable)
            .unwrap();
        assert_eq!(fake.delete_item(&id, &read_only.id), Err(Error::ReadOnly));

        let folder = fake.create_folder(&id, "Games").unwrap();
        fake.set_favorite(&id, &read_only.id, true).unwrap();
        fake.move_item(&id, &read_only.id, Some(&folder.id))
            .unwrap();
        let moved = fake.item(&id, &read_only.id).unwrap();
        assert!(moved.favorite);
        assert_eq!(moved.data.folder, Some(folder.id.clone()));
        assert_eq!(
            fake.move_item(&id, &read_only.id, Some(&FolderId("nope".into()))),
            Err(Error::NotFound)
        );
        fake.move_item(&id, &read_only.id, None).unwrap();
        assert_eq!(fake.item(&id, &read_only.id).unwrap().data.folder, None);
        let mut key = ItemData::new(ItemKind::SshKey);
        key.name = "Laptop".into();
        key.folder = Some(folder.id);
        key.content = ItemContent::SshKey(crate::SshKey {
            private_key: "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n".into(),
            ..crate::SshKey::default()
        });
        let key = fake.create_item(&id, &key).unwrap();
        assert!(key.subtitle.unwrap().starts_with("SHA256:"));
    }

    #[test]
    fn items_that_ask_for_the_password_again() {
        let fake = unlocked();
        let id = personal();
        let bank = fake
            .items(&id)
            .unwrap()
            .into_iter()
            .find(|item| item.reprompt && item.has_code)
            .unwrap();
        assert_eq!(fake.item(&id, &bank.id), Err(Error::RepromptRequired));
        assert_eq!(
            fake.verification_code(&id, &bank.id),
            Err(Error::RepromptRequired)
        );
        assert_eq!(
            fake.unlock_item(&id, &bank.id, &"nope".into()),
            Err(Error::WrongPassword)
        );
        fake.unlock_item(&id, &bank.id, &FakeBackend::PASSWORD.into())
            .unwrap();
        let item = fake.item(&id, &bank.id).unwrap();
        assert!(item.reprompt);
        fake.verification_code(&id, &bank.id).unwrap();
        fake.edit_item(&id, &bank.id, &item.data).unwrap();
        fake.lock(&id).unwrap();
        fake.unlock(&id, &FakeBackend::PASSWORD.into()).unwrap();
        assert_eq!(fake.item(&id, &bank.id), Err(Error::RepromptRequired));
    }

    #[test]
    fn codes_and_generator() {
        let fake = unlocked();
        let id = personal();
        let item = fake
            .items(&id)
            .unwrap()
            .into_iter()
            .find(|item| item.has_code)
            .unwrap();
        let code = fake.verification_code(&id, &item.id).unwrap();
        assert_eq!(code.code.expose().len(), 6);
        assert!(code.expires_at > SystemTime::now());

        let password = fake
            .generate(&GeneratorOptions::Password {
                length: 24,
                characters: Characters::DigitsOnly,
            })
            .unwrap();
        assert_eq!(password.expose().len(), 24);
        assert!(password.expose().chars().all(|c| c.is_ascii_digit()));
        let phrase = fake
            .generate(&GeneratorOptions::Passphrase { words: 5 })
            .unwrap();
        assert_eq!(phrase.expose().split(' ').count(), 5);
    }

    #[test]
    fn knobs() {
        let fake = unlocked();
        let id = personal();
        fake.fail_next(Error::Timeout);
        assert_eq!(fake.items(&id), Err(Error::Timeout));
        assert!(fake.items(&id).is_ok());

        fake.set_offline(true);
        assert_eq!(fake.sync(&id), Err(Error::Offline));
        assert!(fake.items(&id).is_ok());
        fake.set_offline(false);

        fake.set_latency(Duration::from_millis(20));
        let start = Instant::now();
        fake.sync(&id).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(20));
    }

    #[test]
    fn one_ssh_agent() {
        let fake = unlocked();
        let personal = personal();
        let work = AccountId::new(demo::WORK);
        let status = fake.ssh_agent(&personal).unwrap();
        assert_eq!(status.service, SshService::Installed);
        assert!(!status.keys.unwrap().is_empty());
        assert_eq!(
            fake.install_ssh_agent(&work),
            Err(Error::SshAgentInstalledForOther(personal.clone()))
        );
        fake.uninstall_ssh_agent(&personal).unwrap();
        let status = fake.install_ssh_agent(&work).unwrap();
        assert_eq!(status.service, SshService::Installed);
        assert_eq!(status.keys, None);
    }
}
