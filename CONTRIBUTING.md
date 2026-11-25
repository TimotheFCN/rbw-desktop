# Contributing

Bug reports, ideas and pull requests are all welcome.

## Reporting a bug

[Open an issue](https://github.com/TimotheFCN/rbw-desktop/issues/new/choose)
and say what you did, what you expected and what happened. Add your system,
Fastwarden's version from Settings, About, and whether your server is
bitwarden.com, bitwarden.eu or Vaultwarden. A screenshot helps for anything
visual. Hide your email and item names first.

Never paste a password, a session key or a private key into an issue. A
vulnerability goes through [SECURITY.md](SECURITY.md), not the issue
tracker.

## Suggesting a change

Open an issue before you write anything larger than a fix.
[docs/PRODUCT.md](docs/PRODUCT.md) says what the app is for and what it
leaves out on purpose, such as counters, keyboard hints and anything that
reads like a developer tool. A feature that needs something new from
`rbw` starts in [rbw-ng](https://github.com/TimotheFCN/rbw-ng), in a form
that is useful to any program that drives it.

## Working on the code

You need a current stable Rust. On Linux, install the libraries listed in
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md#build-and-run).

```sh
git clone https://github.com/TimotheFCN/rbw-desktop
cd rbw-desktop
RBW_DESKTOP_DEMO= cargo run -p rbw-desktop          # demo vaults
RBW_DESKTOP_DEMO=unlocked cargo run -p rbw-desktop  # straight into a vault
cargo run -p rbw-desktop-ui --example gallery       # every component
```

The demo needs no account and no `rbw`. Its master password is `demo` and
its codes are `123456`. On Linux, `RBW_DESKTOP_THEME=macos-light` (or
`macos-dark`, `gnome-light`, `gnome-dark`) shows either system's look.
To run on a real vault, have rbw-ng 1.19.0 or later on your `PATH`, or
name its `rbw` in `RBW_DESKTOP_RBW`.

The code is in three crates. `crates/core` talks to `rbw` and has no GPUI
in it, `crates/ui` is the design system, and `crates/app` is the app.
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) describes each, the patterns
the code follows and the GPUI pitfalls we have already hit.

## Before you open a pull request

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --workspace
tools/check-macos -- -D warnings   # on Linux: type-checks and lints the macOS code
```

CI runs these on Linux and macOS, drives the app end to end against a
Vaultwarden container (`tools/e2e`) and builds the packages. You don't
need a Mac to change macOS code, or Linux to change Linux code: say in the
pull request what you couldn't run and CI covers the rest.

For a visual change, look at a screenshot of it in both looks and in light
and dark, and attach them to the pull request. On Linux, `tools/shot` takes
them without touching your desktop:

```sh
tools/shot --theme macos-dark vault out.png   # tools/shot --help lists the states
```

Add a line to [CHANGELOG.md](CHANGELOG.md) for anything a user would
notice. Keep a pull request to one change, and write its commit messages
as a sentence that says what the app now does.
