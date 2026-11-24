# Development

## Crates

| Crate | Package | What it holds |
| --- | --- | --- |
| `crates/core` | `rbw-desktop-core` | The `Backend` trait over rbw and `FakeBackend` with demo vaults. No GPUI. |
| `crates/ui` | `rbw-desktop-ui` | The design system: theme tokens, motion, window chrome, the sidebar, components. Its `gallery` example shows every component. |
| `crates/app` | `rbw-desktop` | The app. `window.rs` is the main window, which shows one screen at a time; one module per screen (`sign_in.rs`, `lock.rs`, `vault/`); `settings/` the Settings window, a module per page; `accounts.rs` the shared accounts and the account menu; `reprompt.rs` the master password prompt of protected items; `icons.rs` websites' icons; `generator.rs` the password generator; `panel.rs` the menu bar item's panel and `tray/` the item itself; `preferences.rs` the shared preferences; `recent.rs` notes the items used; `instance.rs` keeps one copy running; `alerts.rs` asks before what can't be undone and says what happened; plus `backend.rs`, `clipboard.rs` and `demo.rs`. |

GPUI comes from `gpui-kit` with its default features off: GPUI itself (the
`gpui-pre` snapshots of Zed's GPUI) and `gpui-base`, the unstyled behaviour
layer (activation, focus, text editing, accessibility roles, transitions,
reduced motion). Nothing from the styled `gpui-component` layer is used. The
workspace names the dependency `gpui`, because GPUI's macros expand to
`gpui::…` paths.

The app runs on `RbwBackend`: the `rbw` in `RBW_DESKTOP_RBW`, on `PATH`, or
bundled with the app (`crates/core/README.md` says where). When it is missing or too old the main window says so
and offers to look again (`backend::connect`, `crate::start`).

`RBW_DESKTOP_DEMO`, set even empty, runs it on `FakeBackend` instead
(`demo.rs`): without words both demo accounts start locked; `first-run`,
`unlocked`, `signed-out`, `touch-id`, `slow`, `offline`, `no-rbw`,
`old-rbw`, `outdated-rbw`, `no-icons`, `big`, `panel`, `settings` and the
`ssh-*` words, comma-separated, reach the other states (`demo.rs` lists
them); `panel` opens the menu bar panel alone, as a click on the item
would, and `settings` the Settings window over the main one. The demo's
master password is `demo` and its codes `123456`; an email with `+device@`
asks for a new device code and one with `+apikey@` for the API key
`user.d3m0` / `demo-secret`; a self-hosted address with `.invalid` in it has
no Bitwarden server. The Bank login and the Visa card ask for the master
password again. The demo runs beside a copy of the app that is already
running, which an ordinary launch would bring forward instead.

## Build and run

```sh
cargo run -p rbw-desktop
RBW_DESKTOP_THEME=gnome-dark cargo run -p rbw-desktop   # macos, gnome, or either with -light / -dark
cargo run -p rbw-desktop-ui --example gallery
cargo fmt && cargo clippy --all-targets                 # clean before every commit
tools/check-macos                                       # type-check and lint the macOS build from Linux
```

On Ubuntu the build needs `libxkbcommon-dev libxkbcommon-x11-dev
libwayland-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev
libvulkan-dev libssl-dev libzstd-dev`; screenshots also need `xvfb xdotool
xcompmgr hsetroot x11-utils mesa-vulkan-drivers imagemagick ffmpeg`, and
`tools/e2e` `xclip`, `dbus-daemon` and Python's `gi`. The packages need
`dpkg-dev`, `desktop-file-utils`, `appstream`, Docker and cargo-about
(`cargo install --locked --features cli cargo-about`); the icons
`librsvg2-bin`.

## Screenshots

Look at a screenshot after every visual change, next to the mockup it
implements.

```sh
tools/shot vault out.png                          # tools/shot --help lists the states
tools/shot --theme gnome-dark sheet out.png
tools/shot --size 680x470 vault out.png           # the mockups' window size
tools/shot --then 'type "Netflix"' --then 'key ctrl+Return' sheet out.png
tools/shot --record 0.9 sheet out.png             # also out-frames/NNN.png at 30 fps, to check motion
tools/shot --demo slow lock-busy out.png          # more RBW_DESKTOP_DEMO words
tools/shot --theme gnome-dark ssh-other out.png   # settings states shoot the Settings window
tools/shot panel-generator out.png                # panel states shoot the panel, opened alone
tools/shot --keyboard --then 'key Tab Tab' vault out.png   # the keyboard's focus ring
tools/shot --binary target/package/linux/rbw-desktop-*/bin/rbw-desktop vault out.png   # a package's app
```

Each state starts the demo it needs. It runs the app on a private Xvfb with Mesa's software Vulkan at 2x, over a
wallpaper with `xcompmgr`, reaches the state with real keys and clicks, and
crops around the window. `--then` runs `xdotool` with `$window` and `$scale`
set; pointer positions are in device pixels from the window's top-left, which
on Linux includes a 12-point invisible frame border. Without a window manager
GPUI does not see the pointer leave, so the script rests it on an empty spot
before capturing.

The shared X11 setup lives in `tools/lib/x11.sh`, which `tools/shot` and
`tools/e2e` source. It gives the display a session bus of its own, so the
app reaches none of this machine's desktop services, and `x11_tray` adds a
stand-in system tray (`tools/lib/sni-watcher.py`, a StatusNotifierWatcher
as KDE's) for the app's item: `x11_tray_item`, `x11_tray_click` and
`x11_tray_menu LABEL` find it, click it and choose from its menu over D-Bus,
as a tray would. Without a window manager GPUI knows no active window, so
nothing that depends on it, such as the panel closing when it loses the
keyboard, happens under Xvfb.

`--keyboard` rests the pointer before the input rather than after it: any
movement of the pointer hides the keyboard's focus ring, as focus-visible
does on the web.

## Performance

```sh
tools/perf                     # release build with frame timings, demo `big`
tools/perf --demo no-icons     # without the website icons' requests
tools/perf --no-build
```

`tools/perf` runs a release build with the `frame-stats` feature, which
prints how long each window took to draw its frames every second it drew
some, on the demo's `big` vault (5,000 more logins, whose icons take 300 ms
each). It types, hovers and scrolls through the vault and the menu bar
panel and sums each up: frames drawn, the slowest, and how many took over
16 ms; and how long Return takes to copy the panel's first result. Keep
every frame under 16 ms. A main thread so busy that it can't print its
timings shows as no frames at all.

## The real backend, end to end

```sh
RBW_NG=~/src/rbw-ng tools/e2e            # builds rbw-ng into target/rbw-ng
RBW_NG=~/src/rbw-ng tools/e2e --no-build
```

`tools/e2e --app path/to/bin/rbw-desktop` drives a packaged app instead,
on the `rbw` it bundles, with none on `PATH`, and checks that the agent
that runs is the bundled one; its screenshots go to
`target/shots/package/e2e`.

`tools/e2e` starts a throwaway Vaultwarden in Docker on 127.0.0.1, makes two
accounts with rbw-ng's `tools/scripting-check` helpers (the first with
shared items, some protected), runs the app on rbw with a private `HOME`,
`XDG_RUNTIME_DIR` and no `SSH_AUTH_SOCK`, and drives it with keys and
clicks: add an account on a self-hosted server, open, copy (pasted back
into the search to check the clipboard), create, edit (the name, then
notes on two lines, another website and a custom field), favourite, move,
delete, a protected item, a copied password cleared by locking, lock,
unlock, a second account, switch, the menu
bar panel from the stand-in tray (a generated password, a login found by
its search, the same login among the items used last, lock), closing the
last window and opening it again from the tray, the lock timeout, the SSH
agent set up and removed with the vault staying unlocked, sign out, and
Quit from the tray. Each step is
checked with the `rbw` command line in the same `HOME`, or the clipboard
with `xclip`, and screenshotted to `target/shots/e2e/NN-step.png`. A stand-in `systemctl` first on `PATH`
records what rbw asks of systemd and refuses `start`, so nothing of this
machine's user services changes; rbw then starts its agent itself. Run it
after any change to how the app calls the backend.

