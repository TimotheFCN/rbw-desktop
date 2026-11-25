# rbw-desktop-core

The backend of rbw-desktop: everything the interface asks of rbw, behind the
`Backend` trait, with no GPUI dependency.

* `RbwBackend` drives rbw-ng's `rbw` command line.
* `FakeBackend` holds demo data in memory, for building the interface, tests,
  screenshots and a demo mode.

Every method blocks, some on the network, so the interface calls them off its
main thread. Errors are `Error` values, never text to parse.

## The interface

| Area | Methods |
| --- | --- |
| Accounts | `accounts`, `account`, `add_account`, `sign_in`, `send_email_code`, `register_device`, `remove_account` |
| Locking | `unlock`, `unlock_with_touch_id`, `lock` |
| Vault | `sync`, `items`, `item`, `unlock_item`, `create_item`, `edit_item`, `delete_item`, `set_favorite`, `move_item`, `folders`, `create_folder`, `verification_code`, `website_icon`, `recent_items`, `item_used` |
| Generator | `generate` |
| Settings | `account_settings`, `set_account_settings`, `preferences`, `set_preferences` |
| SSH agent | `ssh_agent`, `install_ssh_agent`, `uninstall_ssh_agent` |
| About | `rbw_version` |

An account is an rbw profile (`RBW_PROFILE`), shared with the command line.
Adding one creates a signed-out profile; `sign_in` then fails with
`TwoStepRequired` or `NewDeviceCodeRequired` until it gets the code.

It fails with `RegistrationRequired` when the server wants the device
registered with the account's personal API key first: Bitwarden servers
from before mid-2025 did so for an unknown device by asking for a captcha,
which rbw can't show. Ask for the key (the web vault shows it under Settings,
Security, Keys), pass it to `register_device` and call `sign_in` again. On
current servers, which email a new device code instead, registering first
also spares that code. rbw uses the key for that one request and keeps
nothing of it. The first
account takes the default profile, the next ones a profile named after the
email.

`items` lists summaries without secrets. Secrets come only from `item` and
`verification_code`, in `Secret` values that wipe their memory when dropped.
An item with `reprompt` set asks for the master password again, as
Bitwarden's "master password re-prompt" does: `item`, `verification_code` and
`edit_item` fail with `RepromptRequired` until `unlock_item` gets the master
password, which `rbw-agent` checks. The item then stays unlocked until it
goes unread for two minutes or the account locks; each read, such as
refreshing its code, keeps it unlocked. The official clients likewise ask
each time such an item is opened and not again while it stays open. The
list (`items`) never needs it: it shows only what the official clients show
of such an item, its name and subtitle, and everything else stays closed,
for every kind of item.

The re-prompt guards against someone at an unlocked computer, as in the
official clients; it isn't more encryption. `rbw-agent` keeps the values
closed whatever program asks, but a program that runs as the same user can
still read every other item while the account is unlocked, read the item
while it is unlocked, turn the re-prompt off on the server with the token
rbw keeps on disk, or ask the person for the master password itself.
rbw-ng's `docs/scripting.md` says more.

`ItemSummary::editable` and `Item::editable` say whether an item can be
edited and deleted: items in read-only collections can't. Every item can be
marked favourite and moved between folders, which change only what belongs to
the account and send no secrets. Item kinds this version doesn't know are
left out of the list.

To edit an item, change a copy of its `Item::data`: it carries the version
of the item it was read at (`ItemData::revision`), and `edit_item` fails with
`Conflict` when the item changed on the server since, for example on another
device, instead of overwriting that change.

`account` is cheap (a few milliseconds): poll it every second or two to notice
locking, unlocking, and syncs through `last_sync`. `Account::touch_id_ready`
says when `unlock_with_touch_id` can work: once Touch ID unlock is on and the
account was unlocked with its password since its agent started.

