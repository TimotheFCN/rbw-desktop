#!/bin/sh
# Prints the notes of a release: its CHANGELOG.md section, then which file
# is for which system. Unless NOTARIZED is set, they say how to open an app
# that Apple hasn't notarized.
#
# usage: packaging/release-notes.sh VERSION
set -eu

cd "$(dirname "$0")/.."
awk -v version="$1" '
    index($0, "## [" version "]") == 1 { found = 1; next }
    found && /^## \[/ { exit }
    found { print }
' CHANGELOG.md | sed -e '/./,$!d' | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}'

cat <<'EOF_NOTES'

## Install

| System | File |
| --- | --- |
| macOS 12 or later, Apple Silicon and Intel | `rbw-desktop-macos-universal.dmg`, or `brew install --cask timothefcn/tap/fastwarden` |
| Ubuntu 24.04, Debian 13 and later | `rbw-desktop_amd64.deb` or `rbw-desktop_arm64.deb` |
| Other Linux with glibc 2.39 or later | `rbw-desktop-x86_64-linux.tar.xz` or `rbw-desktop-aarch64-linux.tar.xz` |

The [README](https://github.com/TimotheFCN/rbw-desktop#install) has the steps.
EOF_NOTES
if [ -z "${NOTARIZED:-}" ]; then
	cat <<'EOF_NOTES'

**macOS blocks the first launch.** Apple hasn't notarized this release, so
macOS says it can't check the app. That is expected for now. Open System
Settings, Privacy & Security, and choose Open Anyway next to Fastwarden.
EOF_NOTES
fi
printf '\nBuilt with rbw-ng at revision %s.\n' "$(cat packaging/rbw-ng-revision)"