## Size and start-up

```sh
tools/footprint                # release build: binary, launch to window, memory idle
tools/footprint --binary target/dist/rbw-desktop
```

`tools/footprint` runs the demo on the private display: the size of the
binary, the time from launch to the window shown (the best and the median
of five), and the memory, resident and proportional, once idle with the
vault open, then with only the menu bar item left. Mesa's software Vulkan
counts towards the memory there, about 100 MB of it, which a GPU's driver
mostly keeps elsewhere.

The packages are built with the `dist` profile, the release profile with a
whole-program LTO, which trims 7% off the binary and takes three times as
long to build. Most of the rest is GPUI's and the menu bar item's: D-Bus
(zbus), wgpu and naga, the Secret Service client, and every image format of
the `image` crate, which GPUI turns on.

## Packages

```sh
RBW_NG=~/src/rbw-ng packaging/linux/package.sh    # target/packages: .tar.xz and .deb
packaging/linux/check.sh                          # install and run them in containers
RBW_NG=~/src/rbw-ng packaging/macos/bundle.sh     # on a Mac: Fastwarden.app and a .dmg
RBW_NG=~/src/rbw-ng packaging/macos/bundle.sh --dry-run --universal   # its layout, anywhere
packaging/icon/render.py                          # after changing the icon
```

