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
    Account, AccountId, AccountSettings, Backend, CommandLineTool, Error, Folder, FolderId,
    GeneratorOptions, IconFormat, Item, ItemContent, ItemData, ItemId, ItemSummary, LockState,
    LoginCode, OtherRbw, Preferences, Result, Secret, Server, SshAgentStatus, SshKeySummary,
    SshService, SshUses, ToolState, TwoStepMethod, VerificationCode, WebsiteIcon, icon, ssh,
};

/// A call to the backend, to make that one fail or wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
    Accounts,
    Account,
    AddAccount,
    SignIn,
    RegisterDevice,
    SendEmailCode,
    RemoveAccount,
    Unlock,
    UnlockWithTouchId,
    Lock,
    Sync,
    Items,
    Item,
    UnlockItem,
    CreateItem,
    EditItem,
    DeleteItem,
    SetFavorite,
    MoveItem,
    Folders,
    CreateFolder,
    VerificationCode,
    WebsiteIcon,
    RecentItems,
    ItemUsed,
    Generate,
    AccountSettings,
    SetAccountSettings,
    Preferences,
    SetPreferences,
    SshAgent,
    InstallSshAgent,
    UninstallSshAgent,
    RbwVersion,
    CommandLineTool,
    SetCommandLineTool,
    OtherRbw,
}

pub struct FakeBackend {
    state: Mutex<State>,
}

struct State {
    accounts: Vec<FakeAccount>,
    preferences: Preferences,
    latency: Duration,
    latencies: HashMap<Operation, Duration>,
    fail_next: Option<Error>,
    failures: HashMap<Operation, Error>,
    offline: bool,
    touch_id: bool,
    // the account whose SSH agent is installed
    ssh_agent: Option<AccountId>,
    ssh_supported: bool,
    // what ssh uses instead of an installed agent
    ssh_elsewhere: Option<SshUses>,
    command_line: CommandLineTool,
    other_rbw: Option<OtherRbw>,
    next_id: u64,
}

struct FakeAccount {
    id: AccountId,
    email: String,
    server: Server,
    state: LockState,
    // set once the account was unlocked with its password
    touch_id_ready: bool,
    // set once register_device took the API key
    registered: bool,
    last_use: Instant,
    last_sync: Option<SystemTime>,
    settings: AccountSettings,
    items: Vec<Item>,
    folders: Vec<Folder>,
    // items that ask for the password again, unlocked, with their last read
    unlocked_items: HashMap<ItemId, Instant>,
    recent: Vec<ItemId>,
    // when items were created or last edited here
    modified: HashMap<ItemId, SystemTime>,
    // the websites' icons fetched, as rbw keeps them on disk
    icons: HashMap<String, Option<WebsiteIcon>>,
}

// How long an unlocked item that asks for the password again stays
// unlocked without being read, as in rbw-agent.
const ITEM_UNLOCK_IDLE: Duration = Duration::from_secs(120);

