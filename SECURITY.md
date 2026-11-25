# Security

Report a vulnerability in private, not in a public issue: on GitHub, open
the repository's Security tab and choose
[Report a vulnerability](https://github.com/TimotheFCN/rbw-desktop/security/advisories/new).
Say how to reproduce it and what an attacker gains.

Fastwarden is the window. [rbw-ng](https://github.com/TimotheFCN/rbw-ng)
holds the vault's key, encrypts and decrypts, and talks to the server. A
problem in `rbw` or `rbw-agent` can be reported here too: both projects
have the same maintainer.

Only the latest release gets fixes.
