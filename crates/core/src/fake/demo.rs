//! The demo vaults.

use crate::{
    Card, CustomField, FieldValue, Folder, FolderId, Identity, Item, ItemContent, ItemData, ItemId,
    Login, Secret, Server, SshKey, UriMatch, Website,
};

pub const PERSONAL: &str = "personal";
pub const WORK: &str = "work";

pub struct DemoAccount {
    pub profile: &'static str,
    pub email: &'static str,
    pub server: Server,
    pub folders: Vec<Folder>,
    pub items: Vec<Item>,
}

pub fn accounts() -> Vec<DemoAccount> {
    vec![personal(), work()]
}

pub fn ssh_public_key(private_key: &str) -> (String, String) {
    let mut hash: u64 = 0x8422_2325_cbf2_9ce4;
    for byte in private_key.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    (
        format!("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI{hash:016x}Qm9vbXN0aWNr"),
        format!("SHA256:{hash:016x}c3Nob2tleWZpbmdlcnByaW50"),
    )
}

pub const WORDS: &[&str] = &[
    "acorn", "anchor", "apron", "arcade", "atlas", "badge", "bamboo", "banjo", "barley", "basil",
    "beacon", "bison", "blanket", "bonfire", "breeze", "bucket", "cactus", "canyon", "carbon",
    "cedar", "chalk", "cherry", "cinema", "clover", "cobalt", "comet", "copper", "coral", "cotton",
    "crayon", "dahlia", "denim", "desert", "dolphin", "dragon", "drizzle", "eagle", "ember",
    "falcon", "fennel", "fiddle", "flannel", "fossil", "galaxy", "garnet", "geyser", "ginger",
    "glacier", "gravel", "harbor", "hazel", "helmet", "hollow", "iris", "island", "ivory",
    "jasmine", "jigsaw", "juniper", "kettle", "kiwi", "lagoon", "lantern", "lemon", "lilac",
    "linen", "lobster", "magnet", "mango", "marble", "meadow", "meteor", "mitten", "mosaic",
    "nectar", "nickel", "nutmeg", "oasis", "olive", "orbit", "orchid", "otter", "paddle", "pebble",
    "pepper", "pillow", "pine", "plaza", "pocket", "pollen", "puzzle", "quartz", "quill", "radish",
    "raven", "ribbon", "river", "rocket", "saddle", "saffron", "salmon", "sandal", "satin",
    "shovel", "silver", "sketch", "sparrow", "spruce", "summit", "tangle", "thistle", "timber",
    "topaz", "tulip", "tundra", "velvet", "violet", "walnut", "willow", "window", "yarrow",
    "zephyr", "zinnia",
];

struct Vault {
    prefix: &'static str,
    folders: Vec<Folder>,
    items: Vec<Item>,
}

impl Vault {
    fn new(prefix: &'static str, folders: &[&str]) -> Self {
        let folders = folders
            .iter()
            .enumerate()
            .map(|(i, name)| Folder {
                id: FolderId(format!("{prefix}f00000-0000-4000-8000-{i:012}")),
                name: (*name).to_string(),
            })
            .collect();
        Self {
            prefix,
            folders,
            items: vec![],
        }
    }

    fn folder(&self, name: &str) -> Option<FolderId> {
        self.folders
            .iter()
            .find(|folder| folder.name == name)
            .map(|folder| folder.id.clone())
    }

    fn add(&mut self, name: &str, folder: Option<&str>, content: ItemContent) -> &mut Item {
        let id = ItemId(format!(
            "{}000000-0000-4000-8000-{:012}",
            self.prefix,
            self.items.len()
        ));
        let folder = folder.and_then(|name| self.folder(name));
        self.items.push(Item {
            id,
            data: ItemData {
                name: name.to_string(),
                folder,
                notes: Secret::default(),
                fields: vec![],
                content,
            },
            organization: None,
            favorite: false,
            editable: true,
            reprompt: false,
        });
        self.items.last_mut().expect("just pushed")
    }

    fn login(
        &mut self,
        name: &str,
        folder: Option<&str>,
        username: &str,
        website: &str,
    ) -> &mut Item {
        let password = password_for(name);
        self.add(
            name,
            folder,
            ItemContent::Login(Login {
                username: username.to_string(),
                password: Secret::new(password),
                totp: Secret::default(),
                websites: vec![Website {
                    uri: website.to_string(),
                    matching: None,
                }],
            }),
        )
    }
}

trait ItemExt {
    fn favorite(&mut self) -> &mut Self;
    fn code(&mut self) -> &mut Self;
    fn notes(&mut self, notes: &str) -> &mut Self;
    fn field(&mut self, name: &str, value: FieldValue) -> &mut Self;
    fn shared(&mut self, organization: &str, editable: bool) -> &mut Self;
    fn reprompt(&mut self) -> &mut Self;
}

impl ItemExt for Item {
    fn favorite(&mut self) -> &mut Self {
        self.favorite = true;
        self
    }

    fn code(&mut self) -> &mut Self {
        if let ItemContent::Login(login) = &mut self.data.content {
            login.totp = Secret::new(format!("JBSWY3DPEHPK3PXP{}", self.data.name.len()));
        }
        self
    }

