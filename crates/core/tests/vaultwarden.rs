//! The real backend against a Vaultwarden server, through rbw-ng. Run it
//! with `crates/core/tests/vaultwarden.sh`, which starts the server, makes
//! the accounts and sets the variables below; plain `cargo test` skips it.

use std::time::{Duration, SystemTime};

use rbw_desktop_core::{
    Backend, Card, Characters, CustomField, Error, FieldValue, FolderId, GeneratorOptions,
    Identity, ItemContent, ItemData, ItemKind, LockState, Login, LoginCode, Preferences,
    RbwBackend, Secret, Server, SshKey, TwoStepMethod, UriMatch, Website,
};

// an unencrypted ed25519 key, made for this test
const SSH_PRIVATE_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACC4DOVa+kn8HZu7XPoSipFq+oUfoZbqOgcTIHmhNyxV9gAAAJAVDmEWFQ5h
FgAAAAtzc2gtZWQyNTUxOQAAACC4DOVa+kn8HZu7XPoSipFq+oUfoZbqOgcTIHmhNyxV9g
AAAEDPhuqJUm+QNatlbazPhiaEyM0VmpB6TVIsEBBU6Xijx7gM5Vr6Sfwdm7tc+hKKkWr6
hR+hluo6BxMgeaE3LFX2AAAACHRlc3RAcmJ3AQIDBAU=
-----END OPENSSH PRIVATE KEY-----
";
const SSH_FINGERPRINT: &str = "SHA256:82tRvuz0hLHtR5dkHl65L2UtySKaBT2BHA254wi5bkc";

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set; run tests/vaultwarden.sh"))
}

fn login(name: &str, password: &str) -> ItemData {
    let mut data = ItemData::new(ItemKind::Login);
    data.name = name.to_string();
    data.notes = "recovery codes".into();
    data.fields = vec![
        CustomField {
            name: "PIN".into(),
            value: FieldValue::Hidden("1234".into()),
        },
        CustomField {
            name: "Remember".into(),
            value: FieldValue::Boolean(true),
        },
    ];
    data.content = ItemContent::Login(Login {
        username: "octo".into(),
        password: password.into(),
        totp: "JBSWY3DPEHPK3PXP".into(),
        websites: vec![Website {
            uri: "https://github.com".into(),
            matching: Some(UriMatch::Host),
        }],
    });
    data
}

