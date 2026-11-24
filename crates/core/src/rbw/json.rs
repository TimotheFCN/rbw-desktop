//! Items, folders and statuses in the JSON `rbw` reads and writes:
//! Bitwarden's item format, with secrets kept in [`Secret`]s.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{
    Card, CustomField, FieldValue, Folder, FolderId, Identity, Item, ItemContent, ItemData, ItemId,
    ItemKind, ItemSummary, Login, Secret, Server, SshKey, SshKeySummary, UriMatch,
    VerificationCode, Website, ssh,
};

const LOGIN: u8 = 1;
const SECURE_NOTE: u8 = 2;
const CARD: u8 = 3;
const IDENTITY: u8 = 4;
const SSH_KEY: u8 = 5;

const EU_BASE_URL: &str = "https://api.bitwarden.eu";

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ItemJson {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    ty: u8,
    name: Option<String>,
    #[serde(default)]
    notes: Option<Secret>,
    #[serde(default)]
    folder_id: Option<String>,
    #[serde(default)]
    pub organization_id: Option<String>,
    // left out of an edit, it stays as it is
    #[serde(default, skip_serializing_if = "Option::is_none")]
    favorite: Option<bool>,
    #[serde(default = "yes", skip_serializing)]
    edit: bool,
    #[serde(default, skip_serializing)]
    reprompt: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    login: Option<LoginJson>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    card: Option<CardJson>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity: Option<IdentityJson>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ssh_key: Option<SshKeyJson>,
    #[serde(default)]
    fields: Vec<FieldJson>,
}