Account settings (lock timeout, Touch ID, SSH confirmation) are rbw's, per
profile; changing them doesn't lock. Preferences (clipboard clearing, website
icons, the generator's last options, the menu bar item) are the app's, in
`preferences.json` in its configuration folder:
`~/.config/rbwdesktop` on Linux and
`~/Library/Application Support/io.github.timothefcn.RbwDesktop` on macOS
(`APP_ID`).
On Linux, `rbw-desktop` would be the folder of an rbw profile called
`desktop`, which `rbw purge --all` removes. `session_dir` gives the app a
folder for what lasts as long as the login session, such as a socket.

`item_used` notes that an item was copied from or opened in the app, and
`recent_items` lists each account's last 20, newest first: their ids only,
in `recent.json` next to the preferences. Removing the account forgets them.

`GeneratorOptions::strength` says how hard what the options make is to
guess, from the options alone: rbw draws each character or word at random,
so the bits of randomness are known exactly (`entropy`), whatever the
result looks like. Under 45 bits is weak, 60 and over strong.
`Characters::Classes` chooses the classes of characters one by one, as
Bitwarden's generator does, and with `each_class` the password holds one of
each: rbw draws it uniformly from all the passwords that do, and `entropy`
counts exactly those. A passphrase can take another separator than a space,
or none, capitals, and a digit after one of its words, which adds the
randomness of the digit and of its place. Options that make nothing, such
as no class or fewer characters than classes to hold, fail
(`makes_anything` says so before).

`website_icon` gives a login's website icon, to show instead of its letter.
It picks the website as the official clients do (the first one, when it is a
web address, never a `.onion` or `.i2p` site) and asks the icon service of
the account's server through `rbw icon`, which sends the host name and
nothing about the account. The server may pass on what the website serves,
so only bitmaps (PNG, JPEG, GIF, WebP, BMP, ICO) of at most 1024 pixels a
side, by what their headers say, count as icons; anything else, SVG included,
counts as none. Icons are kept in the app's cache folder for 30
days, sites without one for 3 days, and the oldest go past 20 MB, so after
the first time an icon comes from the disk. Each account keeps its own, as
another server may have an icon that this one hasn't, and removing the
account forgets them. With the website icons
preference off it returns `None` without asking anything, and turning the
preference off forgets the icons. The fake backend makes up an icon for most
websites, as SVG.

`ssh_agent` lists the keys the agent serves with their type ("Ed25519",
"RSA", "ECDSA"), from the public key. Installing the agent's service leaves
the account as it is: the `rbw-agent` that runs keeps it unlocked, and
serves the SSH keys, until it stops, and the service takes over then.
Removing the service does too on Linux; on macOS it stops the service's
agent, which locks the account. On macOS, installing fails with
`AppNotInstalled` while the app runs from its disk image or from the
temporary copy macOS makes of an app opened where it was downloaded: the
service would run the bundled `rbw-agent` from a path that goes away.

The fake backend has knobs for the interface's states: `set_latency` and
`set_latency_of` one `Operation`, `fail_next` and `fail` one `Operation`
until `succeed`, `set_offline`, `set_touch_id`, `sign_out_now` and
`unlock_now` an account, `set_ssh_supported` and `set_ssh_uses` for the
SSH agent's states. A self-hosted address with
`.invalid` in it leads to no Bitwarden server.

## How it maps to rbw

Each call runs `rbw` once, with `RBW_PROFILE` set for the account,
`RBW_NONINTERACTIVE=1` so that it never prompts, and `RBW_ERROR_FORMAT=json`
so that its errors come with a code. The vault stays unlocked in the profile's
`rbw-agent` between calls. Passwords and items with secrets go to `rbw` on
stdin, never in arguments, which every user of the computer can see, and
nothing is written to disk or logged. Two-step codes and the API key go in the
environment of the one `rbw` process that needs them, which only the same user
can read. Values that come from people or the server, such as folder names,
item ids and host names, follow a `--`, so that one starting with `-` can't
pass for an option.