#[test]
#[ignore = "needs a Vaultwarden server; run tests/vaultwarden.sh"]
fn against_vaultwarden() {
    let server = Server::SelfHosted(env("RBW_DESKTOP_TEST_SERVER"));
    let email = env("RBW_DESKTOP_TEST_EMAIL");
    let password = Secret::new(env("RBW_DESKTOP_TEST_PASSWORD"));
    let backend = RbwBackend::new().expect("rbw-ng with the client API");
    assert!(backend.accounts().unwrap().is_empty());

    // signing in
    let id = backend.add_account(&server, &email).unwrap();
    assert!(
        id.is_default(),
        "the first account takes the default profile"
    );
    let account = backend.account(&id).unwrap();
    assert_eq!(account.state, LockState::SignedOut);
    assert_eq!(account.server, server);
    assert_eq!(
        backend.sign_in(&id, &"wrong".into(), None),
        Err(Error::WrongPassword)
    );
    backend.sign_in(&id, &password, None).unwrap();
    let account = backend.account(&id).unwrap();
    assert_eq!(account.state, LockState::Unlocked);
    assert!(account.last_sync.is_some());
    assert_eq!(
        backend.add_account(&server, &email.to_uppercase()),
        Err(Error::AccountExists(id.clone()))
    );
    assert_eq!(backend.accounts().unwrap(), [account]);

    // items
    let folder = backend.create_folder(&id, "Developer").unwrap();
    assert!(backend.folders(&id).unwrap().contains(&folder));
    // a name that looks like an option is a name
    let dashes = backend.create_folder(&id, "--help").unwrap();
    assert_eq!(dashes.name, "--help");
    let mut github = login("GitHub", "first-password");
    github.folder = Some(folder.id.clone());
    let created = backend.create_item(&id, &github).unwrap();
    assert_eq!(created.subtitle.as_deref(), Some("octo"));
    assert!(created.has_code && created.editable);
    assert_eq!(created.folder, Some(folder.id.clone()));
    assert_eq!(backend.item(&id, &created.id).unwrap().data, github);

    let code = backend.verification_code(&id, &created.id).unwrap();
    assert_eq!(code.code.expose().len(), 6);
    assert_eq!(code.period, Duration::from_secs(30));
    assert!(code.expires_at > SystemTime::now());

    github.content = match github.content {
        ItemContent::Login(login) => ItemContent::Login(Login {
            password: "second-password".into(),
            ..login
        }),
        other => other,
    };
    backend.edit_item(&id, &created.id, &github).unwrap();
    backend.set_favorite(&id, &created.id, true).unwrap();
    let edited = backend.item(&id, &created.id).unwrap();
    assert_eq!(edited.data, github);
    assert!(edited.favorite);
    backend.move_item(&id, &created.id, None).unwrap();
    let moved = backend.item(&id, &created.id).unwrap();
    assert_eq!(moved.data.folder, None);
    assert!(moved.favorite);
    backend
        .move_item(&id, &created.id, Some(&folder.id))
        .unwrap();
    assert_eq!(
        backend.move_item(&id, &created.id, Some(&FolderId("0".repeat(36)))),
        Err(Error::NotFound)
    );

    let mut card = ItemData::new(ItemKind::Card);
    card.name = "Visa".into();
    card.content = ItemContent::Card(Card {
        cardholder_name: "Alex Martin".into(),
        brand: "Visa".into(),
        number: "4111111111111111".into(),
        exp_month: "09".into(),
        exp_year: "2029".into(),
        code: "123".into(),
    });
    let saved = backend.create_item(&id, &card).unwrap();
    assert_eq!(saved.subtitle.as_deref(), Some("Visa"));
    assert_eq!(backend.item(&id, &saved.id).unwrap().data, card);

    let mut identity = ItemData::new(ItemKind::Identity);
    identity.name = "Me".into();
    identity.content = ItemContent::Identity(Identity {
        first_name: "Alex".into(),
        last_name: "Martin".into(),
        passport_number: "19AB12345".into(),
        ..Identity::default()
    });
    let saved = backend.create_item(&id, &identity).unwrap();
    assert_eq!(saved.subtitle.as_deref(), Some("Alex Martin"));
    assert_eq!(backend.item(&id, &saved.id).unwrap().data, identity);

    let mut key = ItemData::new(ItemKind::SshKey);
    key.name = "Laptop".into();
    key.content = ItemContent::SshKey(SshKey {
        private_key: SSH_PRIVATE_KEY.into(),
        ..SshKey::default()
    });
    let saved_key = backend.create_item(&id, &key).unwrap();
    assert_eq!(saved_key.subtitle.as_deref(), Some(SSH_FINGERPRINT));
    let ItemContent::SshKey(read) = backend.item(&id, &saved_key.id).unwrap().data.content else {
        panic!("not an SSH key");
    };
    assert!(read.public_key.starts_with("ssh-ed25519 "));

    let mut note = ItemData::new(ItemKind::SecureNote);
    note.name = "Old".into();
    let old = backend.create_item(&id, &note).unwrap();
    backend.delete_item(&id, &old.id).unwrap();
    assert_eq!(backend.item(&id, &old.id), Err(Error::NotFound));
    // and the two organization items that vaultwarden.sh made
    let items = backend.items(&id).unwrap();
    assert_eq!(items.len(), 6);

    // an item that asks for the master password again
    let bank = items.iter().find(|item| item.name == "Bank").unwrap();
    assert!(bank.reprompt && bank.has_code && bank.organization.is_some());
    assert_eq!(backend.item(&id, &bank.id), Err(Error::RepromptRequired));
    assert_eq!(
        backend.verification_code(&id, &bank.id),
        Err(Error::RepromptRequired)
    );
    assert_eq!(
        backend.unlock_item(&id, &bank.id, &"wrong".into()),
        Err(Error::WrongPassword)
    );
    backend.unlock_item(&id, &bank.id, &password).unwrap();
    let unlocked = backend.item(&id, &bank.id).unwrap();
    assert!(unlocked.reprompt);
    let ItemContent::Login(bank_login) = &unlocked.data.content else {
        panic!("not a login");
    };
    assert_eq!(bank_login.password.expose(), "bank-password");
    backend.verification_code(&id, &bank.id).unwrap();
    backend.set_favorite(&id, &bank.id, true).unwrap();
    assert!(
        backend
            .items(&id)
            .unwrap()
            .iter()
            .any(|item| item.id == bank.id && item.favorite)
    );

    let ssh = backend.ssh_agent(&id).unwrap();
    let keys = ssh.keys.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].fingerprint, SSH_FINGERPRINT);

    // the generator
    let generated = backend
        .generate(&GeneratorOptions::Password {
            length: 24,
            characters: Characters::DigitsOnly,
        })
        .unwrap();
    assert_eq!(generated.expose().len(), 24);
    assert!(generated.expose().bytes().all(|b| b.is_ascii_digit()));
    let phrase = backend
        .generate(&GeneratorOptions::Passphrase { words: 4 })
        .unwrap();
    assert_eq!(phrase.expose().split(' ').count(), 4);

    // settings, which don't lock
    let mut settings = backend.account_settings(&id).unwrap();
    settings.lock_timeout = Duration::from_secs(600);
    settings.ssh_confirm = true;
    backend.set_account_settings(&id, &settings).unwrap();
    assert_eq!(backend.account_settings(&id).unwrap(), settings);
    assert_eq!(backend.account(&id).unwrap().state, LockState::Unlocked);
    // icons, which the server finds or not depending on its access to the
    // internet, and keeps on disk either way
    let github = backend
        .items(&id)
        .unwrap()
        .into_iter()
        .find(|item| item.id == created.id)
        .unwrap();
    let icon = backend.website_icon(&id, &github).unwrap();
    let cache = std::path::PathBuf::from(env("XDG_CACHE_HOME")).join("rbwdesktop/icons/default");
    let kept = if icon.is_some() {
        "github.com.icon"
    } else {
        "github.com.miss"
    };
    assert!(cache.join(kept).exists());
    assert_eq!(backend.website_icon(&id, &github).unwrap(), icon);
    assert_eq!(backend.website_icon(&id, &saved_key).unwrap(), None);

    let preferences = Preferences {
        clipboard_clear: None,
        website_icons: false,
    };
    backend.set_preferences(&preferences).unwrap();
    assert_eq!(backend.preferences().unwrap(), preferences);
    assert!(!cache.exists());
    assert_eq!(backend.website_icon(&id, &github).unwrap(), None);
    assert!(!cache.exists());

    // locking
    let synced = backend.account(&id).unwrap().last_sync;
    std::thread::sleep(Duration::from_secs(1));
    backend.sync(&id).unwrap();
    assert!(backend.account(&id).unwrap().last_sync > synced);
    backend.lock(&id).unwrap();
    assert_eq!(backend.account(&id).unwrap().state, LockState::Locked);
    assert_eq!(backend.items(&id), Err(Error::Locked));
    assert_eq!(backend.ssh_agent(&id).unwrap().keys, None);
    assert_eq!(
        backend.unlock_with_touch_id(&id),
        Err(Error::TouchIdUnavailable)
    );
    assert_eq!(
        backend.unlock(&id, &"wrong".into()),
        Err(Error::WrongPassword)
    );
    backend.unlock(&id, &password).unwrap();
    assert_eq!(backend.items(&id).unwrap().len(), 6);
    assert_eq!(backend.item(&id, &bank.id), Err(Error::RepromptRequired));

    // an account with two-step login
    let two_step_email = env("RBW_DESKTOP_TEST_TWO_STEP_EMAIL");
    let other = backend.add_account(&server, &two_step_email).unwrap();
    assert!(!other.is_default());
    let Err(Error::TwoStepRequired { methods }) = backend.sign_in(&other, &password, None) else {
        panic!("no two-step login");
    };
    assert_eq!(methods, [TwoStepMethod::Authenticator]);
    let wrong = LoginCode::TwoStep {
        method: TwoStepMethod::Authenticator,
        code: "000000".into(),
    };
    assert_eq!(
        backend.sign_in(&other, &password, Some(&wrong)),
        Err(Error::WrongCode)
    );
    // the first account's vault computes the codes: a login with the
    // authenticator's secret. The server refuses a code twice, and setting
    // up the authenticator used the current one, so wait for the next.
    let mut authenticator = ItemData::new(ItemKind::Login);
    authenticator.name = "Authenticator".into();
    authenticator.content = ItemContent::Login(Login {
        totp: env("RBW_DESKTOP_TEST_TOTP_SECRET").into(),
        ..Login::default()
    });
    let authenticator = backend.create_item(&id, &authenticator).unwrap();
    let current = backend.verification_code(&id, &authenticator.id).unwrap();
    if let Ok(left) = current.expires_at.duration_since(SystemTime::now()) {
        std::thread::sleep(left + Duration::from_secs(1));
    }
    let code = backend.verification_code(&id, &authenticator.id).unwrap();
    assert_ne!(code.code, current.code);
    let right = LoginCode::TwoStep {
        method: TwoStepMethod::Authenticator,
        code: code.code.expose().to_string(),
    };
    backend.sign_in(&other, &password, Some(&right)).unwrap();
    let signed_in = backend.account(&other).unwrap();
    assert_eq!(signed_in.state, LockState::Unlocked);
    assert!(backend.items(&other).unwrap().is_empty());

    // an account that registers the device with its API key
    let api_key_email = env("RBW_DESKTOP_TEST_API_KEY_EMAIL");
    let registering = backend.add_account(&server, &api_key_email).unwrap();
    let client_id = env("RBW_DESKTOP_TEST_CLIENT_ID");
    assert_eq!(
        backend.register_device(&registering, &client_id, &"wrong".into()),
        Err(Error::WrongApiKey)
    );
    backend
        .register_device(
            &registering,
            &client_id,
            &env("RBW_DESKTOP_TEST_CLIENT_SECRET").into(),
        )
        .unwrap();
    backend.sign_in(&registering, &password, None).unwrap();
    assert_eq!(
        backend.account(&registering).unwrap().state,
        LockState::Unlocked
    );

    assert_eq!(backend.accounts().unwrap().len(), 3);
    for account in [registering, other, id] {
        backend.remove_account(&account).unwrap();
    }
    assert!(backend.accounts().unwrap().is_empty());
}