fn yes() -> bool {
    true
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct LoginJson {
    username: Option<String>,
    #[serde(default)]
    password: Option<Secret>,
    #[serde(default)]
    totp: Option<Secret>,
    #[serde(default)]
    uris: Vec<UriJson>,
    // rbw's: whether the login has a code, in a list without secrets
    #[serde(default, skip_serializing)]
    has_totp: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct UriJson {
    uri: Option<String>,
    #[serde(rename = "match")]
    matching: Option<u8>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CardJson {
    #[serde(default)]
    cardholder_name: Option<String>,
    brand: Option<String>,
    #[serde(default)]
    number: Option<Secret>,
    #[serde(default)]
    exp_month: Option<String>,
    #[serde(default)]
    exp_year: Option<String>,
    #[serde(default)]
    code: Option<Secret>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct IdentityJson {
    title: Option<String>,
    first_name: Option<String>,
    middle_name: Option<String>,
    last_name: Option<String>,
    address1: Option<String>,
    address2: Option<String>,
    address3: Option<String>,
    city: Option<String>,
    state: Option<String>,
    postal_code: Option<String>,
    country: Option<String>,
    phone: Option<String>,
    email: Option<String>,
    username: Option<String>,
    ssn: Option<Secret>,
    license_number: Option<Secret>,
    passport_number: Option<Secret>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct SshKeyJson {
    private_key: Option<Secret>,
    public_key: Option<String>,
    key_fingerprint: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FieldJson {
    name: Option<String>,
    value: Option<Secret>,
    #[serde(rename = "type")]
    ty: u8,
    #[serde(default)]
    linked_id: Option<u16>,
}

const TEXT: u8 = 0;
const HIDDEN: u8 = 1;
const BOOLEAN: u8 = 2;
const LINKED: u8 = 3;

fn text(value: Option<String>) -> String {
    value.unwrap_or_default()
}

fn secret(value: Option<Secret>) -> Secret {
    value.unwrap_or_default()
}

// Bitwarden stores an empty value as null.
fn some(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn some_secret(value: &Secret) -> Option<Secret> {
    (!value.is_empty()).then(|| value.clone())
}

fn uri_match(id: u8) -> Option<UriMatch> {
    Some(match id {
        0 => UriMatch::Domain,
        1 => UriMatch::Host,
        2 => UriMatch::StartsWith,
        3 => UriMatch::Exact,
        4 => UriMatch::RegularExpression,
        5 => UriMatch::Never,
        _ => return None,
    })
}

fn uri_match_id(matching: UriMatch) -> u8 {
    match matching {
        UriMatch::Domain => 0,
        UriMatch::Host => 1,
        UriMatch::StartsWith => 2,
        UriMatch::Exact => 3,
        UriMatch::RegularExpression => 4,
        UriMatch::Never => 5,
    }
}

fn kind(ty: u8) -> Option<ItemKind> {
    Some(match ty {
        LOGIN => ItemKind::Login,
        SECURE_NOTE => ItemKind::SecureNote,
        CARD => ItemKind::Card,
        IDENTITY => ItemKind::Identity,
        SSH_KEY => ItemKind::SshKey,
        _ => return None,
    })
}

impl ItemJson {
    /// The item as the list shows it; `None` for kinds this version
    /// doesn't know.
    pub fn summary(self) -> Option<ItemSummary> {
        let kind = kind(self.ty)?;
        let (subtitle, websites, has_code) = match kind {
            ItemKind::Login => {
                let login = self.login.unwrap_or_default();
                (
                    login.username,
                    login.uris.into_iter().filter_map(|uri| uri.uri).collect(),
                    login.has_totp,
                )
            }
            ItemKind::Card => (self.card.and_then(|card| card.brand), vec![], false),
            ItemKind::Identity => {
                let identity = self.identity.unwrap_or_default();
                let name = Identity {
                    first_name: text(identity.first_name),
                    middle_name: text(identity.middle_name),
                    last_name: text(identity.last_name),
                    ..Identity::default()
                }
                .full_name();
                (Some(name), vec![], false)
            }
            ItemKind::SshKey => (
                self.ssh_key.and_then(|key| key.key_fingerprint),
                vec![],
                false,
            ),
            ItemKind::SecureNote => (None, vec![], false),
        };
        Some(ItemSummary {
            id: ItemId(self.id.unwrap_or_default()),
            kind,
            name: text(self.name),
            subtitle: subtitle.filter(|subtitle| !subtitle.is_empty()),
            websites,
            folder: self.folder_id.map(FolderId),
            organization: self.organization_id,
            favorite: self.favorite.unwrap_or(false),
            has_code,
            editable: self.edit,
            reprompt: self.reprompt != 0,
        })
    }

    /// The SSH key this item holds, for the keys the agent serves.
    pub fn ssh_key(&self) -> Option<SshKeySummary> {
        let key = self.ssh_key.as_ref().filter(|_| self.ty == SSH_KEY)?;
        Some(SshKeySummary {
            item: ItemId(self.id.clone().unwrap_or_default()),
            name: self.name.clone().unwrap_or_default(),
            key_type: ssh::key_type(key.public_key.as_deref().unwrap_or_default()),
            fingerprint: key.key_fingerprint.clone().unwrap_or_default(),
        })
    }

    pub fn item(self) -> Option<Item> {
        let content = match kind(self.ty)? {
            ItemKind::Login => {
                let login = self.login.unwrap_or_default();
                ItemContent::Login(Login {
                    username: text(login.username),
                    password: secret(login.password),
                    totp: secret(login.totp),
                    websites: login
                        .uris
                        .into_iter()
                        .filter_map(|uri| {
                            Some(Website {
                                uri: uri.uri?,
                                matching: uri.matching.and_then(uri_match),
                            })
                        })
                        .collect(),
                })
            }
            ItemKind::SecureNote => ItemContent::SecureNote,
            ItemKind::Card => {
                let card = self.card.unwrap_or_default();
                ItemContent::Card(Card {
                    cardholder_name: text(card.cardholder_name),
                    brand: text(card.brand),
                    number: secret(card.number),
                    exp_month: text(card.exp_month),
                    exp_year: text(card.exp_year),
                    code: secret(card.code),
                })
            }
            ItemKind::Identity => {
                let identity = self.identity.unwrap_or_default();
                ItemContent::Identity(Identity {
                    title: text(identity.title),
                    first_name: text(identity.first_name),
                    middle_name: text(identity.middle_name),
                    last_name: text(identity.last_name),
                    address1: text(identity.address1),
                    address2: text(identity.address2),
                    address3: text(identity.address3),
                    city: text(identity.city),
                    state: text(identity.state),
                    postal_code: text(identity.postal_code),
                    country: text(identity.country),
                    phone: text(identity.phone),
                    email: text(identity.email),
                    username: text(identity.username),
                    ssn: secret(identity.ssn),
                    license_number: secret(identity.license_number),
                    passport_number: secret(identity.passport_number),
                })
            }
            ItemKind::SshKey => {
                let key = self.ssh_key.unwrap_or_default();
                ItemContent::SshKey(SshKey {
                    private_key: secret(key.private_key),
                    public_key: text(key.public_key),
                    fingerprint: text(key.key_fingerprint),
                })
            }
        };
        let fields = self
            .fields
            .into_iter()
            .map(|field| CustomField {
                name: text(field.name),
                value: match field.ty {
                    HIDDEN => FieldValue::Hidden(secret(field.value)),
                    BOOLEAN => FieldValue::Boolean(
                        field.value.is_some_and(|value| value.expose() == "true"),
                    ),
                    LINKED => FieldValue::Linked(field.linked_id.unwrap_or_default()),
                    _ => FieldValue::Text(
                        field
                            .value
                            .map(|value| value.expose().to_string())
                            .unwrap_or_default(),
                    ),
                },
            })
            .collect();
        Some(Item {
            id: ItemId(self.id.unwrap_or_default()),
            data: ItemData {
                name: text(self.name),
                folder: self.folder_id.map(FolderId),
                notes: secret(self.notes),
                fields,
                content,
            },
            organization: self.organization_id,
            favorite: self.favorite.unwrap_or(false),
            editable: self.edit,
            reprompt: self.reprompt != 0,
        })
    }

    /// The item to send to `rbw item create` or `rbw item edit`.
    pub fn from_data(data: &ItemData) -> Self {
        let mut item = Self {
            ty: match data.content.kind() {
                ItemKind::Login => LOGIN,
                ItemKind::SecureNote => SECURE_NOTE,
                ItemKind::Card => CARD,
                ItemKind::Identity => IDENTITY,
                ItemKind::SshKey => SSH_KEY,
            },
            name: Some(data.name.clone()),
            notes: some_secret(&data.notes),
            folder_id: data.folder.as_ref().map(|folder| folder.0.clone()),
            edit: true,
            fields: data.fields.iter().map(field_json).collect(),
            ..Self::default()
        };
        match &data.content {
            ItemContent::Login(login) => {
                item.login = Some(LoginJson {
                    username: some(&login.username),
                    password: some_secret(&login.password),
                    totp: some_secret(&login.totp),
                    uris: login
                        .websites
                        .iter()
                        .map(|website| UriJson {
                            uri: Some(website.uri.clone()),
                            matching: website.matching.map(uri_match_id),
                        })
                        .collect(),
                    has_totp: false,
                });
            }
            ItemContent::SecureNote => {}
            ItemContent::Card(card) => {
                item.card = Some(CardJson {
                    cardholder_name: some(&card.cardholder_name),
                    brand: some(&card.brand),
                    number: some_secret(&card.number),
                    exp_month: some(&card.exp_month),
                    exp_year: some(&card.exp_year),
                    code: some_secret(&card.code),
                });
            }
            ItemContent::Identity(identity) => {
                item.identity = Some(IdentityJson {
                    title: some(&identity.title),
                    first_name: some(&identity.first_name),
                    middle_name: some(&identity.middle_name),
                    last_name: some(&identity.last_name),
                    address1: some(&identity.address1),
                    address2: some(&identity.address2),
                    address3: some(&identity.address3),
                    city: some(&identity.city),
                    state: some(&identity.state),
                    postal_code: some(&identity.postal_code),
                    country: some(&identity.country),
                    phone: some(&identity.phone),
                    email: some(&identity.email),
                    username: some(&identity.username),
                    ssn: some_secret(&identity.ssn),
                    license_number: some_secret(&identity.license_number),
                    passport_number: some_secret(&identity.passport_number),
                });
            }
            ItemContent::SshKey(key) => {
                item.ssh_key = Some(SshKeyJson {
                    private_key: some_secret(&key.private_key),
                    public_key: None,
                    key_fingerprint: None,
                });
            }
        }
        item
    }
}

fn field_json(field: &CustomField) -> FieldJson {
    let (ty, value, linked_id) = match &field.value {
        FieldValue::Text(text) => (TEXT, some(text).map(Secret::new), None),
        FieldValue::Hidden(value) => (HIDDEN, some_secret(value), None),
        FieldValue::Boolean(value) => (BOOLEAN, Some(Secret::new(value.to_string())), None),
        FieldValue::Linked(id) => (LINKED, None, Some(*id)),
    };
    FieldJson {
        name: some(&field.name),
        value,
        ty,
        linked_id,
    }
}

/// JSON for `rbw`'s stdin, written into a buffer of the right size from the
/// start, so that no copy of a secret is left behind by its growing.
pub fn to_stdin(value: &impl serde::Serialize) -> crate::Result<zeroize::Zeroizing<Vec<u8>>> {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let invalid = |error: serde_json::Error| crate::Error::Other(error.to_string());
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value).map_err(invalid)?;
    let mut buffer = zeroize::Zeroizing::new(Vec::with_capacity(count.0));
    serde_json::to_writer(&mut *buffer, value).map_err(invalid)?;
    Ok(buffer)
}

#[derive(serde::Deserialize)]
pub struct FolderJson {
    id: String,
    name: String,
}

impl From<FolderJson> for Folder {
    fn from(folder: FolderJson) -> Self {
        Self {
            id: FolderId(folder.id),
            name: folder.name,
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeJson {
    code: Secret,
    period: u64,
    expires_at: u64,
}

impl From<CodeJson> for VerificationCode {
    fn from(code: CodeJson) -> Self {
        Self {
            code: code.code,
            period: Duration::from_secs(code.period),
            expires_at: UNIX_EPOCH + Duration::from_secs(code.expires_at),
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusJson {
    pub status: String,
    pub server_url: Option<String>,
    pub user_email: Option<String>,
    #[serde(default)]
    pub last_sync: Option<String>,
    pub touch_id: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileJson {
    pub profile: Option<String>,
    pub user_email: Option<String>,
    pub server_url: Option<String>,
}

pub fn server(base_url: Option<&str>) -> Server {
    match base_url.map(|url| url.trim_end_matches('/')) {
        None | Some("") => Server::BitwardenCom,
        Some(EU_BASE_URL) => Server::BitwardenEu,
        Some(url) => Server::SelfHosted(url.to_string()),
    }
}

/// The `base_url` rbw's configuration needs for a server; `None` for
/// bitwarden.com, its default.
pub fn base_url(server: &Server) -> Option<&str> {
    match server {
        Server::BitwardenCom => None,
        Server::BitwardenEu => Some(EU_BASE_URL),
        Server::SelfHosted(url) => Some(url),
    }
}

/// A time as RFC 3339 writes it in UTC, such as `2026-11-24T09:41:07Z`,
/// with or without fractions of a second.
pub fn parse_time(text: &str) -> Option<SystemTime> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.splitn(3, '-').map(str::parse::<i64>);
    let (year, month, day) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let time = time.split('.').next()?;
    let mut time = time.splitn(3, ':').map(str::parse::<u64>);
    let (hour, minute, second) = (time.next()?.ok()?, time.next()?.ok()?, time.next()?.ok()?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    // Howard Hinnant's days_from_civil
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = u64::try_from(era * 146_097 + doe - 719_468).ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(days * 86_400 + hour * 3600 + minute * 60 + second))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_without_secrets() {
        let list: Vec<ItemJson> = serde_json::from_str(
            r#"[
            {"object":"item","id":"a","organizationId":null,"folderId":"f","type":1,"reprompt":0,
             "name":"GitHub","favorite":true,"edit":true,"revisionDate":"x","deletedDate":null,
             "login":{"username":"octo","uris":[{"match":null,"uri":"https://github.com"}],"hasTotp":true}},
            {"id":"b","organizationId":"o","type":3,"name":"Visa","favorite":false,"edit":false,
             "reprompt":1,"card":{"brand":"Visa"}},
            {"id":"c","type":4,"name":"Me","identity":{"title":null,"firstName":"Alex","middleName":null,"lastName":"Martin"}},
            {"id":"d","type":5,"name":"Laptop","sshKey":{"publicKey":"ssh-ed25519 AAAA","keyFingerprint":"SHA256:x"}},
            {"id":"e","type":9,"name":"From the future"}
            ]"#,
        )
        .unwrap();
        let summaries: Vec<_> = list.into_iter().filter_map(ItemJson::summary).collect();
        assert_eq!(summaries.len(), 4);
        let login = &summaries[0];
        assert_eq!(login.subtitle.as_deref(), Some("octo"));
        assert_eq!(login.websites, ["https://github.com"]);
        assert_eq!(login.folder, Some(FolderId("f".into())));
        assert!(login.has_code && login.favorite && login.editable);
        assert_eq!(summaries[1].subtitle.as_deref(), Some("Visa"));
        assert_eq!(summaries[1].organization.as_deref(), Some("o"));
        assert!(!summaries[1].editable && summaries[1].reprompt);
        assert!(!login.reprompt);
        assert_eq!(summaries[2].subtitle.as_deref(), Some("Alex Martin"));
        assert_eq!(summaries[3].subtitle.as_deref(), Some("SHA256:x"));
    }

    #[test]
    fn ssh_keys_with_their_type() {
        let list: Vec<ItemJson> = serde_json::from_str(
            r#"[
            {"id":"a","type":1,"name":"GitHub","login":{"username":"octo"}},
            {"id":"d","type":5,"name":"Laptop","sshKey":{"publicKey":"ssh-ed25519 AAAA","keyFingerprint":"SHA256:x"}}
            ]"#,
        )
        .unwrap();
        let keys: Vec<_> = list.iter().filter_map(ItemJson::ssh_key).collect();
        assert_eq!(
            keys,
            [SshKeySummary {
                item: ItemId("d".into()),
                name: "Laptop".into(),
                key_type: "Ed25519".into(),
                fingerprint: "SHA256:x".into(),
            }]
        );
    }

    #[test]
    fn items_go_there_and_back() {
        let mut data = ItemData::new(ItemKind::Login);
        data.name = "GitHub".into();
        data.folder = Some(FolderId("f".into()));
        data.notes = "codes".into();
        data.content = ItemContent::Login(Login {
            username: "octo".into(),
            password: "pw".into(),
            totp: Secret::default(),
            websites: vec![Website {
                uri: "https://github.com".into(),
                matching: Some(UriMatch::Host),
            }],
        });
        data.fields = vec![
            CustomField {
                name: "PIN".into(),
                value: FieldValue::Hidden("1234".into()),
            },
            CustomField {
                name: "Remember".into(),
                value: FieldValue::Boolean(true),
            },
            CustomField {
                name: "Note".into(),
                value: FieldValue::Text(String::new()),
            },
            CustomField {
                name: "Who".into(),
                value: FieldValue::Linked(100),
            },
        ];
        let json = to_stdin(&ItemJson::from_data(&data)).unwrap();
        let text = std::str::from_utf8(&json).unwrap();
        assert!(text.contains(r#""totp":null"#), "{text}");
        assert!(
            !text.contains("favorite") && !text.contains("\"id\""),
            "{text}"
        );
        let mut back: ItemJson = serde_json::from_slice(&json).unwrap();
        back.id = Some("a".into());
        let item = back.item().unwrap();
        assert_eq!(item.id, ItemId("a".into()));
        assert_eq!(item.data, data);

        for kind in [ItemKind::Card, ItemKind::Identity, ItemKind::SecureNote] {
            let mut data = ItemData::new(kind);
            data.name = "x".into();
            let json = to_stdin(&ItemJson::from_data(&data)).unwrap();
            let back: ItemJson = serde_json::from_slice(&json).unwrap();
            assert_eq!(back.item().unwrap().data, data);
        }
    }

    #[test]
    fn servers_and_their_base_urls() {
        for server in [
            Server::BitwardenCom,
            Server::BitwardenEu,
            Server::SelfHosted("https://vault.example.com".into()),
        ] {
            assert_eq!(super::server(base_url(&server)), server);
        }
        assert_eq!(
            super::server(Some("https://api.bitwarden.eu/")),
            Server::BitwardenEu
        );
    }

    #[test]
    fn times() {
        assert_eq!(
            parse_time("1970-01-02T00:00:01Z"),
            Some(UNIX_EPOCH + Duration::from_secs(86_401))
        );
        assert_eq!(
            parse_time("2026-11-24T09:41:07.123456Z"),
            Some(UNIX_EPOCH + Duration::from_secs(1_795_513_267))
        );
        assert_eq!(
            parse_time("2000-03-01T00:00:00Z"),
            Some(UNIX_EPOCH + Duration::from_secs(951_868_800))
        );
        assert_eq!(parse_time("2026-11-24 09:41:07"), None);
        assert_eq!(parse_time("2026-13-24T09:41:07Z"), None);
    }
}