    fn notes(&mut self, notes: &str) -> &mut Self {
        self.data.notes = Secret::new(notes);
        self
    }

    fn field(&mut self, name: &str, value: FieldValue) -> &mut Self {
        self.data.fields.push(CustomField {
            name: name.to_string(),
            value,
        });
        self
    }

    fn shared(&mut self, organization: &str, editable: bool) -> &mut Self {
        self.organization = Some(organization.to_string());
        self.editable = editable;
        self
    }

    fn reprompt(&mut self) -> &mut Self {
        self.reprompt = true;
        self
    }
}

// A stable password that looks generated.
fn password_for(name: &str) -> String {
    const CHARS: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!#%&*+-=?@^_";
    let mut hash = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    (0..20)
        .map(|_| {
            hash = (hash ^ (hash >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            char::from(CHARS[(hash % CHARS.len() as u64) as usize])
        })
        .collect()
}

fn ssh_key(comment: &str) -> ItemContent {
    let private_key = format!(
        "-----BEGIN OPENSSH PRIVATE KEY-----\n\
         b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\n\
         QyNTUxOQAAACD{comment}AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n\
         -----END OPENSSH PRIVATE KEY-----\n"
    );
    let (public_key, fingerprint) = ssh_public_key(&private_key);
    ItemContent::SshKey(SshKey {
        private_key: Secret::new(private_key),
        public_key: format!("{public_key} {comment}"),
        fingerprint,
    })
}

fn personal() -> DemoAccount {
    let mut vault = Vault::new(
        "a",
        &[
            "Personal",
            "Finance",
            "Shopping",
            "Travel",
            "Developer",
            "Home",
        ],
    );
    let me = "alex.martin@example.com";

    vault
        .login(
            "GitHub",
            Some("Developer"),
            "alexmartin",
            "https://github.com/login",
        )
        .favorite()
        .code();
    vault
        .login(
            "GitLab",
            Some("Developer"),
            "alexm",
            "https://gitlab.com/users/sign_in",
        )
        .code();
    vault.login(
        "npm",
        Some("Developer"),
        "alexmartin",
        "https://www.npmjs.com/login",
    );
    vault.login(
        "crates.io",
        Some("Developer"),
        "alexmartin",
        "https://crates.io",
    );
    vault
        .login(
            "DigitalOcean",
            Some("Developer"),
            me,
            "https://cloud.digitalocean.com/login",
        )
        .code();
    vault.login(
        "Docker Hub",
        Some("Developer"),
        "alexmartin",
        "https://hub.docker.com",
    );
    vault
        .login(
            "Google",
            Some("Personal"),
            "alex.martin@gmail.example",
            "https://accounts.google.com",
        )
        .favorite()
        .code();
    vault.login(
        "Apple Account",
        Some("Personal"),
        me,
        "https://account.apple.com",
    );
    vault
        .login(
            "Proton Mail",
            Some("Personal"),
            "alexmartin@proton.example",
            "https://account.proton.me/login",
        )
        .code();
    vault.login(
        "Mastodon",
        Some("Personal"),
        "@alex@social.example",
        "https://social.example/auth/sign_in",
    );
    vault.login(
        "Reddit",
        Some("Personal"),
        "alex_m",
        "https://www.reddit.com/login",
    );
    vault
        .login(
            "Spotify",
            Some("Personal"),
            me,
            "https://accounts.spotify.com/login",
        )
        .favorite();
    vault.login("Netflix", Some("Home"), me, "https://www.netflix.com/login");
    vault
        .login("Home Wi-Fi", Some("Home"), "", "http://192.168.1.1")
        .field("Network", FieldValue::Text("Martin-5G".into()))
        .notes("Router admin is on the sticker under the box.");
    vault.login(
        "Electricity",
        Some("Home"),
        "AM-2209871",
        "https://www.energy.example/account",
    );
    vault
        .login(
            "Bank",
            Some("Finance"),
            "40213377",
            "https://online.bank.example",
        )
        .reprompt()
        .favorite()
        .field("Security answer", FieldValue::Hidden("blue heron".into()))
        .code();
    vault.login(
        "PayPal",
        Some("Finance"),
        me,
        "https://www.paypal.com/signin",
    );
    vault
        .login(
            "Broker",
            Some("Finance"),
            "alexmartin",
            "https://broker.example/login",
        )
        .code();
    vault.login(
        "Tax office",
        Some("Finance"),
        "1 84 03 75 123 456",
        "https://tax.example.gov",
    );
    vault.login(
        "Amazon",
        Some("Shopping"),
        me,
        "https://www.amazon.com/ap/signin",
    );
    vault.login(
        "IKEA",
        Some("Shopping"),
        me,
        "https://www.ikea.com/profile/login",
    );
    vault.login("Bookshop", Some("Shopping"), me, "https://bookshop.example");
    vault
        .login(
            "Airline",
            Some("Travel"),
            "FF 4471 0923",
            "https://www.airline.example/login",
        )
        .field("Frequent flyer", FieldValue::Text("Gold".into()));
    vault.login(
        "Booking",
        Some("Travel"),
        me,
        "https://account.booking.com/sign-in",
    );
    vault.login(
        "Rail pass",
        Some("Travel"),
        me,
        "https://rail.example/account",
    );
    vault
        .login("Old forum", None, "alex1987", "forum.example.net")
        .field("Remember me", FieldValue::Boolean(true));

    let visa = vault.add(
        "Visa",
        Some("Finance"),
        ItemContent::Card(Card {
            cardholder_name: "Alex Martin".into(),
            brand: "Visa".into(),
            number: Secret::new("4111111111111111"),
            exp_month: "09".into(),
            exp_year: "2029".into(),
            code: Secret::new("123"),
        }),
    );
    visa.favorite().reprompt();
    vault.add(
        "Mastercard",
        Some("Finance"),
        ItemContent::Card(Card {
            cardholder_name: "Alex Martin".into(),
            brand: "Mastercard".into(),
            number: Secret::new("5555555555554444"),
            exp_month: "02".into(),
            exp_year: "2027".into(),
            code: Secret::new("321"),
        }),
    );
    vault.add(
        "Alex Martin",
        Some("Personal"),
        ItemContent::Identity(Identity {
            first_name: "Alex".into(),
            last_name: "Martin".into(),
            address1: "12 Rue des Lilas".into(),
            city: "Lyon".into(),
            postal_code: "69003".into(),
            country: "France".into(),
            phone: "+33 6 12 34 56 78".into(),
            email: me.into(),
            passport_number: Secret::new("19AB12345"),
            ..Identity::default()
        }),
    );
    vault.add(
        "Passport",
        Some("Travel"),
        ItemContent::Identity(Identity {
            title: "Mx".into(),
            first_name: "Alex".into(),
            middle_name: "Jordan".into(),
            last_name: "Martin".into(),
            country: "France".into(),
            passport_number: Secret::new("19AB12345"),
            license_number: Secret::new("12AA34567"),
            ..Identity::default()
        }),
    );
    vault
        .add("Recovery codes", Some("Personal"), ItemContent::SecureNote)
        .notes("GitHub\n  4f2a-91cd\n  77b0-3e1f\n\nGoogle\n  8812 4410\n  0937 2251");
    vault
        .add("Alarm code", Some("Home"), ItemContent::SecureNote)
        .notes("2580, then the key with the house on it.");
    vault
        .add("Wedding speech", None, ItemContent::SecureNote)
        .notes("Start with the story about the canoe.");
    vault
        .add(
            "Software licences",
            Some("Developer"),
            ItemContent::SecureNote,
        )
        .notes("Editor: AM-7781-PRO\nFonts: studio licence, 2 seats");
    vault
        .add("Laptop", Some("Developer"), ssh_key("alex@laptop"))
        .favorite();
    vault.add("Home server", Some("Developer"), ssh_key("alex@nas"));
    vault.add("Deploy key", Some("Developer"), ssh_key("deploy@ci"));
    vault
        .login(
            "Family Netflix",
            None,
            "family@example.com",
            "https://www.netflix.com",
        )
        .shared("Martin family", true);
    vault
        .login("Family router", None, "admin", "http://192.168.1.1")
        .shared("Martin family", false)
        .notes("Shared read-only: ask Sam to change it.");

    DemoAccount {
        profile: PERSONAL,
        email: me,
        server: Server::BitwardenCom,
        folders: vault.folders,
        items: vault.items,
    }
}

fn work() -> DemoAccount {
    let mut vault = Vault::new("b", &["Infrastructure", "Accounts"]);
    let me = "alex@studio.example";
    vault
        .login(
            "Studio mail",
            Some("Accounts"),
            me,
            "https://mail.studio.example",
        )
        .code();
    vault.login("Figma", Some("Accounts"), me, "https://www.figma.com/login");
    vault.login("Slack", Some("Accounts"), me, "https://studio.slack.com");
    vault
        .login(
            "Grafana",
            Some("Infrastructure"),
            "admin",
            "https://grafana.studio.example",
        )
        .shared("Studio", false);
    vault
        .login(
            "AWS console",
            Some("Infrastructure"),
            "alex",
            "https://studio.signin.aws.amazon.com/console",
        )
        .code()
        .shared("Studio", true)
        .field("Account ID", FieldValue::Text("4417-0921-3381".into()));
    vault.add(
        "Build server",
        Some("Infrastructure"),
        ssh_key("alex@studio"),
    );
    vault
        .add("VPN setup", None, ItemContent::SecureNote)
        .notes("Server: vpn.studio.example\nProfile: studio-full");
    if let Some(Item {
        data: ItemData {
            content: ItemContent::Login(login),
            ..
        },
        ..
    }) = vault.items.first_mut()
    {
        login.websites.push(Website {
            uri: "https://webmail.studio.example".into(),
            matching: Some(UriMatch::Host),
        });
    }
    DemoAccount {
        profile: WORK,
        email: me,
        server: Server::SelfHosted("https://vault.studio.example".into()),
        folders: vault.folders,
        items: vault.items,
    }
}