Every package bundles `rbw` and `rbw-agent`, built from the rbw-ng checkout
in `RBW_NG` (only read; built into `target/package/rbw-ng`) where the app
finds them (`crates/core/README.md`): off `PATH`, so they never take the
place of a terminal's `rbw`. Releases bundle the revision in
`packaging/rbw-ng-revision`; the scripts say when the checkout is at
another. `packaging/lib.sh` holds what the scripts share: the app's id, name
and version, read from the code, the rbw-ng build, and the licences every
package carries: ours, rbw-ng's, the Inter font's, the Rust crates' of both
as cargo-about lists them (`packaging/licenses`), and a `copyright` file
that says which is which.

**The icon.** `packaging/icon/icon.svg` is the drawing, in the macOS icon
grid (an 824-point rounded square with continuous corners on 1024), which
reads on Linux too; `icon-small.svg` is its 16 to 32 pixel version, on the
pixel grid and without the keyhole. `render.py` (rsvg-convert) writes the
Linux theme's PNGs and `AppIcon.icns`, which are committed. Look at the
16-pixel icon after any change. The menu bar item's icon is
`crates/app/assets/menu-bar-symbolic.svg`.

**Linux.** `package.sh` builds the `dist` binary and lays out a prefix:
`bin/rbw-desktop`, `libexec/rbw-desktop/{rbw,rbw-agent}`, the desktop file
and AppStream metadata from `packaging/linux`, the icons and
`share/doc/rbw-desktop`. The tarball is that prefix and unpacks into
`~/.local`; the .deb puts it under `/usr`, with its dependencies from
`dpkg-shlibdeps` plus what GPUI opens as it starts (Vulkan or EGL, and
Wayland's client library). The binaries need the glibc of the machine that
builds them or a later one: the release builds on Ubuntu 24.04, for glibc
2.39. `check.sh` validates the metadata and installs the .deb on Ubuntu
24.04 and Debian 13 and the tarball on Fedora 43 in containers, where it
checks the libraries and the bundled `rbw` and opens the demo's window.
The AppStream screenshots are `docs/screenshots/linux-*.png`, taken with
`tools/shot --theme gnome-light` (`gnome-dark` for the item); the metadata
links them on GitHub.

There's no Flatpak. rbw-agent has to outlive the app and be the one the
`rbw` command line reaches, through sockets in `$XDG_RUNTIME_DIR`; the SSH
agent is a systemd user service that `ssh` outside the app reaches; and the
whole design is that the app and the terminal share one agent. A sandbox
would need host access for each of these (`flatpak-spawn --host`, the
session's runtime folder, systemd), which leaves little of it while adding
its costs.

**macOS.** `bundle.sh` builds `Fastwarden.app` for this Mac's architecture
or, with `--universal`, both: `Contents/MacOS/rbw-desktop`,
`Contents/Helpers/{rbw,rbw-agent}`, `Info.plist` from
`packaging/macos/Info.plist` (macOS 12 or later, GPUI's Metal shaders'
minimum), the icon and the licences. `check.py` checks the layout, the
plist and the icon, on any system. It signs with the hardened runtime, ad
hoc unless `--sign` names a Developer ID identity, the helpers first;
`--notarize` notarizes and staples the app and the .dmg with
`NOTARY_PROFILE` or an App Store Connect API key (`NOTARY_KEY`,
`NOTARY_KEY_ID`, `NOTARY_ISSUER`). Neither the app nor rbw-agent needs an
entitlement: rbw-agent's Touch ID unlock makes a Secure Enclave key that
isn't kept in the keychain and lives as long as the agent, and
LocalAuthentication needs none. `tools/smoke-macos` runs a built app on the
demo for CI and keeps screenshots and logs. Without a Mac, `--dry-run`
lays the bundle out from stand-in executables and checks it.

## Continuous integration and releases

`.github/workflows/ci.yaml` runs on every push to main and pull request:
formatting, clippy, `tools/check-macos`, `cargo deny`, shellcheck and the
metadata validators; the tests; a build, the tests and `tools/smoke-macos`
on a macOS runner; `tools/e2e` against Vaultwarden; the Linux packages with
`packaging/linux/check.sh` and the macOS dry run. Pushes to main also build
the macOS bundle and its .dmg, signed ad hoc.

`.github/workflows/mac-check.yaml` runs on pushes to a branch named
`mac-check` and on request: it builds the app on the macOS runner and
`tools/mac-check` screenshots it over a busy, colourful desktop
(`tools/lib/mac-backdrop.swift`, since the runner's own is black and hides
how translucent surfaces look), in the light and the dark appearance: the
vault, Settings, the lock screen, the panel and the vault behind another
app. Download them with `gh run download RUN -n mac-check`. The runner
draws no blur, so materials show as their solid colours there; its
display is 1024 by 768 at 1x. The jobs share
`.github/actions/setup`: the system libraries, the pinned Rust, a cache, and
rbw-ng at the pinned revision, which must be pushed to GitHub.

To release:

1. Set the version in `Cargo.toml` and add a `<release>` to
   `packaging/linux/*.metainfo.xml`. Update `packaging/rbw-ng-revision` if
   the app needs a newer rbw-ng.
2. Commit, then push a tag of the bare version, such as `0.1.0`.
3. `.github/workflows/release.yaml` builds the universal .dmg and the Linux
   packages for x86_64 and arm64, checks them (the Linux ones end to end
   against Vaultwarden, on the rbw they bundle), and attaches them to a
   draft release.
4. Read the draft, then publish it.

The .dmg is signed with a Developer ID and notarized when the repository
has the secrets the workflow lists, and signed ad hoc otherwise.

## Patterns

**Preferences** (`preferences::Prefs`) are read once and shared: change
them with `preferences::update(cx, |preferences| …)`, which applies the
change at once (the clipboard's clearing, website icons) and saves it, and
observe `Prefs::global(cx)` to follow them. A failed save puts them back
and emits `SaveFailed`, which Settings shows.

**The generator** (`generator::Generator`) is one entity wherever it shows:
rendered whole in the panel and the main window's popover, or only
`render_options` in the editor, which subscribes to `Generated` for each new
value. It remembers its options in the preferences, makes a new value as
soon as an option changes (one request at a time, the latest options
next), and lays the value over the room the widest value of these options
would take, so that a new one never moves anything; when that room
changes, with the length, it grows to it (`motion::grow`). Its strength comes
from `GeneratorOptions::strength`, never from the value.

**Recently used.** Call `recent::used(account, item, cx)` when something of
an item is copied, when it is opened with a click or its website opened,
and when it is saved; the panel lists them. Ids only, kept by the backend.

**The current account** is the shared accounts' `current()`: the one the
main window chose last (`choose`), else the first unlocked, else the
first. The panel and the menu bar item follow it.

**The menu bar item and its panel.** `tray::init` shows the item while the
preference says so: `tray/macos.rs` (tray-icon, a template image, the panel
hung under the item) and `tray/linux.rs` (ksni, a StatusNotifierItem with a
symbolic icon written to the session folder and pixmaps as a fallback).
Both report to `tray::MenuBar` through a channel, on the app's thread.
`panel::toggle(anchor, cx)` opens or closes the panel, a window of its own
(`chrome::panel_options`). Closing the last window quits the app unless
`tray::keeps_app_running`, which is true while something shows the item;
GPUI's own quitting is off (`QuitMode::Explicit`). Each window handles
`CloseWindow` (Ctrl-W, Cmd-W) itself.

**One copy at a time** (`instance.rs`): a launch first offers itself to the
socket in the session folder; the copy listening there brings its main
window forward and the new one exits. The demo doesn't take part.

**Accounts** are one shared entity (`accounts::Accounts`), which asks the
backend every two seconds while a window shows, every 30 seconds while
none does, and emits `Refreshed` after each answer. Every window follows
it, so a lock or sign-out anywhere shows everywhere at once; after
changing an account, call `accounts::refresh(cx)` rather than waiting for
the poll, and call it when a window opens. A screen given the same
account again redraws nothing.

**The main window** (`MainWindow`) shows the screen the current account's
state calls for; on a change the old screen fades out, then the new one
in (`motion::screen_leave`, `motion::screen_replace`), so the two are never
seen at once. Screens don't poll: the window passes them the
account's latest state (`set_account`). A screen asks the window for things
by emitting a `Request` (show an account, add one, sign out).
`window::with_main_window` reaches it from another window, such as
Settings' Add Account, opening it first if it is closed.

**Alerts and toasts.** Each window view keeps the `Entity<Alerts>` that
`alerts::Alerts::new(window, cx)` makes and draws it as its last child.
`alerts::confirm(Confirmation { … }, window, cx, |window, cx| …)` asks
before something that can't be undone and `alerts::toast` says something
happened, from any view in the window. An alert's default button has the
keyboard and takes Return: Cancel when confirming removes something.

**Settings** (`settings/`) is its own window, opened with `secondary-,`, the
app menu's Settings… on macOS and the sidebar's primary menu on GNOME, and
laid out per look: a sidebar on macOS, a view switcher in the header bar
on GNOME. Each page is an entity. Rows are `settings_group` and
`setting_row` from the design system, choices a `Select` (an entity that
emits `Selected`). Each account's `AccountSettings` live in one
`settings::store::Store`, written back at once; a failed save puts the
setting back and says so. Preferences apply at once too: the
`clipboard::ClearAfter` global and `Icons::set_enabled`.

**Protected items.** When `item`, `verification_code` or `edit_item` fail
with `Error::RepromptRequired`, show a `reprompt::Reprompt` (the lock
screen's field, for one item); it emits `Unlocked` once `unlock_item` took
the password, then try again. The detail pane shows it in place of the
fields, the editor above its buttons.

**Website icons.** `icons::icon(account, summary, cx)` says what a tile
shows: the icon, its letter when the website has none, or `Pending`, a
neutral tile, while that isn't known; call it only while drawing the row
that shows it, which is what asks for it. An icon rbw keeps on disk
(`Backend::cached_website_icon`) is read off the main thread and never
asks the server: a list's first rows with the list itself
(`icons::read_cached` in its load, then `Icons::insert_cached`), the rows
a few screens around those drawn ahead of them (`Icons::prefetch`), any
other in a batch at the end of the update. Only one rbw never kept is
asked of the server, at most three at once, on threads of their own
(`backend::request_on_thread`), so that a first fetch never holds up what
people wait for; when one arrives the views observing `Icons::global(cx)`
redraw and the rows still on screen ask for the next. A website without
an icon is asked again after an hour, one that failed after a minute.
Icons are decoded to at most 128 pixels a side, the 400 shown last are
kept, and SVG is never decoded. `tile(name).icon(id, icon)` fades the
icon, or the letter's colour, in from the neutral tile; what was known
when the tile first drew shows at once.

**Context menus**: `popover(…).opening(Opening::At(position))`, opened on a
right-click and from the keyboard with Shift-F10 or the Menu key (the
vault's list does both).

**Submenus**: `popover(…).submenu(menu_item(…), items)` adds a row with a
chevron whose items open beside the menu, as Move to Folder does. The
popover handles it as AppKit and GTK do: it opens once the pointer rests
on the row, stays open while the pointer crosses other rows towards it,
and closes once another row is rested on; Right, Return or a click open
it with its first item focused, Left closes it, and Up and Down in the
menu pass it by.

**A screen** is a view entity holding its state, `impl Render`, and methods
for its actions; long renders split into `render_*` methods in submodules
(`vault/sidebar.rs`, `vault/list.rs`). A part that redraws on its own, like
the item detail with its live code, is an entity of its own. Declare the
screen's actions with `actions!`, bind keys under the screen's key context in
its `init`, and handle them with `.on_action(cx.listener(Self::method))`.

**The clipboard.** `clipboard::copy_text` for what isn't secret, such as a
username; `clipboard::copy_secret(value, account, cx)` for a secret, which
goes after the preferences' delay, when its account locks or signs out,
and when the app quits, unless something else was copied since.

**The item editor** (`vault/editor/`) reads the item into text fields
through `form::texts` and builds what it saves with `form::apply`, which
writes back only the fields that changed: an item saved unchanged is the
same item, and the save sends nothing. `form.rs` is tested without a
display; keep any new field's round trip there.

**Calling the backend.** Every `Backend` call blocks. Use
`backend::request(cx, |backend| backend.items(&id), |this, result, cx| …)`:
the call runs on the background executor and the result comes back to the
view. Keep the returned `Task` in a field when a newer request should cancel
an older one (`load_task`), or `.detach()` it. Use `backend::request_in` when
the answer needs the window, such as setting a text field. Show data as
`Load<T>`. Never blank a pane or show the previous item's data while new
data loads: show what is known already (the item pane draws the list's
summary, and a login's fields from it, until the item arrives, and
copies what was asked meanwhile once it does). Show a failure as the sentence from
`backend::describe(&error)`: in place where data failed to load, in the
window's `ToastSlot` when an action failed. Match on `Error` variants, never
on their text.

**Sheets.** The window view keeps a `SheetSlot` and renders it as one of its
last children; `present(entity)` shows a sheet and `dismiss()` animates it out
and then drops it. The sheet view binds `escape` to its own Cancel action and
calls back into the window view to dismiss (`ItemEditor::close`). Draw the
slot outside the screen's key context, so that none of the screen's
shortcuts act behind the sheet, and trap Tab with `focus_trap` on the
sheet's root. A sheet is no taller than its window: put its fields in a
`flex_1().min_h_0()` column that scrolls, and the buttons after it.

**The sidebar** of the vault and of Settings is `sidebar(id)`, flush with
the window's edges, with `chrome::traffic_lights` at its top on macOS and
its rows in a `scroll_area` padded by `metrics.sidebar_padding`.

**Scrolling.** Anything that scrolls has a scroll bar: a column that
scrolls is `scroll_area(id)` (with `track_scroll(&handle)` when the view
scrolls it itself), and a `uniform_list` gets `scrollbar(id, &handle, cx)`
after it in its parent. Their looks and timing come from `scroll::style`,
which the theme projects onto gpui-base's scrollbar; popovers scroll on
their own once taller than the window.

**The list's divider** (`vault/list.rs`) is a strip that straddles the
list's edge, drawn over both panes after them, so that it is above the item
and the list's scroll bar and under any sheet or menu, and drags with
GPUI's drag API: the panes' row follows `DragMoveEvent`s and
the list's share is saved to the preferences once the divider rests. The
list is a fraction of the row between a minimum and a maximum, so the
layout follows the window without a frame's lag.

**Popovers and menus**: `popover(id, open, &focus, label)` with
`menu_item`s, placed inside the element it hangs from, its `Opening` saying
where it goes. Keep `open` and a `FocusHandle` in the view, focus the handle
when it opens, and close it and put the focus back in `on_dismiss` (a click
outside or Escape). The editor's folder menu and `accounts::menu` (with the
`AccountMenu` trait the screens implement) are the examples.

**Forms**: `text_field(id, &input, label)` (the label is for assistive
technologies; the placeholder shows it), `field_error` under it in a line
that stays when empty so nothing moves, `button(…).busy(true)` while the
backend works, and `motion::shake` after a wrong entry. Escape goes back or
cancels, Return goes on.

**A keyboard shortcut and a menu item.** Add the action to the screen's
`actions!`, bind it in `init` with `KeyBinding::new(secondary!("e"), …)`
(`secondary!` is Command on macOS and Control elsewhere), and add
`MenuItem::action` to `menus()` in `main.rs`.

**Theme and motion.** Colours, sizes and the durations of motion come from
`cx.theme()` and `rbw_desktop_ui::motion`; app code holds none of its own.
Timings of behaviour, such as how often the accounts are asked or how long
a selection rests before its item is fetched, are named constants where
they are used. A new colour goes into `Colors` for all four palettes; text
that says something reads at 4.5:1 on every surface.

**Motion** goes through `rbw_desktop_ui::motion`, whose module comment says
how the app moves, and nothing else names a duration or a curve. What
moves or resizes is on a spring (`SELECTION`, `RESIZE`, `SETTLE`), so that
a new target takes over from where it is and how fast it goes; what
appears in place fades (`ARRIVE`), and what leaves goes faster
(`DISMISS`, `QUICK`); things arriving with a duration rather than a
target (sheets, popovers, screens, `PRESENT`, `SCREEN`) follow
`spring_curve`. The helpers: `presence` for what shows and leaves,
`replace` for content replacing content in place, `appear` for content
arriving where there was none, `enter` for a part settling from a few
points off, `reveal` for a line opening under others, `grow` for a box
that springs to its content's height (the generator's value), `slide`
for a selection along rows (drawn by the rows with `Highlight`, or
`selection_track` for rows of different places, such as a sidebar's),
`settle` for something let go, `screen_replace` and `screen_leave`,
`shake`, `spin`. Key an element animation by what it shows, so that
drawing the same thing again replays nothing: the item pane by its item,
the list by its filter, sort and choices, a new value by its count. They
follow the system's reduced-motion setting, and none lays out what is
around it again except `grow` and `reveal`, whose job it is.

**Keyboard.** Every focusable control draws the focus ring from
the design system (`row_focus_ring` for a row the app builds on
gpui-base's button): outside it on macOS, inside it on GNOME. A list moved through
with the arrow keys is one stop of Tab (`ListRow::in_focusable_list`), and
sheets and alerts trap Tab. Copy shortcuts are the panel's in the vault
too: Cmd/Ctrl-C the password where no text field has the keyboard,
Cmd/Ctrl-Shift-C the username, Cmd/Ctrl-Alt-C the code; in the panel,
Return copies what the item is for and Cmd/Ctrl-O opens it in the vault.
A button with a shortcut says so in its tooltip (`button(…).shortcut(…)`,
or `WithTooltip::with_tooltip` on another element), which waits the
platform's delay, takes its style and shows centred under the control
(GPUI places tooltips at the pointer; ours moves to the control).
`key_hint` names an action with its key in a line, as under the panel's
list. `button(…).toggled(on)` shows that what a button turns on is on.
Escape in the vault empties the search, then leaves its field.

## GPUI gotchas

- Updating or reading an entity from a listener that runs inside that
  entity's update panics ("already being updated"). To reach a parent view
  from a child's listener, dispatch an action (`EditItem`).
- `gpui::open_window` (gpui-kit) already wraps the view in gpui-base's
  `Root`, which gives Tab, Shift-Tab and focus traps. Don't wrap it again;
  reach the window's view through `window.root::<gpui::base::Root>()`
  (`window::main_window`).
- gpui-base's `Button` sets the line height to the font size. Text in a
  row built on it needs its own (`ROW_LINE_HEIGHT`).
- A box shadow is painted under the whole element, so a translucent fill
  shows it. GNOME's translucent fields draw their focus ring as a border.
- Text in a column centred with `items_center` takes its own width and
  doesn't wrap; give a sentence `flex_1().min_w_0()` in a full-width row.
- A deferred popover's `on_mouse_down_out` runs before the elements under
  it; it stops propagation so a click outside a menu only closes it.
- `deferred` draws above everything that isn't, sheets included, and takes
  the pointer there too. Keep it for popovers; to draw a part of the layout
  over its neighbours, make it a later sibling instead.
- `svg()` paints only with its own text colour. `ui::Icon` falls back to the
  inherited one.
- `Hsla::opacity(x)` multiplies the alpha; don't pass `color.a * x`.
- GPUI clips to rectangles. A pane that paints at a rounded window corner
  rounds itself with `chrome::corner_radius`.
- There is no backdrop blur, so sheet and popover surfaces are opaque.
- GPUI's Linux text system draws variable fonts (Adwaita Sans, Cantarell) at
  their default weight only. The UI bundles static Inter, which Adwaita Sans
  derives from.
- `KeyBinding::new` doesn't understand `secondary-`; use `secondary!`.
- `uniform_list` rows must all have the height of the first one.
- Element ids must be stable: transitions and other keyed state live under
  them. Two rows with the same id share a transition.
- On X11, GPUI draws its own frame only when it finds a compositing window
  manager with GTK frame extents; its compositor check looks up the wrong
  selection name, so `tools/shot` advertises one on the root window.
- `gpui-pre-apple`'s build script checks the host OS, so checking for macOS
  from Linux needs the empty shader file `tools/check-macos` creates.
- An element that had the keyboard and goes away takes it with it: nothing
  in the window has focus and key bindings stop working. Move the focus
  before removing it (the protected item's prompt, a menu swapping items).
- `role()` on a `div` needs an `id()` first.
- Under Xvfb, `xdotool key shift+F10` arrives as Alt-Shift-F10; shots use the
  Menu key.
- GPUI quits when the last window closes on Linux unless the application's
  quit mode is `Explicit`.
- A single-line `Input` takes Up and Down and does nothing with them. To
  move through a list from a search field, bind them under
  `"YourContext > Input"`, which the later binding wins at that depth.
- Moving the pointer hides every focus ring (`focus_visible`), as on the
  web; screenshots of focus need `tools/shot --keyboard`.
- A key binding of a context matches before a focused button inside it
  sees the key: a dialog binding Return confirms even from its Cancel
  button. Let the focused button take Return instead.
- A one-line `InputState` drops tabs and line breaks from the value it is
  given; compare what it holds with what it held, not with the item.
- A `TextareaState` with `auto_grow` doesn't take Tab, which moves the
  focus on; Return makes a new line and still emits `PressEnter`.
- Children of a column that scrolls shrink before it scrolls; give them
  `flex_none()`.
- A window's actions need something in it focused: without a focused
  element, keys reach only global handlers. But a click anywhere inside an
  element that tracks a focus handle gives it the keyboard, so a window view
  tracks a focus of its own only while no screen or field holds the
  keyboard (`MainWindow` while loading, `QuickPanel` while it has no search
  field).
- `Window::set_native_titlebar_visibility` exists only on macOS.

## macOS

The macOS-only code is small and type-checked from Linux by
`tools/check-macos`; CI builds and runs it on a macOS runner
(`tools/smoke-macos`, and `tools/mac-check` over a colourful desktop):

- `ui/src/material.rs`: the system's materials under what GPUI draws, an
  `NSVisualEffectView` placed below GPUI's view and moved with an element's
  bounds, blending with what is behind the window, in the app's light or
  dark appearance; GPUI paints nothing there. The sidebar and the screens
  of their own (lock, sign-in) are on the sidebar material, grey while the
  window is inactive; the menu bar panel is on the popover material. With
  Reduce Transparency, or on a Mac that draws no blur, such as CI's, the
  views are the materials' solid colours.
- `ui/src/platform.rs`: the accent colour from `NSColor.controlAccentColor`.
- `ui/src/chrome.rs`: transparent title bar, traffic lights at (19, 18) over
  the sidebar, in the middle of the toolbars' height, the `Blurred` window
  background.
- Fonts: `.SystemUIFont` and Menlo.
- GPUI's `Blurred` window background strips the system material's tint and
  saturation, and blurs nothing where the system doesn't, leaving the
  desktop sharp behind the window; it only shows while one screen replaces
  another. Surfaces that show a material use `ui::backdrop` instead.
- The Settings window's title bar and traffic lights over its sidebar, the
  app menu's About and Settings… items, Touch ID's switch on the Accounts
  page, and the SSH page's advice to add `IdentityAgent` to `~/.ssh/config`.
- `app/src/tray/macos.rs`: the menu bar item (a template image drawn from
  `app/assets/menu-bar-symbolic.svg`, its right-click menu) and where the
  panel hangs, from the item's `NSStatusItem` window; the panel as a
  `PopUp` window on the blurred material without its title bar; staying in
  the Dock when the last window closes and coming back with `on_reopen`.

Everything else, the macOS look included, is the code Linux runs and is
checked with `RBW_DESKTOP_THEME=macos-light` / `macos-dark` screenshots.
