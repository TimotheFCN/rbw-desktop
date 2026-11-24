//! An item as the editor's fields hold it, as text, and back. Saving writes
//! back only the fields that changed, so an item saved without a change
//! comes back exactly as it was, down to a space at the end of a password.

use std::collections::BTreeMap;

use rbw_desktop_core::{CustomField, ItemContent, ItemData, Secret, Website};

/// A field the editor shows as one line of text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    Name,
    Username,
    Password,
    /// The secret a login's verification codes come from.
    Authenticator,
    Cardholder,
    Brand,
    Number,
    Expiry,
    SecurityCode,
    Title,
    FirstName,
    MiddleName,
    LastName,
    Email,
    Phone,
    IdentityUsername,
    Address1,
    Address2,
    Address3,
    City,
    State,
    PostalCode,
    Country,
    Ssn,
    Passport,
    License,
}

/// The editor's fields, as text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Texts {
    pub values: BTreeMap<Key, String>,
    pub notes: String,
    pub websites: Vec<WebsiteText>,
    pub fields: Vec<FieldText>,
}

/// A website's row: the website it shows, by its place among the item's,
/// and its text. A row added in the editor comes from none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebsiteText {
    pub origin: Option<usize>,
    pub uri: String,
}

/// A custom field's row, and whether the item had it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldText {
    pub origin: Option<usize>,
    pub field: CustomField,
}

impl Texts {
    pub fn get(&self, key: Key) -> &str {
        self.values.get(&key).map_or("", String::as_str)
    }
}

/// What the editor shows of an item.
pub fn texts(data: &ItemData) -> Texts {
    let mut values = BTreeMap::from([(Key::Name, data.name.clone())]);
    let mut put = |key, value: &str| {
        values.insert(key, value.to_string());
    };
    let mut websites = vec![];
    match &data.content {
        ItemContent::Login(login) => {
            put(Key::Username, &login.username);
            put(Key::Password, login.password.expose());
            put(Key::Authenticator, login.totp.expose());
            websites = login
                .websites
                .iter()
                .enumerate()
                .map(|(index, website)| WebsiteText {
                    origin: Some(index),
                    uri: website.uri.clone(),
                })
                .collect();
        }
        ItemContent::Card(card) => {
            put(Key::Cardholder, &card.cardholder_name);
            put(Key::Brand, &card.brand);
            put(Key::Number, card.number.expose());
            put(Key::Expiry, &expiry_text(&card.exp_month, &card.exp_year));
            put(Key::SecurityCode, card.code.expose());
        }
        ItemContent::Identity(identity) => {
            for (key, value) in [
                (Key::Title, &identity.title),
                (Key::FirstName, &identity.first_name),
                (Key::MiddleName, &identity.middle_name),
                (Key::LastName, &identity.last_name),
                (Key::Email, &identity.email),
                (Key::Phone, &identity.phone),
                (Key::IdentityUsername, &identity.username),
                (Key::Address1, &identity.address1),
                (Key::Address2, &identity.address2),
                (Key::Address3, &identity.address3),
                (Key::City, &identity.city),
                (Key::State, &identity.state),
                (Key::PostalCode, &identity.postal_code),
                (Key::Country, &identity.country),
            ] {
                put(key, value);
            }
            put(Key::Ssn, identity.ssn.expose());
            put(Key::Passport, identity.passport_number.expose());
            put(Key::License, identity.license_number.expose());
        }
        ItemContent::SecureNote | ItemContent::SshKey(_) => {}
    }
    Texts {
        values,
        notes: data.notes.expose().to_string(),
        websites,
        fields: data
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| FieldText {
                origin: Some(index),
                field: field.clone(),
            })
            .collect(),
    }
}

