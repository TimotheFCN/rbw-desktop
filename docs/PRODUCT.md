# Product brief

rbw-desktop is a desktop app for Bitwarden and Vaultwarden vaults. It exists
because the official desktop app is slow, heavy and awkward to use. It is
built on [rbw-ng](https://github.com/TimotheFCN/rbw-ng): `rbw-agent` holds the
vault key, syncs and serves SSH keys, and this app is the window onto it.

People know the app as **Fastwarden**: its windows, menus, Dock, About page,
desktop entry and README say so. The project, its repository, crates,
binary (`rbw-desktop`), packages and files, and the application id
(`io.github.timothefcn.RbwDesktop`) keep the name rbw-desktop.

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
   macOS app: a sidebar of the system's material, flush with the window's
   edges under the traffic lights as in Finder, raised controls, the system
   accent colour, light and dark from the system, standard shortcuts and
   menus. On Linux it follows
   GNOME conventions. It never looks like a web page or a developer tool.
4. **Fast.** It opens at once, never blocks the interface on the network or
   on `rbw`, and stays small in memory.
5. **Polished motion.** Navigation, sheets, selection, copy feedback and
   lock and unlock move as the platform's own apps do: what moves or
   resizes on springs that a new change redirects from where it is, what
   appears in place with a short fade, what leaves faster than it came,
   over short distances. Motion explains what changed; none of it is
   decoration, nothing replays when it didn't change, and under the
   system's reduced motion only fades remain.
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
  The lock screen asks for Touch ID by itself when it shows in the window
  in use, so a finger on the sensor is enough; not over another app, not
  right after Lock was chosen in the window, and not again after a cancel,
  until the window comes back to the front. Its button stays, filled, and
  Return with the password field empty asks for it too.
* Lock from the sidebar, the menu bar item, the app menu, and automatically
  after the timeout the account is configured with.

### Vault

* Browse all items, favourites, by type (logins, cards, identities, notes,
  SSH keys) and by folder, "No Folder" included. Search as you type.
* A menu next to the search field sorts the list by name, by last change
  or by recent use (remembered), and shows only the logins with a
  verification code or only the items shared with the account. What it
  shows only is written under the search field, with a button that shows
  everything again.
* The window opens where it was last, on the same display, at its size,
  unless no display shows that place any more.
* The list takes a share of the window, which dragging its edge changes
  (remembered; a double-click puts it back).
* Item view: every field copies on click, with a brief "Copied" confirmation.
  Choosing an item shows it complete at once when it can: as the pane
  showed it before, or as fetched ahead for the row under the pointer and
  the next one down the list; otherwise its name and what the list knows
  of it, then the rest as it arrives. Content that is there doesn't fade;
  only what comes after a wait does. Nothing of the previous item stays.
  Passwords and other hidden fields reveal on request. Verification codes
  count down live. Websites open in the browser.
* Create, edit and delete items; mark favourites; move between folders.
  The item's sheet shows what is being made at its top: the item's tile
  (its website's icon once the address is one with an icon kept, else
  its letter, or its kind's glyph until it has a name) beside its name.
  A new item's kind is chosen from coloured tiles: login, card, identity,
  note or SSH key (made from its private key). Every field has its glyph,
  a plain label and an example in it; the name says it is required only
  when Save finds it empty. New logins get a generated password, which
  says how strong it is from the randomness its options hold; a typed
  one gets no score. Everything the item view shows can be edited, the
  less common fields behind "Add Field", whose menu says what each is
  for; saving changes only what was changed. Cmd/Ctrl-S or
  Cmd/Ctrl-Return saves; Escape asks before throwing changes away. The
  sheet grows and shrinks with its fields, and scrolls inside once it is
  as tall as the window.
* The clipboard is cleared after a copied secret has sat there for a while,
  when the account it came from locks or signs out, and when the app quits.
* Copy shortcuts: the password, the username and the verification code of
  the selected login, in the vault as in the menu bar panel.
* Every button that shows only an icon says what it does, and its
  shortcut, in a tooltip under it as soon as the pointer rests on it;
  where there is room, a button has a label instead.
* Website icons show with their row when they were fetched before; one
  fetched the first time fades in on a neutral tile, and a website
  without one shows the item's coloured letter.
* An item's menu moves it to a folder from a submenu, which opens when
  the pointer rests on Move to Folder, as native submenus do.
* Escape empties the search, keeping the item chosen.
* Sync runs in the background and on request.

### Generator

* Passwords, passphrases and PINs, using rbw's generator: length (or word
  count); for a password, numbers, symbols, and "easy to read" (lower-case
  letters and digits that can't be mistaken for each other), and a
  password always holds what its switches promise; for a passphrase,
  capitals, a number, and what goes between the words (a hyphen, a space
  or nothing).
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
  Two tabs: the items used last, with a search through the whole vault,
  and the generator. One row is selected, as in Spotlight: the arrow keys
  move it, and so does the pointer when it moves onto a row (not when the
  panel opens under it or the list scrolls under it). A click or Return
  copies what the selected item is for: a login's password, a card's
  number, a note's text, an identity's name, an SSH key's public key. The
  selected row has a button for each thing it can copy (a login's
  username, password and verification code, a card's number and security
  code, and so on, leaving out what the item hasn't got) and one that
  opens the item in the main window. A line under the list says what
  Return does to the selected row, with ↵, or what the button under the
  pointer does, with its own shortcut. Under it, "Open Vault" and "Lock".
  An item that asks for the master password again asks for it in the
  panel, then copies what was asked for; Escape goes back to the list. It
  shows the account the main window shows. Nothing about SSH lives here.
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
* The app never sees the vault key. Hidden values (passwords, card
  numbers and codes, hidden fields, private keys, identity numbers) are
  fetched when the item is shown or copied and kept only while it is. To
  draw an item again at once, the vault remembers what the item pane
  shows of the items it showed, with stand-ins for their hidden values,
  and a verification code until it expires; it forgets them when the
  account locks, syncs or is switched. An item that asks for the master
  password again is never remembered or fetched ahead.

## Engineering

* Rust, GPUI, a Cargo workspace under `crates/`.
* The code that talks to rbw has no GPUI dependency and is tested without a
  display. The interface is built from a small design system, not ad hoc
  styling.
* Clean and small: no dead code, no speculative abstraction, comments only
  where the code cannot say why.
