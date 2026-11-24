# Product brief

rbw-desktop is a desktop app for Bitwarden and Vaultwarden vaults. It exists
because the official desktop app is slow, heavy and awkward to use. It is
built on [rbw-ng](https://github.com/TimotheFCN/rbw-ng): `rbw-agent` holds the
vault key, syncs and serves SSH keys, and this app is the window onto it.

## Who it is for

Anyone. A person who has never opened a terminal must be able to sign in,
find a password, copy it and save a new one without reading anything.

## Principles

1. **The vault is the app.** Opening it shows your items. Everything else is
   secondary.
2. **Nothing useless on screen.** No counters, status bars, keyboard hints,
   process ids or jargon. If an element doesn't help someone find, copy or
   save a secret, it goes.
3. **Native on each platform.** On macOS it looks and behaves like a current
   macOS app: glass sidebar and controls, system accent colour, light and
   dark from the system, standard shortcuts and menus. On Linux it follows
   GNOME conventions. It never looks like a web page or a developer tool.
4. **Fast.** It opens at once, never blocks the interface on the network or
   on `rbw`, and stays small in memory.
5. **Polished motion.** Navigation, sheets, selection, copy feedback and
   lock and unlock are animated with short, eased transitions. Motion
   explains what changed; none of it is decoration, and it is off when the
   system asks for reduced motion.
6. **Plain words.** "Verification code", not "TOTP". "Website", not "URI".

The mockups in `design/mockups` are the reference for look and density.
Open the `.html` files, or rebuild them with `design/mockups/src/build.py`.

## What it does

### Accounts

* Accounts are rbw profiles on this computer, shared with the `rbw` command
  line. The app lists the profiles that exist and can add new ones.
* Adding an account: choose bitwarden.com, bitwarden.eu or a self-hosted
  server, then email, master password, and a two-step or new-device code
  when the server asks for one.
* Several accounts side by side, each locked or unlocked on its own. Switch
  from the bottom of the sidebar. Sign out removes the account from this
  computer.

### Locking

* Unlock with the master password, or Touch ID on a Mac that has it.
* Lock from the sidebar, the menu bar item, the app menu, and automatically
  after the timeout the account is configured with.

### Vault

* Browse all items, favourites, by type (logins, cards, identities, notes,
  SSH keys) and by folder. Search as you type.
* Item view: every field copies on click, with a brief "Copied" confirmation.
  Passwords and other hidden fields reveal on request. Verification codes
  count down live. Websites open in the browser.
* Create, edit and delete items; mark favourites; move between folders.
  New logins get a generated password by default.
* The clipboard is cleared after a copied secret has sat there for a while.
* Sync runs in the background and on request.

### Generator

* Passwords, passphrases and PINs, using rbw's generator: length (or word
  count), symbols, and "easy to read" (lower-case letters and digits that
  can't be mistaken for each other). rbw offers no letters-only password,
  so there is no "numbers" switch.
* The result is large, its digits and symbols coloured; Copy is the main
  action, with another value next to it. A password or passphrase says
  whether it is weak, fair or strong, from the randomness its options hold;
  a PIN says nothing, since what it unlocks limits the tries.
* The options are remembered, and each change makes a new value at once.
* The same generator is in the menu bar panel, in the main window (a
  toolbar button, the Vault menu, GNOME's primary menu, Ctrl-Shift-G) and,
  its options, in the item editor next to the password.

### Menu bar item

* On every system that has one: the macOS menu bar, and the system tray of
  KDE and other desktops that show StatusNotifierItems (GNOME with the
  AppIndicator extension). Without one, as on stock GNOME, the app is as it
  would be without the item. Settings, General turns it off.
* A click opens a panel: on macOS hung under the item, elsewhere a small
  window next to the pointer (X11) or in the middle of the screen (Wayland).
  Two tabs: the items used last, with a search through the whole vault
  (a click or Return copies a login's password; the selected row also
  copies its username and code), and the generator. Plus "Open Vault" and
  "Lock". It shows the account the main window shows. Nothing about SSH
  lives here.
* The item's menu has Open Vault, Lock and Quit.
* While the item shows, closing the last window leaves the app running.
  Launching the app again brings the running one forward.

### SSH agent

* A page in Settings shows whether rbw's SSH agent is installed and running,
  which agent `ssh` currently uses, and the keys it serves.
* Install and remove the agent service from that page, and choose whether
  each use asks for confirmation.

### Settings

* General (lock timeout, clipboard clearing, website icons), Accounts,
  SSH Agent, About.

## What it does not do

* No quick-access palette or global hotkey for now.
* No browser autofill: that stays with the Bitwarden browser extension.
* No Windows build while rbw-ng is Unix-only.
* Attachments, Sends, passkeys and organization administration are out of
  scope until rbw-ng supports them.

## How it relates to rbw-ng

* rbw-ng stays a general tool. Anything this app needs from it must be
  useful to any program that drives `rbw`, and goes into rbw-ng as such.
  No rbw-desktop-specific code there unless a specific case is clearly
  cleaner that way, and then it is called out.
* The app never sees the vault key. Secrets are fetched when they are shown
  or copied and are not kept.

## Engineering

* Rust, GPUI, a Cargo workspace under `crates/`.
* The code that talks to rbw has no GPUI dependency and is tested without a
  display. The interface is built from a small design system, not ad hoc
  styling.
* Clean and small: no dead code, no speculative abstraction, comments only
  where the code cannot say why.