/// The item to save: `original` with what changed from `initial`, what the
/// editor first showed, to `current`, what it shows now.
pub fn apply(original: &ItemData, initial: &Texts, current: &Texts) -> ItemData {
    let mut data = original.clone();
    for (&key, value) in &current.values {
        if initial.values.get(&key) != Some(value) {
            set(&mut data, key, value);
        }
    }
    if current.notes != initial.notes {
        data.notes = Secret::new(current.notes.clone());
    }
    if current.websites != initial.websites
        && let (ItemContent::Login(before), ItemContent::Login(after)) =
            (&original.content, &mut data.content)
    {
        after.websites = websites(&before.websites, &current.websites);
    }
    if current.fields != initial.fields {
        data.fields = current
            .fields
            .iter()
            // A field added and left empty is no field.
            .filter(|row| row.origin.is_some() || !is_blank(&row.field))
            .map(|row| row.field.clone())
            .collect();
    }
    data
}

fn is_blank(field: &CustomField) -> bool {
    use rbw_desktop_core::FieldValue;
    field.name.trim().is_empty()
        && match &field.value {
            FieldValue::Text(text) => text.is_empty(),
            FieldValue::Hidden(secret) => secret.is_empty(),
            FieldValue::Boolean(on) => !on,
            FieldValue::Linked(_) => false,
        }
}

/// The websites the rows show. A row left as it was keeps its website as
/// it was, how browsers match it included; one emptied is removed.
fn websites(before: &[Website], rows: &[WebsiteText]) -> Vec<Website> {
    rows.iter()
        .filter_map(|row| {
            let was = row.origin.and_then(|index| before.get(index));
            match was {
                Some(website) if website.uri == row.uri => Some(website.clone()),
                _ if row.uri.trim().is_empty() => None,
                _ => Some(Website {
                    uri: row.uri.trim().to_string(),
                    matching: was.and_then(|website| website.matching),
                }),
            }
        })
        .collect()
}

/// Writes one changed field into the item. Secrets, notes and names are
/// kept exactly as typed; the item's name, addresses and the card's
/// expiry lose the spaces around them.
fn set(data: &mut ItemData, key: Key, value: &str) {
    let text = value.to_string();
    let secret = || Secret::new(value);
    match (key, &mut data.content) {
        (Key::Name, _) => data.name = value.trim().to_string(),
        (Key::Username, ItemContent::Login(login)) => login.username = text,
        (Key::Password, ItemContent::Login(login)) => login.password = secret(),
        (Key::Authenticator, ItemContent::Login(login)) => login.totp = secret(),
        (Key::Cardholder, ItemContent::Card(card)) => card.cardholder_name = text,
        (Key::Brand, ItemContent::Card(card)) => card.brand = value.trim().to_string(),
        (Key::Number, ItemContent::Card(card)) => card.number = secret(),
        (Key::Expiry, ItemContent::Card(card)) => {
            (card.exp_month, card.exp_year) = parse_expiry(value);
        }
        (Key::SecurityCode, ItemContent::Card(card)) => card.code = secret(),
        (Key::Title, ItemContent::Identity(identity)) => identity.title = text,
        (Key::FirstName, ItemContent::Identity(identity)) => identity.first_name = text,
        (Key::MiddleName, ItemContent::Identity(identity)) => identity.middle_name = text,
        (Key::LastName, ItemContent::Identity(identity)) => identity.last_name = text,
        (Key::Email, ItemContent::Identity(identity)) => identity.email = value.trim().into(),
        (Key::Phone, ItemContent::Identity(identity)) => identity.phone = text,
        (Key::IdentityUsername, ItemContent::Identity(identity)) => identity.username = text,
        (Key::Address1, ItemContent::Identity(identity)) => identity.address1 = text,
        (Key::Address2, ItemContent::Identity(identity)) => identity.address2 = text,
        (Key::Address3, ItemContent::Identity(identity)) => identity.address3 = text,
        (Key::City, ItemContent::Identity(identity)) => identity.city = text,
        (Key::State, ItemContent::Identity(identity)) => identity.state = text,
        (Key::PostalCode, ItemContent::Identity(identity)) => identity.postal_code = text,
        (Key::Country, ItemContent::Identity(identity)) => identity.country = text,
        (Key::Ssn, ItemContent::Identity(identity)) => identity.ssn = secret(),
        (Key::Passport, ItemContent::Identity(identity)) => identity.passport_number = secret(),
        (Key::License, ItemContent::Identity(identity)) => identity.license_number = secret(),
        // A field of another kind, left from before the kind was changed.
        _ => {}
    }
}