| Method | Command |
| --- | --- |
| `accounts` | `rbw profiles --json`, then `account` for each |
| `account` | `rbw status --json` |
| `add_account` | `rbw config set email` and `base_url` |
| `sign_in` | `rbw login --password-stdin [--method ID] [--code-env]`, with the code in `RBW_CODE` |
| `send_email_code` | `rbw login --password-stdin --method 1 --code ""` |
| `register_device` | `rbw register --api-key-env`, with the key in `RBW_CLIENT_ID` and `RBW_CLIENT_SECRET` |
| `remove_account` | `rbw purge --all` |
| `unlock`, `unlock_with_touch_id`, `lock` | `rbw unlock --password-stdin`, `rbw unlock --touch-id`, `rbw lock` |
| `sync` | `rbw sync` |
| `items`, `item`, `unlock_item` | `rbw item list`, `rbw item get ID`, `rbw item unlock ID --password-stdin` |
| `create_item`, `edit_item`, `delete_item` | `rbw item create`, `rbw item edit ID`, `rbw item delete ID` |
| `set_favorite`, `move_item` | `rbw item favorite ID true\|false`, `rbw item move ID [FOLDER]` |
| `folders`, `create_folder` | `rbw folder list`, `rbw folder create NAME` |
| `verification_code` | `rbw item code ID` |
| `website_icon` | `rbw icon HOST`, then the cache |
| `generate` | `rbw generate LENGTH`, with `--no-uppercase`, `--no-lowercase`, `--no-numbers`, `--no-symbols` and `--each-class`, or `--only-numbers`, `--nonconfusables`, or `--diceware` with `--separator`, `--capitalize` and `--include-number` |
| `recent_items`, `item_used` | none: `recent.json` |
| `account_settings`, `set_account_settings` | `rbw config show`, `rbw config set` |
| `ssh_agent` | `rbw ssh-agent status --json`, and the SSH keys of `rbw item list` |
| `install_ssh_agent`, `uninstall_ssh_agent` | `rbw ssh-agent install`, `rbw ssh-agent uninstall` |

`RbwBackend::new` needs rbw-ng 1.19.0 or later (`REQUIRED_RBW`), as
`rbw --version` says; another program called `rbw`, such as the original
rbw, doesn't count. It uses the `rbw` in `RBW_DESKTOP_RBW` when that is set,
and otherwise the one bundled with the app. The packages bundle `rbw` and
`rbw-agent` off `PATH`, from the folder above the app's executable: in
`libexec/rbw-desktop` on Linux (`/usr/libexec/rbw-desktop` beside
`/usr/bin/rbw-desktop`) and in the bundle's `Contents/Helpers` on macOS. Only
a build without them, such as one run from source, uses the `rbw` on
`PATH`. `rbw` starts the `rbw-agent` beside it. Local calls time out after
30 seconds, calls that reach the server after 120.

The app runs its own `rbw` whatever else is installed because of the agent:
each profile has one `rbw-agent`, and `rbw` restarts an agent of another
version, which locks the vault. An app and a command line of different
versions would each lock the account whenever the other used it.

With 300 items, `rbw status --json` takes about 2 ms, `rbw item list` 5 ms and
`rbw item get` 2 ms.

## Tests

`cargo test -p rbw-desktop-core` runs the unit tests: the fake backend, the
JSON mapping and error codes, with no server.

`RBW_NG=path/to/rbw-ng crates/core/tests/vaultwarden.sh` runs the real backend
against Vaultwarden in Docker. It builds rbw-ng, starts a server on
127.0.0.1, makes three accounts (one with an organization whose login asks for the
master password again, one with two-step login, which the test completes
with an authenticator code, and one that registers with its API key) with
rbw-ng's `tools/scripting-check`, runs the tests with a private `HOME`, and removes the
server and the `HOME` afterwards. It needs Docker, Python 3.10+ and the
`cryptography` package.
