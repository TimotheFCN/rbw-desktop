#!/bin/sh
# Prints the Homebrew cask of a release, with the checksum the release
# workflow leaves in DIST beside the disk image. Unless NOTARIZED is set,
# the cask says how to open an app that Apple hasn't notarized.
#
# usage: packaging/homebrew/cask.sh VERSION DIST > fastwarden.rb
set -eu

version=$1
dist=$2
image=rbw-desktop-macos-universal.dmg
sum=$(cut -d' ' -f1 "$dist/$image.sha256")

cat <<RUBY
cask "fastwarden" do
  version "$version"
  sha256 "$sum"

  url "https://github.com/TimotheFCN/rbw-desktop/releases/download/#{version}/$image"
  name "Fastwarden"
  desc "Fast desktop client for Bitwarden and Vaultwarden"
  homepage "https://github.com/TimotheFCN/rbw-desktop"

  depends_on macos: ">= :monterey"

  app "Fastwarden.app"

  uninstall quit: "io.github.timothefcn.RbwDesktop"

  zap trash: [
    "~/Library/Application Support/io.github.timothefcn.RbwDesktop",
    "~/Library/Caches/io.github.timothefcn.RbwDesktop",
    "~/Library/Saved Application State/io.github.timothefcn.RbwDesktop.savedState",
  ]
RUBY
if [ -z "${NOTARIZED:-}" ]; then
	cat <<'RUBY'

  caveats <<~EOS
    Fastwarden isn't notarized by Apple yet, so macOS blocks its first launch.
    Open System Settings, Privacy & Security, and choose Open Anyway next to
    Fastwarden. Or clear the download flag:
      xattr -dr com.apple.quarantine /Applications/Fastwarden.app
  EOS
RUBY
fi
echo end
