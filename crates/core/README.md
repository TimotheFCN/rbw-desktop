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
| Vault | `sync`, `items`, `item`, `unlock_item`, `create_item`, `edit_item`, `delete_item`, `set_favorite`, `move_item`, `folders`, `create_folder`, `verification_code` |
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
list (`items`) never needs it.

`ItemSummary::editable` and `Item::editable` say whether an item can be
edited and deleted: items in read-only collections can't. Every item can be
marked favourite and moved between folders, which change only what belongs to
the account and send no secrets. Item kinds this version doesn't know are
left out of the list.

`account` is cheap (a few milliseconds): poll it every second or two to notice
locking, unlocking, and syncs through `last_sync`. `Account::touch_id_ready`
says when `unlock_with_touch_id` can work: once Touch ID unlock is on and the
account was unlocked with its password since its agent started.

Account settings (lock timeout, Touch ID, SSH confirmation) are rbw's, per
profile; changing them doesn't lock. Preferences (clipboard clearing, website
icons) are the app's, in `preferences.json` in its configuration folder.

## How it maps to rbw

Each call runs `rbw` once, with `RBW_PROFILE` set for the account,
`RBW_NONINTERACTIVE=1` so that it never prompts, and `RBW_ERROR_FORMAT=json`
so that its errors come with a code. The vault stays unlocked in the profile's
`rbw-agent` between calls. Passwords and items with secrets go to `rbw` on
stdin, never in arguments, and nothing is written to disk or logged. The API
key goes in the environment of the one `rbw register` process, which only
the same user can read, since `rbw` reads it from there.

| Method | Command |
| --- | --- |
| `accounts` | `rbw profiles --json`, then `account` for each |
| `account` | `rbw status --json` |
| `add_account` | `rbw config set email` and `base_url` |
| `sign_in` | `rbw login --password-stdin [--method ID] [--code CODE]` |
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
| `generate` | `rbw generate LENGTH [--no-symbols\|--only-numbers\|--nonconfusables\|--diceware]` |
| `account_settings`, `set_account_settings` | `rbw config show`, `rbw config set` |
| `ssh_agent` | `rbw ssh-agent status --json`, and the SSH keys of `rbw item list` |
| `install_ssh_agent`, `uninstall_ssh_agent` | `rbw ssh-agent install`, `rbw ssh-agent uninstall` |

`RbwBackend::new` looks for `rbw` in `RBW_DESKTOP_RBW`, then next to the app's
executable, then on `PATH`, and uses the `rbw-agent` next to it when there is
one. It needs rbw-ng 1.19 or later, which it recognizes by the fields of
`rbw status --json`, so that development builds work too. Local calls time out
after 30 seconds, calls that reach the server after 120.

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