/// A card's expiry as the editor shows it: "09/2029".
pub fn expiry_text(month: &str, year: &str) -> String {
    match (month, year) {
        ("", "") => String::new(),
        ("", year) => format!("/{year}"),
        (month, year) => format!("{month:0>2}/{year}"),
    }
}

/// The month and year of an expiry as typed, such as "9/29" or "09 / 2029".
/// A two-digit year is this century's.
fn parse_expiry(text: &str) -> (String, String) {
    let (month, year) = text.split_once('/').unwrap_or((text, ""));
    let (month, year) = (month.trim(), year.trim());
    let year = if year.len() == 2 && year.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("20{year}")
    } else {
        year.to_string()
    };
    (month.to_string(), year)
}

#[cfg(test)]
mod tests {
    use rbw_desktop_core::{Card, FieldValue, Identity, ItemKind, Login, SshKey, UriMatch};

    use super::*;

    fn login() -> ItemData {
        let mut data = ItemData::new(ItemKind::Login);
        data.name = " Bank ".into();
        data.notes = Secret::new("first line\nsecond line\n");
        data.fields = vec![
            CustomField {
                name: "PIN".into(),
                value: FieldValue::Hidden(" 1234 ".into()),
            },
            CustomField {
                name: "Remember me".into(),
                value: FieldValue::Boolean(true),
            },
            CustomField {
                name: "Linked".into(),
                value: FieldValue::Linked(101),
            },
            CustomField {
                name: String::new(),
                value: FieldValue::Text(String::new()),
            },
        ];
        data.content = ItemContent::Login(Login {
            username: " alex ".into(),
            password: Secret::new(" pass phrase "),
            totp: Secret::new("JBSW Y3DP"),
            websites: vec![
                Website {
                    uri: "https://bank.example/login ".into(),
                    matching: Some(UriMatch::Exact),
                },
                Website {
                    uri: "bank.example".into(),
                    matching: Some(UriMatch::Host),
                },
                Website {
                    uri: String::new(),
                    matching: None,
                },
            ],
        });
        data
    }

    fn card() -> ItemData {
        let mut data = ItemData::new(ItemKind::Card);
        data.name = "Visa".into();
        data.content = ItemContent::Card(Card {
            cardholder_name: "Alex Martin".into(),
            brand: "Visa ".into(),
            number: Secret::new("4111 1111 1111 1111 "),
            exp_month: "9".into(),
            exp_year: "29".into(),
            code: Secret::new("123"),
        });
        data
    }

    fn identity() -> ItemData {
        let mut data = ItemData::new(ItemKind::Identity);
        data.name = "Me".into();
        data.content = ItemContent::Identity(Identity {
            title: "Mx".into(),
            first_name: "Alex".into(),
            middle_name: "J ".into(),
            last_name: "Martin".into(),
            address1: "12 Rue des Lilas".into(),
            address2: "Bâtiment B".into(),
            address3: "3e étage".into(),
            city: "Lyon".into(),
            state: "Rhône".into(),
            postal_code: "69003".into(),
            country: "France".into(),
            phone: "+33 6 12 34 56 78".into(),
            email: " alex@example.com".into(),
            username: "alexm".into(),
            ssn: Secret::new("1 84 03"),
            license_number: Secret::new("12AA34567"),
            passport_number: Secret::new("19AB12345"),
        });
        data
    }

    fn ssh_key() -> ItemData {
        let mut data = ItemData::new(ItemKind::SshKey);
        data.name = "Laptop".into();
        data.notes = Secret::new("  ");
        data.content = ItemContent::SshKey(SshKey {
            private_key: Secret::new("-----BEGIN OPENSSH PRIVATE KEY-----\n"),
            public_key: "ssh-ed25519 AAAA".into(),
            fingerprint: "SHA256:abc".into(),
        });
        data
    }

    fn note() -> ItemData {
        let mut data = ItemData::new(ItemKind::SecureNote);
        data.name = "Codes".into();
        data.notes = Secret::new("\n  indented\n\n");
        data
    }

    /// What the editor makes when only `change` is changed.
    fn edited(data: &ItemData, change: impl FnOnce(&mut Texts)) -> ItemData {
        let initial = texts(data);
        let mut current = initial.clone();
        change(&mut current);
        apply(data, &initial, &current)
    }

