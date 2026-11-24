# rbw-desktop

A fast, native-feeling desktop app for Bitwarden and Vaultwarden vaults, built
in Rust with GPUI on top of [rbw-ng](https://github.com/TimotheFCN/rbw-ng).

The app is a front end for the same `rbw` accounts the command line uses:
`rbw-agent` keeps the vault key, syncs and serves SSH keys, and the app talks
to it.

* [What the app is and who it is for](docs/PRODUCT.md)
* [Design mockups](design/mockups)
* [Building, screenshots and code patterns](docs/DEVELOPMENT.md)
