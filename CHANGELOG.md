# Changelog

## [0.1.0] - 2026-11-26

Security hardening, faster syncs and polls with rbw-ng 1.19.0, and smoother
rendering across large vaults.

### Security

* Mark copied secrets as concealed and transient on the macOS pasteboard so
  clipboard managers and Universal Clipboard skip them, wipe clipboard
  readback buffers, and clear generated passwords on lock.
* Copy Secure Notes as secrets (auto-cleared after the configured delay and on
  lock), mask notes in cached item shapes, and drop cached vault views and
  item shapes immediately on lock or account switch.
* Require master-password reprompt before deleting protected items and expire
  item reprompts after 120 seconds of idle time.
* Tighten config and session directory/file permissions (`0700` / `0600`), use
  unique atomic writes for JSON files, harden website icon fetching against
  private addresses and oversized files, and reject control characters and
  whitespace in URLs opened from items.
* Fix `rbw` non-interactive status polls overwriting the agent's interactive
  environment, keeping SSH agent confirmation prompts working while
  Fastwarden is open.

### Performance & Reliability

* Use `rbw profiles --status`, `rbw item get --with-code`, and
  `rbw item list --type` from rbw-ng 1.19.0, coalesce overlapping status polls,
  and cache the vault snapshot for instant frame-0 menu bar panel opens.
* Keep scrolling, hovering, and searching under 16 ms per frame on 5,000-item
  vaults: isolated TOTP countdown updates, precomputed sidebar counts and
  search blobs, host-keyed icon caching, lazy popovers and Settings pages,
  and gap-free list pointer tracking.
* Make startup backend discovery non-blocking with loading placeholders,
  serialize preference saves, and fix subprocess timeout hangs.

## [0.0.1] - 2026-11-25

The first release, for macOS 12 or later and Linux, with rbw-ng 1.19.0
inside.

* Several accounts side by side, on bitwarden.com, bitwarden.eu or your own
  server, with two-step login and new device codes.
* Logins, cards, identities, notes and SSH keys to search, sort, copy,
  create, edit, move and delete. Verification codes count down.
* A generator for passwords, passphrases and PINs.
* A menu bar item, or a tray item on Linux, with the items used last and
  the generator. It keeps the app running once its windows are closed, and
  out of the Dock on macOS.
* Unlock with Touch ID on a Mac that has it.
* rbw's SSH agent, set up as a login service from Settings.
* The `rbw` command line tool, put on `PATH` from Settings.
