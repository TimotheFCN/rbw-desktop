use std::time::{Duration, SystemTime};

use crate::Secret;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FolderId(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub id: FolderId,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Login,
    SecureNote,
    Card,
    Identity,
    SshKey,
}

/// An item as the vault lists it: what the list and search need, without
/// secrets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemSummary {
    pub id: ItemId,
    pub kind: ItemKind,
    pub name: String,
    /// The line under the name: a login's username, a card's brand, an
    /// identity's name or an SSH key's fingerprint.
    pub subtitle: Option<String>,
    pub websites: Vec<String>,
    pub folder: Option<FolderId>,
    /// Set for items shared through an organization.
    pub organization: Option<String>,
    pub favorite: bool,
    /// Whether the item has a verification code.
    pub has_code: bool,
    /// Whether the item asks for the master password again before its
    /// secrets are shown; see [`Backend::unlock_item`](crate::Backend::unlock_item).
    pub reprompt: bool,
    /// Whether the item can be edited and deleted. Every item can be marked
    /// favourite and moved between folders.
    pub editable: bool,
}

/// One item with its secrets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: ItemId,
    pub data: ItemData,
    pub organization: Option<String>,
    pub favorite: bool,
    pub editable: bool,
    pub reprompt: bool,
}

impl Item {
    pub fn kind(&self) -> ItemKind {
        self.data.content.kind()
    }
}

/// The part of an item that creating and editing set. An empty string or
/// secret is a value that isn't set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemData {
    pub name: String,
    pub folder: Option<FolderId>,
    pub notes: Secret,
    pub fields: Vec<CustomField>,
    pub content: ItemContent,
    /// The version of the item on the server that this data was read from,
    /// as [`Backend::item`](crate::Backend::item) gives it. Edit a copy of
    /// [`Item::data`]: [`Backend::edit_item`](crate::Backend::edit_item)
    /// then fails with [`Error::Conflict`](crate::Error::Conflict) if the
    /// item changed on the server since it was read. Without one, the edit
    /// is checked against the last sync only.
    pub revision: Option<Revision>,
}

/// An item's version on the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision(pub(crate) String);

impl ItemData {
    /// An empty item of a kind, to fill in before creating it.
    pub fn new(kind: ItemKind) -> Self {
        let content = match kind {
            ItemKind::Login => ItemContent::Login(Login::default()),
            ItemKind::SecureNote => ItemContent::SecureNote,
            ItemKind::Card => ItemContent::Card(Card::default()),
            ItemKind::Identity => ItemContent::Identity(Identity::default()),
            ItemKind::SshKey => ItemContent::SshKey(SshKey::default()),
        };
        Self {
            name: String::new(),
            folder: None,
            notes: Secret::default(),
            fields: vec![],
            content,
            revision: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "an identity's fields make it large, and items are few"
)]
pub enum ItemContent {
    Login(Login),
    SecureNote,
    Card(Card),
    Identity(Identity),
    SshKey(SshKey),
}

impl ItemContent {
    pub fn kind(&self) -> ItemKind {
        match self {
            Self::Login(_) => ItemKind::Login,
            Self::SecureNote => ItemKind::SecureNote,
            Self::Card(_) => ItemKind::Card,
            Self::Identity(_) => ItemKind::Identity,
            Self::SshKey(_) => ItemKind::SshKey,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Login {
    pub username: String,
    pub password: Secret,
    /// The secret the verification codes come from, not a code; see
    /// [`Backend::verification_code`](crate::Backend::verification_code).
    pub totp: Secret,
    pub websites: Vec<Website>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Website {
    pub uri: String,
    /// How the browser extension matches pages with this website; `None`
    /// for the account's default.
    pub matching: Option<UriMatch>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UriMatch {
    Domain,
    Host,
    StartsWith,
    Exact,
    RegularExpression,
    Never,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Card {
    pub cardholder_name: String,
    pub brand: String,
    pub number: Secret,
    pub exp_month: String,
    pub exp_year: String,
    pub code: Secret,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    pub title: String,
    pub first_name: String,
    pub middle_name: String,
    pub last_name: String,
    pub address1: String,
    pub address2: String,
    pub address3: String,
    pub city: String,
    pub state: String,
    pub postal_code: String,
    pub country: String,
    pub phone: String,
    pub email: String,
    pub username: String,
    pub ssn: Secret,
    pub license_number: Secret,
    pub passport_number: Secret,
}

impl Identity {
    /// First, middle and last name, as one line.
    pub fn full_name(&self) -> String {
        [&self.first_name, &self.middle_name, &self.last_name]
            .into_iter()
            .filter(|part| !part.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// An SSH key. Creating one needs only the private key in OpenSSH format;
/// the public key and fingerprint come from it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshKey {
    pub private_key: Secret,
    pub public_key: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomField {
    pub name: String,
    pub value: FieldValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    Text(String),
    Hidden(Secret),
    Boolean(bool),
    /// Shows another field of the item, by Bitwarden's linked field id.
    Linked(u16),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationCode {
    pub code: Secret,
    /// How long each code lasts, usually 30 seconds.
    pub period: Duration,
    /// When this code stops being valid and the next one starts.
    pub expires_at: SystemTime,
}