    #[test]
    fn an_item_saved_unchanged_is_the_same_item() {
        for data in [login(), card(), identity(), ssh_key(), note()] {
            assert_eq!(edited(&data, |_| {}), data);
        }
    }

    #[test]
    fn a_change_leaves_every_other_field_as_it_was() {
        let data = login();
        let renamed = edited(&data, |texts| {
            texts.values.insert(Key::Name, "  Bank  ".into());
        });
        assert_eq!(renamed.name, "Bank");
        assert_eq!(
            ItemData {
                name: data.name.clone(),
                ..renamed
            },
            data
        );
    }

    #[test]
    fn secrets_and_notes_keep_their_spaces() {
        let saved = edited(&login(), |texts| {
            texts.values.insert(Key::Password, " new pass ".into());
            texts.notes = "a\nb\n".into();
        });
        let ItemContent::Login(login) = &saved.content else {
            panic!("a login");
        };
        assert_eq!(login.password.expose(), " new pass ");
        assert_eq!(saved.notes.expose(), "a\nb\n");
    }

    #[test]
    fn websites_are_added_changed_and_removed() {
        let saved = edited(&login(), |texts| {
            // The first removed, the second changed, the empty third kept,
            // one added and one added empty.
            texts.websites.remove(0);
            texts.websites[0].uri = " bank.example/app ".into();
            texts.websites.push(WebsiteText {
                origin: None,
                uri: "m.bank.example".into(),
            });
            texts.websites.push(WebsiteText {
                origin: None,
                uri: " ".into(),
            });
        });
        let ItemContent::Login(login) = &saved.content else {
            panic!("a login");
        };
        assert_eq!(
            login.websites,
            vec![
                Website {
                    uri: "bank.example/app".into(),
                    matching: Some(UriMatch::Host),
                },
                Website {
                    uri: String::new(),
                    matching: None,
                },
                Website {
                    uri: "m.bank.example".into(),
                    matching: None,
                },
            ]
        );
    }

    #[test]
    fn custom_fields_are_added_changed_and_removed() {
        let saved = edited(&login(), |texts| {
            texts.fields.remove(0);
            texts.fields[0].field.value = FieldValue::Boolean(false);
            texts.fields.push(FieldText {
                origin: None,
                field: CustomField {
                    name: "Member".into(),
                    value: FieldValue::Text("4471".into()),
                },
            });
            texts.fields.push(FieldText {
                origin: None,
                field: CustomField {
                    name: " ".into(),
                    value: FieldValue::Hidden(Secret::default()),
                },
            });
        });
        let names: Vec<_> = saved
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect();
        assert_eq!(names, ["Remember me", "Linked", "", "Member"]);
        assert_eq!(saved.fields[0].value, FieldValue::Boolean(false));
        assert_eq!(saved.fields[1].value, FieldValue::Linked(101));
    }

    #[test]
    fn an_expiry_reads_as_typed() {
        let expiry = |text: &str| {
            let saved = edited(&card(), |texts| {
                texts.values.insert(Key::Expiry, text.into());
            });
            let ItemContent::Card(card) = saved.content else {
                panic!("a card");
            };
            (card.exp_month, card.exp_year)
        };
        assert_eq!(expiry("9/29"), ("9".into(), "2029".into()));
        assert_eq!(expiry(" 11 / 2031 "), ("11".into(), "2031".into()));
        assert_eq!(expiry(""), (String::new(), String::new()));
        assert_eq!(texts(&card()).get(Key::Expiry), "09/29");
    }

    #[test]
    fn every_identity_field_is_edited() {
        let saved = edited(&identity(), |texts| {
            for value in texts.values.values_mut() {
                value.push('!');
            }
        });
        let ItemContent::Identity(identity) = saved.content else {
            panic!("an identity");
        };
        assert_eq!(saved.name, "Me!");
        assert_eq!(identity.address3, "3e étage!");
        assert_eq!(identity.email, "alex@example.com!");
        assert_eq!(identity.passport_number.expose(), "19AB12345!");
        assert_eq!(identity.license_number.expose(), "12AA34567!");
    }
}