impl FakeBackend {
    /// The master password of every demo account.
    pub const PASSWORD: &str = "demo";
    /// The two-step and new device code the demo accepts. Signing in asks
    /// for a two-step code, or a new device code when the email has
    /// `+device@`, or to register the device first when it has `+apikey@`.
    /// A self-hosted server whose address has `.invalid` in it is no
    /// Bitwarden server.
    pub const CODE: &str = "123456";
    /// The API key that registers the device of every demo account.
    pub const API_KEY_ID: &str = "user.d3m0";
    pub const API_KEY_SECRET: &str = "demo-secret";

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
                registered: true,
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
                recent: demo.recent,
                modified: HashMap::new(),
                icons: HashMap::new(),
            })
            .collect();
        Self {
            state: Mutex::new(State {
                accounts,
                preferences: Preferences::default(),
                latency: Duration::ZERO,
                latencies: HashMap::new(),
                fail_next: None,
                failures: HashMap::new(),
                offline: false,
                touch_id,
                ssh_agent: Some(AccountId::new(demo::PERSONAL)),
                ssh_supported: true,
                ssh_elsewhere: None,
                command_line: CommandLineTool {
                    path: if cfg!(target_os = "macos") {
                        PathBuf::from("/usr/local/bin")
                    } else {
                        std::env::var_os("HOME")
                            .map_or_else(|| PathBuf::from("/home/demo"), PathBuf::from)
                            .join(".local/bin")
                    }
                    .join("rbw"),
                    state: ToolState::Off,
                    on_path: true,
                },
                other_rbw: None,
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

    /// Makes every call of one operation take this long, whatever the
    /// latency of the others.
    pub fn set_latency_of(&self, operation: Operation, latency: Duration) {
        self.state().latencies.insert(operation, latency);
    }

    /// Makes the next call fail with this error, whatever it is.
    pub fn fail_next(&self, error: Error) {
        self.state().fail_next = Some(error);
    }

    /// Makes every call of one operation fail with this error, until
    /// [`FakeBackend::succeed`].
    pub fn fail(&self, operation: Operation, error: Error) {
        self.state().failures.insert(operation, error);
    }

    /// Lets an operation that [`FakeBackend::fail`] made fail work again.
    pub fn succeed(&self, operation: Operation) {
        self.state().failures.remove(&operation);
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

    /// Whether this system can run the SSH agent as a service. When it
    /// can't, its status says so and installing it fails with
    /// [`Error::SshAgentUnsupported`].
    pub fn set_ssh_supported(&self, supported: bool) {
        let mut state = self.state();
        state.ssh_supported = supported;
        if !supported {
            state.ssh_agent = None;
        }
    }

    /// Makes `ssh` use something else than an installed agent, as when the
    /// login session hasn't picked it up yet; `None` lets it use the agent.
    pub fn set_ssh_uses(&self, uses: Option<SshUses>) {
        self.state().ssh_elsewhere = uses;
    }

    /// Makes [`Backend::other_rbw`] find this one.
    pub fn set_other_rbw(&self, other: Option<OtherRbw>) {
        self.state().other_rbw = other;
    }

    /// Whether a terminal looks in the command line tool's folder.
    pub fn set_command_line_on_path(&self, on_path: bool) {
        self.state().command_line.on_path = on_path;
    }

    /// Signs an account out, keeping it on this computer with its email
    /// and server, as when its session ended: it needs
    /// [`Backend::sign_in`] again.
    pub fn sign_out_now(&self, account: &AccountId) {
        if let Ok(account) = self.state().account_mut(account) {
            account.lock();
            account.state = LockState::SignedOut;
        }
    }

    /// Adds `count` logins to an account, each with a website of its own,
    /// to see the interface with a vault as big as the largest ones.
    pub fn add_logins(&self, account: &AccountId, count: usize) {
        if let Ok(account) = self.state().account_mut(account) {
            account.items.extend(demo::many_logins(count));
        }
    }

    /// Keeps the icons of the account's websites as if they had been
    /// fetched before, as on a computer the vault was opened on already.
    pub fn keep_icons(&self, account: &AccountId) {
        if let Ok(account) = self.state().account_mut(account) {
            let hosts: Vec<String> = account
                .items
                .iter()
                .filter_map(|item| icon::icon_host(&summary(item, SystemTime::UNIX_EPOCH)))
                .collect();
            for host in hosts {
                let icon = demo_icon(&host);
                account.icons.insert(host, icon);
            }
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
    // fail_next or fail asked for.
    fn call(&self, operation: Operation) -> Result<std::sync::MutexGuard<'_, State>> {
        let latency = {
            let state = self.state();
            state
                .latencies
                .get(&operation)
                .copied()
                .unwrap_or(state.latency)
        };
        if !latency.is_zero() {
            std::thread::sleep(latency);
        }
        let mut state = self.state();
        state.lock_idle_accounts();
        if let Some(error) = state.fail_next.take() {
            return Err(error);
        }
        match state.failures.get(&operation) {
            Some(error) => Err(error.clone()),
            None => Ok(state),
        }
    }

    fn online(&self, operation: Operation) -> Result<std::sync::MutexGuard<'_, State>> {
        let state = self.call(operation)?;
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

    fn summaries(&self) -> Vec<ItemSummary> {
        self.items
            .iter()
            .map(|item| summary(item, self.modified(&item.id)))
            .collect()
    }

    /// When an item was last changed: when it was created or edited here,
    /// else, for the demo's items, a time some hours to a few months ago
    /// that follows from its id.
    fn modified(&self, id: &ItemId) -> SystemTime {
        self.modified.get(id).copied().unwrap_or_else(|| {
            let hash = id.0.bytes().fold(0u64, |hash, byte| {
                hash.wrapping_mul(31).wrapping_add(u64::from(byte))
            });
            SystemTime::now() - Duration::from_secs(3600 * (1 + hash % 2000))
        })
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
        if let Server::SelfHosted(url) = &self.server
            && url.contains(".invalid")
        {
            return Err(Error::WrongServer);
        }
        if password.expose() != FakeBackend::PASSWORD {
            return Err(Error::WrongPassword);
        }
        if self.email.contains("+apikey@") && !self.registered {
            return Err(Error::RegistrationRequired);
        }
        // registering makes the device known
        let new_device = self.email.contains("+device@") && !self.registered;
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

fn summary(item: &Item, modified: SystemTime) -> ItemSummary {
    let (subtitle, websites, has_code) = match &item.data.content {
        ItemContent::Login(login) => (
            Some(login.username.clone()),
            login.websites.iter().map(|site| site.uri.clone()).collect(),
            !login.totp.is_empty(),
        ),
        ItemContent::SecureNote => (None, vec![], false),
        ItemContent::Card(card) => (Some(card.brand.clone()), vec![], false),
        ItemContent::Identity(identity) => (Some(identity.full_name()), vec![], false),
        ItemContent::SshKey(key) => (ssh::key_subtitle(&key.public_key), vec![], false),
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
        modified: Some(modified),
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

// The icon of a demo website, drawn for the demo (design/demo-icons), so
// that the interface shows icons without a server. Only the made-up
// websites have one: the others show their letter.
fn demo_icon(host: &str) -> Option<WebsiteIcon> {
    let bytes: &'static [u8] = match host {
        "online.bank.example" => include_bytes!("fake/icons/bank.png"),
        "www.airline.example" => include_bytes!("fake/icons/airline.png"),
        "rail.example" => include_bytes!("fake/icons/rail.png"),
        "bookshop.example" => include_bytes!("fake/icons/bookshop.png"),
        "www.energy.example" => include_bytes!("fake/icons/energy.png"),
        "broker.example" => include_bytes!("fake/icons/broker.png"),
        "tax.example.gov" => include_bytes!("fake/icons/tax.png"),
        "social.example" => include_bytes!("fake/icons/social.png"),
        "forum.example.net" => include_bytes!("fake/icons/forum.png"),
        "mail.studio.example" => include_bytes!("fake/icons/studio-mail.png"),
        _ => return None,
    };
    Some(WebsiteIcon {
        format: IconFormat::Png,
        bytes: bytes.into(),
    })
}

// As rbw does: a password that must hold each class is drawn again until
// it does.
fn generate(options: &GeneratorOptions) -> Result<Secret> {
    if !options.makes_anything() {
        return Err(Error::Other("these options make nothing".into()));
    }
    let mut rng = rand::rng();
    let value = match *options {
        GeneratorOptions::Password { length, characters } => {
            let classes = characters.classes();
            let alphabet: Vec<char> = classes.concat().chars().collect();
            loop {
                let value: String = (0..length)
                    .map(|_| alphabet[rng.random_range(0..alphabet.len())])
                    .collect();
                if !characters.each_class()
                    || classes
                        .iter()
                        .all(|class| value.chars().any(|c| class.contains(c)))
                {
                    break value;
                }
            }
        }
        GeneratorOptions::Passphrase {
            words,
            separator,
            capitalize,
            number,
        } => {
            let mut words: Vec<String> = (0..words)
                .filter_map(|_| demo::WORDS.choose(&mut rng))
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        Some(first) if capitalize => first.to_uppercase().chain(chars).collect(),
                        _ => (*word).to_string(),
                    }
                })
                .collect();
            if number {
                let at = rng.random_range(0..words.len());
                words[at].push(char::from(b'0' + rng.random_range(0..10)));
            }
            words.join(separator.map(String::from).unwrap_or_default().as_str())
        }
    };
    Ok(Secret::new(value))
}

impl Backend for FakeBackend {
    fn accounts(&self) -> Result<Vec<Account>> {
        let state = self.call(Operation::Accounts)?;
        Ok(state
            .accounts
            .iter()
            .map(|account| account.public(state.touch_id))
            .collect())
    }

    fn account(&self, account: &AccountId) -> Result<Account> {
        let mut state = self.call(Operation::Account)?;
        let touch_id = state.touch_id;
        Ok(state.account_mut(account)?.public(touch_id))
    }

    fn add_account(&self, server: &Server, email: &str) -> Result<AccountId> {
        let mut state = self.call(Operation::AddAccount)?;
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
            registered: false,
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
            recent: vec![],
            modified: HashMap::new(),
            icons: HashMap::new(),
        });
        Ok(id)
    }

    fn sign_in(
        &self,
        account: &AccountId,
        password: &Secret,
        code: Option<&LoginCode>,
    ) -> Result<()> {
        self.online(Operation::SignIn)?
            .account_mut(account)?
            .sign_in(password, code)
    }

    fn register_device(
        &self,
        account: &AccountId,
        client_id: &str,
        client_secret: &Secret,
    ) -> Result<()> {
        let mut state = self.online(Operation::RegisterDevice)?;
        let account = state.account_mut(account)?;
        if account.state != LockState::SignedOut {
            return Ok(());
        }
        if client_id.trim() != Self::API_KEY_ID || client_secret.expose() != Self::API_KEY_SECRET {
            return Err(Error::WrongApiKey);
        }
        account.registered = true;
        Ok(())
    }

    fn send_email_code(&self, account: &AccountId, password: &Secret) -> Result<()> {
        let mut state = self.online(Operation::SendEmailCode)?;
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
        let mut state = self.call(Operation::RemoveAccount)?;
        state.account_mut(account)?;
        state.accounts.retain(|known| known.id != *account);
        if state.ssh_agent.as_ref() == Some(account) {
            state.ssh_agent = None;
        }
        Ok(())
    }

    fn unlock(&self, account: &AccountId, password: &Secret) -> Result<()> {
        self.call(Operation::Unlock)?
            .account_mut(account)?
            .unlock(password)
    }

    fn unlock_with_touch_id(&self, account: &AccountId) -> Result<()> {
        let mut state = self.call(Operation::UnlockWithTouchId)?;
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
        let mut state = self.call(Operation::Lock)?;
        let account = state.account_mut(account)?;
        if account.state == LockState::Unlocked {
            account.lock();
        }
        Ok(())
    }

    fn sync(&self, account: &AccountId) -> Result<()> {
        self.online(Operation::Sync)?.unlocked(account)?.last_sync = Some(SystemTime::now());
        Ok(())
    }

    fn items(&self, account: &AccountId) -> Result<Vec<ItemSummary>> {
        Ok(self.call(Operation::Items)?.unlocked(account)?.summaries())
    }

    fn item(&self, account: &AccountId, item: &ItemId) -> Result<Item> {
        Ok(self
            .call(Operation::Item)?
            .unlocked(account)?
            .readable_item_mut(item)?
            .clone())
    }

    fn unlock_item(&self, account: &AccountId, item: &ItemId, password: &Secret) -> Result<()> {
        let mut state = self.call(Operation::UnlockItem)?;
        let account = state.unlocked(account)?;
        account.item_mut(item)?;
        if password.expose() != Self::PASSWORD {
            return Err(Error::WrongPassword);
        }
        account.unlocked_items.insert(item.clone(), Instant::now());
        Ok(())
    }

    fn create_item(&self, account: &AccountId, data: &ItemData) -> Result<ItemSummary> {
        let mut state = self.online(Operation::CreateItem)?;
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
        account.modified.insert(item.id.clone(), SystemTime::now());
        let summary = summary(&item, account.modified(&item.id));
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
        let mut state = self.online(Operation::EditItem)?;
        let account = state.unlocked(account)?;
        account.check(data)?;
        account.readable_item_mut(item)?;
        let item = account.editable_item_mut(item)?;
        if item.kind() != data.content.kind() {
            return Err(Error::InvalidItem("an item's kind can't change".into()));
        }
        item.data = complete(data.clone());
        let id = item.id.clone();
        account.modified.insert(id.clone(), SystemTime::now());
        let modified = account.modified(&id);
        let summary = summary(account.item_mut(&id)?, modified);
        account.last_sync = Some(SystemTime::now());
        Ok(summary)
    }

    fn delete_item(&self, account: &AccountId, item: &ItemId) -> Result<()> {
        let mut state = self.online(Operation::DeleteItem)?;
        let account = state.unlocked(account)?;
        account.editable_item_mut(item)?;
        account.items.retain(|known| known.id != *item);
        account.last_sync = Some(SystemTime::now());
        Ok(())
    }

    fn set_favorite(&self, account: &AccountId, item: &ItemId, favorite: bool) -> Result<()> {
        let mut state = self.online(Operation::SetFavorite)?;
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
        let mut state = self.online(Operation::MoveItem)?;
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
        Ok(self
            .call(Operation::Folders)?
            .unlocked(account)?
            .folders
            .clone())
    }

    fn create_folder(&self, account: &AccountId, name: &str) -> Result<Folder> {
        let mut state = self.online(Operation::CreateFolder)?;
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
        let mut state = self.call(Operation::VerificationCode)?;
        let item = state.unlocked(account)?.readable_item_mut(item)?;
        match &item.data.content {
            ItemContent::Login(login) if !login.totp.is_empty() => {
                Ok(totp(login.totp.expose(), SystemTime::now()))
            }
            _ => Err(Error::NotFound),
        }
    }

    fn website_icon(&self, account: &AccountId, item: &ItemSummary) -> Result<Option<WebsiteIcon>> {
        let mut state = self.online(Operation::WebsiteIcon)?;
        state.account_mut(account)?;
        if !state.preferences.website_icons {
            return Ok(None);
        }
        let Some(host) = icon::icon_host(item) else {
            return Ok(None);
        };
        let icon = demo_icon(&host);
        state.account_mut(account)?.icons.insert(host, icon.clone());
        Ok(icon)
    }

    fn cached_website_icon(
        &self,
        account: &AccountId,
        item: &ItemSummary,
    ) -> Result<Option<Option<WebsiteIcon>>> {
        let mut state = self.state();
        if !state.preferences.website_icons {
            return Ok(Some(None));
        }
        let Some(host) = icon::icon_host(item) else {
            return Ok(Some(None));
        };
        Ok(state.account_mut(account)?.icons.get(&host).cloned())
    }

    fn recent_items(&self, account: &AccountId) -> Result<Vec<ItemId>> {
        Ok(self
            .call(Operation::RecentItems)?
            .account_mut(account)?
            .recent
            .clone())
    }

    fn item_used(&self, account: &AccountId, item: &ItemId) -> Result<()> {
        let mut state = self.call(Operation::ItemUsed)?;
        let recent = &mut state.account_mut(account)?.recent;
        recent.retain(|known| known != item);
        recent.insert(0, item.clone());
        recent.truncate(20);
        Ok(())
    }

    fn generate(&self, options: &GeneratorOptions) -> Result<Secret> {
        drop(self.call(Operation::Generate)?);
        generate(options)
    }

    fn account_settings(&self, account: &AccountId) -> Result<AccountSettings> {
        Ok(self
            .call(Operation::AccountSettings)?
            .account_mut(account)?
            .settings
            .clone())
    }

    fn set_account_settings(&self, account: &AccountId, settings: &AccountSettings) -> Result<()> {
        let mut state = self.call(Operation::SetAccountSettings)?;
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
        Ok(self.call(Operation::Preferences)?.preferences.clone())
    }

    fn set_preferences(&self, preferences: &Preferences) -> Result<()> {
        self.call(Operation::SetPreferences)?.preferences = preferences.clone();
        Ok(())
    }

    fn ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus> {
        let mut state = self.call(Operation::SshAgent)?;
        state.ssh_status(account)
    }

    fn install_ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus> {
        let mut state = self.call(Operation::InstallSshAgent)?;
        state.account_mut(account)?;
        if !state.ssh_supported {
            return Err(Error::SshAgentUnsupported);
        }
        match &state.ssh_agent {
            Some(other) if other != account => {
                return Err(Error::SshAgentInstalledForOther(other.clone()));
            }
            _ => state.ssh_agent = Some(account.clone()),
        }
        state.ssh_status(account)
    }

    fn uninstall_ssh_agent(&self, account: &AccountId) -> Result<SshAgentStatus> {
        let mut state = self.call(Operation::UninstallSshAgent)?;
        if state.ssh_agent.as_ref() == Some(account) {
            state.ssh_agent = None;
            // launchd stops the service's agent with the service
            if cfg!(target_os = "macos") {
                let account = state.account_mut(account)?;
                if account.state == LockState::Unlocked {
                    account.lock();
                    account.touch_id_ready = false;
                }
            }
        }
        state.ssh_status(account)
    }

    fn rbw_version(&self) -> Result<String> {
        drop(self.call(Operation::RbwVersion)?);
        Ok("demo".to_string())
    }

    fn command_line_tool(&self) -> Result<Option<CommandLineTool>> {
        Ok(Some(
            self.call(Operation::CommandLineTool)?.command_line.clone(),
        ))
    }

    fn set_command_line_tool(&self, on: bool) -> Result<CommandLineTool> {
        let mut state = self.call(Operation::SetCommandLineTool)?;
        let tool = &mut state.command_line;
        match (tool.state, on) {
            (ToolState::Taken, true) => {
                return Err(Error::Other(format!(
                    "{} is another program",
                    tool.path.display()
                )));
            }
            (ToolState::Taken, false) => {}
            (_, on) => tool.state = if on { ToolState::On } else { ToolState::Off },
        }
        Ok(tool.clone())
    }

    fn other_rbw(&self) -> Result<Option<OtherRbw>> {
        Ok(self.call(Operation::OtherRbw)?.other_rbw.clone())
    }
}

impl State {
    fn ssh_status(&mut self, id: &AccountId) -> Result<SshAgentStatus> {
        let installed = self.ssh_agent.clone();
        let (supported, elsewhere) = (self.ssh_supported, self.ssh_elsewhere.clone());
        let account = self.account_mut(id)?;
        let keys = (account.state == LockState::Unlocked).then(|| {
            account
                .items
                .iter()
                .filter_map(|item| match &item.data.content {
                    ItemContent::SshKey(key) => Some(SshKeySummary {
                        item: item.id.clone(),
                        name: item.data.name.clone(),
                        key_type: ssh::key_type(&key.public_key),
                        fingerprint: key.fingerprint.clone(),
                        public_key: key.public_key.clone(),
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
            _ if !supported => (
                SshService::Unsupported,
                false,
                SshUses::Other(PathBuf::from("/run/user/1000/keyring/ssh")),
            ),
            Some(owner) if owner == *id => (
                SshService::Installed,
                true,
                elsewhere.unwrap_or(SshUses::Rbw),
            ),
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
    use crate::{CharacterClasses, Characters, FieldValue, ItemKind, Login};

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

        let registering = fake
            .add_account(&Server::BitwardenCom, "sam+apikey@example.com")
            .unwrap();
        assert_eq!(
            fake.sign_in(&registering, &password, None),
            Err(Error::RegistrationRequired)
        );
        assert_eq!(
            fake.register_device(&registering, FakeBackend::API_KEY_ID, &"nope".into()),
            Err(Error::WrongApiKey)
        );
        fake.register_device(
            &registering,
            FakeBackend::API_KEY_ID,
            &FakeBackend::API_KEY_SECRET.into(),
        )
        .unwrap();
        assert!(matches!(
            fake.sign_in(&registering, &password, None),
            Err(Error::TwoStepRequired { .. })
        ));
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
        // The public key, from the private one.
        assert_eq!(key.subtitle.as_deref(), Some("Ed25519 key"));
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
    fn recent_items_come_newest_first() {
        let fake = FakeBackend::new();
        let id = personal();
        let recent = fake.recent_items(&id).unwrap();
        assert!(recent.len() >= 2);
        fake.item_used(&id, &recent[1]).unwrap();
        let after = fake.recent_items(&id).unwrap();
        assert_eq!(after[0], recent[1]);
        assert_eq!(after[1], recent[0]);
        assert_eq!(after.len(), recent.len());
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
        let phrase = fake.generate(&GeneratorOptions::passphrase(5)).unwrap();
        assert_eq!(phrase.expose().split(' ').count(), 5);
        let options = GeneratorOptions::Passphrase {
            words: 4,
            separator: Some('-'),
            capitalize: true,
            number: true,
        };
        let phrase = fake.generate(&options).unwrap();
        let words: Vec<_> = phrase.expose().split('-').collect();
        assert_eq!(words.len(), 4);
        assert!(
            words
                .iter()
                .all(|word| word.starts_with(char::is_uppercase))
        );
        assert_eq!(
            phrase.expose().chars().filter(char::is_ascii_digit).count(),
            1
        );
    }

    #[test]
    fn generated_passwords_follow_their_classes() {
        let fake = FakeBackend::new();
        let classes = |digits, symbols, each_class| {
            Characters::Classes(CharacterClasses {
                uppercase: true,
                lowercase: true,
                digits,
                symbols,
                each_class,
            })
        };
        let generate =
            |length, characters| fake.generate(&GeneratorOptions::Password { length, characters });
        for _ in 0..50 {
            let letters = generate(30, classes(false, false, false)).unwrap();
            assert!(letters.expose().chars().all(|c| c.is_ascii_alphabetic()));
            let no_digits = generate(30, classes(false, true, false)).unwrap();
            assert!(!no_digits.expose().chars().any(|c| c.is_ascii_digit()));
            let each = generate(4, classes(true, true, true)).unwrap();
            let each = each.expose();
            for holds in [
                char::is_ascii_uppercase,
                char::is_ascii_lowercase,
                char::is_ascii_digit,
                char::is_ascii_punctuation,
            ] {
                assert!(each.chars().any(|c| holds(&c)), "{each}");
            }
        }
        // as rbw refuses them
        assert!(generate(3, classes(true, true, true)).is_err());
        assert!(generate(0, Characters::All).is_err());
    }

    #[test]
    fn made_up_icons() {
        let fake = unlocked();
        let id = personal();
        let logins: Vec<_> = fake
            .items(&id)
            .unwrap()
            .into_iter()
            .filter(|item| item.kind == ItemKind::Login)
            .collect();
        // Nothing is kept before an icon is fetched, or kept.
        let with_website = logins
            .iter()
            .find(|item| !item.websites.is_empty())
            .unwrap();
        assert_eq!(fake.cached_website_icon(&id, with_website).unwrap(), None);
        let icons: Vec<_> = logins
            .iter()
            .map(|item| fake.website_icon(&id, item).unwrap())
            .collect();
        for (item, icon) in logins.iter().zip(&icons) {
            assert_eq!(
                fake.cached_website_icon(&id, item).unwrap(),
                Some(icon.clone())
            );
        }
        let fresh = unlocked();
        fresh.keep_icons(&id);
        assert_eq!(
            fresh.cached_website_icon(&id, with_website).unwrap(),
            Some(fake.website_icon(&id, with_website).unwrap())
        );
        // The made-up websites have one; the real ones show their letter.
        assert!((8..logins.len() / 2).contains(&icons.iter().flatten().count()));
        for icon in icons.into_iter().flatten() {
            assert_eq!(WebsiteIcon::from_server(&icon.bytes), Some(icon));
        }
        fake.set_preferences(&Preferences {
            website_icons: false,
            ..Preferences::default()
        })
        .unwrap();
        assert!(
            logins
                .iter()
                .all(|item| fake.website_icon(&id, item).unwrap().is_none())
        );
        assert_eq!(
            fake.cached_website_icon(&id, with_website).unwrap(),
            Some(None)
        );
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
        fake.set_latency(Duration::ZERO);

        fake.fail(Operation::InstallSshAgent, Error::SshAgentUnsupported);
        assert_eq!(fake.install_ssh_agent(&id), Err(Error::SshAgentUnsupported));
        assert_eq!(fake.install_ssh_agent(&id), Err(Error::SshAgentUnsupported));
        assert!(fake.items(&id).is_ok());
        fake.succeed(Operation::InstallSshAgent);
        assert!(fake.install_ssh_agent(&id).is_ok());

        fake.set_latency_of(Operation::Lock, Duration::from_millis(20));
        let start = Instant::now();
        fake.items(&id).unwrap();
        assert!(start.elapsed() < Duration::from_millis(20));
        fake.lock(&id).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(20));

        fake.sign_out_now(&id);
        let account = fake.account(&id).unwrap();
        assert_eq!(account.state, LockState::SignedOut);
        assert!(account.email.is_some());
        assert_eq!(fake.items(&id), Err(Error::NotSignedIn));
        fake.unlock(&id, &FakeBackend::PASSWORD.into()).unwrap_err();
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
        // as rbw does, installing leaves the account unlocked, and so does
        // uninstalling but on macOS
        fake.install_ssh_agent(&personal).unwrap();
        assert_eq!(fake.account(&personal).unwrap().state, LockState::Unlocked);
        fake.uninstall_ssh_agent(&personal).unwrap();
        let after = if cfg!(target_os = "macos") {
            LockState::Locked
        } else {
            LockState::Unlocked
        };
        assert_eq!(fake.account(&personal).unwrap().state, after);
        let status = fake.install_ssh_agent(&work).unwrap();
        assert_eq!(status.service, SshService::Installed);
        assert_eq!(status.keys, None);

        let elsewhere = SshUses::Other(PathBuf::from("/tmp/other"));
        fake.set_ssh_uses(Some(elsewhere.clone()));
        assert_eq!(fake.ssh_agent(&work).unwrap().ssh_uses, elsewhere);
        fake.set_ssh_supported(false);
        assert_eq!(
            fake.ssh_agent(&work).unwrap().service,
            SshService::Unsupported
        );
        assert_eq!(
            fake.install_ssh_agent(&work),
            Err(Error::SshAgentUnsupported)
        );
    }
}
