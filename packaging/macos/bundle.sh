#!/usr/bin/env bash
# Builds the macOS app and a disk image holding it into target/packages:
#
# - "rbw Desktop.app": the app, with rbw and rbw-agent built from the rbw-ng
#   checkout in $RBW_NG in Contents/Helpers, where the app finds them and
#   where code signing expects helper tools;
# - rbw-desktop-VERSION-macos-ARCH.dmg: the app and a link to Applications.
#
# It signs everything with the hardened runtime: ad hoc by default, which
# runs on this Mac and, once allowed in System Settings, on others; with
# --sign, as a Developer ID for Gatekeeper, and with --notarize also
# notarized. Neither the app nor rbw-agent needs an entitlement: Touch ID
# unlock keeps its key in the Secure Enclave without the keychain, for as
# long as rbw-agent runs. See docs/DEVELOPMENT.md.
#
#     RBW_NG=path/to/rbw-ng packaging/macos/bundle.sh [options]
#
# --universal      arm64 and x86_64 in one (default: this Mac's architecture)
# --sign IDENTITY  a "Developer ID Application" identity from the keychain
# --notarize       notarize and staple the app and the image, with
#                  NOTARY_PROFILE, a notarytool keychain profile, or an App
#                  Store Connect API key: NOTARY_KEY (the .p8 file),
#                  NOTARY_KEY_ID and NOTARY_ISSUER
# --dry-run        lay the bundle out with stand-in executables, without
#                  building, signing or an image, and check it; runs on
#                  Linux too
set -euo pipefail

usage() {
	echo "usage: RBW_NG=path/to/rbw-ng packaging/macos/bundle.sh" \
		"[--universal] [--sign IDENTITY] [--notarize] [--dry-run]" >&2
	exit 2
}

universal=0 identity=- notarize=0 dry_run=0
while [[ $# -gt 0 ]]; do
	case $1 in
	--universal) universal=1; shift ;;
	--sign) identity=$2; shift 2 ;;
	--notarize) notarize=1; shift ;;
	--dry-run) dry_run=1; shift ;;
	*) usage ;;
	esac
done
if ((!dry_run)) && [[ $(uname -s) != Darwin ]]; then
	echo "packaging/macos/bundle.sh builds on macOS only; this is $(uname -s)." >&2
	echo "--dry-run lays the bundle out and checks it without building." >&2
	exit 1
fi
if ((notarize)) && [[ $identity == - ]]; then
	echo "--notarize needs --sign: Apple notarizes only Developer ID signatures" >&2
	exit 2
fi

root=$(cd "$(dirname "$0")/../.." && pwd)
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-8}
# shellcheck source=packaging/lib.sh
source "$root/packaging/lib.sh"

# GPUI's Metal shaders target macOS 12, and so does everything built here.
minimum_macos=12.0
export MACOSX_DEPLOYMENT_TARGET=$minimum_macos
if ((universal)); then
	targets=(aarch64-apple-darwin x86_64-apple-darwin) arch=universal
elif [[ $(uname -m) == x86_64 && $(uname -s) == Darwin ]]; then
	targets=(x86_64-apple-darwin) arch=x86_64
else
	targets=(aarch64-apple-darwin) arch=arm64
fi

dist=$root/target/packages
work=$root/target/package/macos
app=$work/$app_name.app
rm -rf "$work" && mkdir -p "$work" "$dist"

# Makes one binary of a program built for each target: lipo joins them.
binary() {
	local out=$1
	shift
	local built=("$@")
	if ((dry_run)); then
		printf '#!/bin/sh\n# stands in for %s\n' "$(basename "$out")" >"$out"
	elif ((${#built[@]} == 1)); then
		cp "${built[0]}" "$out"
	else
		lipo -create -output "$out" "${built[@]}"
	fi
	chmod 755 "$out"
}

# The macOS steps, which only a Mac can run: building, signing, the image
# and notarization.

build() {
	local target
	for target in "${targets[@]}"; do
		rustup target list --installed | grep -qx "$target" || rustup target add "$target"
		cargo build --quiet --locked --profile dist --target "$target" \
			--manifest-path "$root/Cargo.toml" -p rbw-desktop
		rbw_ng_build "$target"
	done
}

sign() {
	local options=(--force --options runtime --sign "$identity")
	if [[ $identity == - ]]; then
		options+=(--timestamp=none)
	else
		options+=(--timestamp)
	fi
	# the helpers first: signing the bundle seals them as they are
	codesign "${options[@]}" --identifier "$app_id.rbw" "$app/Contents/Helpers/rbw"
	codesign "${options[@]}" --identifier "$app_id.rbw-agent" "$app/Contents/Helpers/rbw-agent"
	codesign "${options[@]}" "$app"
	codesign --verify --strict --deep --verbose=2 "$app"
}

# Submits a file to Apple's notary service, and fails with its log unless
# Apple accepts it.
notarize() {
	local credentials result id status
	if [[ -n ${NOTARY_PROFILE:-} ]]; then
		credentials=(--keychain-profile "$NOTARY_PROFILE")
	else
		credentials=(--key "${NOTARY_KEY:?set NOTARY_PROFILE, or NOTARY_KEY, NOTARY_KEY_ID and NOTARY_ISSUER}"
			--key-id "${NOTARY_KEY_ID:?set NOTARY_KEY_ID}" --issuer "${NOTARY_ISSUER:?set NOTARY_ISSUER}")
	fi
	result=$(xcrun notarytool submit "$1" "${credentials[@]}" --wait --output-format json)
	id=$(python3 -I -c 'import json, sys; print(json.load(sys.stdin)["id"])' <<<"$result")
	status=$(python3 -I -c 'import json, sys; print(json.load(sys.stdin)["status"])' <<<"$result")
	if [[ $status != Accepted ]]; then
		echo "notarization of $(basename "$1"): $status" >&2
		xcrun notarytool log "$id" "${credentials[@]}" >&2 || true
		exit 1
	fi
}

image() {
	local stage=$work/image
	mkdir -p "$stage"
	cp -R "$app" "$stage/"
	ln -s /Applications "$stage/Applications"
	hdiutil create -quiet -volname "$app_name" -srcfolder "$stage" -fs HFS+ -format ULFO -ov "$dmg"
	if [[ $identity != - ]]; then
		codesign --force --timestamp --sign "$identity" "$dmg"
	fi
}

((dry_run)) || build

contents=$app/Contents
mkdir -p "$contents/MacOS" "$contents/Helpers" "$contents/Resources"
# (macOS's own bash is 3.2: no mapfile)
paths=()
for target in "${targets[@]}"; do paths+=("$root/target/$target/dist/rbw-desktop"); done
binary "$contents/MacOS/rbw-desktop" "${paths[@]}"
for program in rbw rbw-agent; do
	paths=()
	for target in "${targets[@]}"; do paths+=("$(rbw_ng_dir "$target")/$program"); done
	binary "$contents/Helpers/$program" "${paths[@]}"
done
((dry_run)) || rbw_ng_check "$contents/Helpers"
sed -e "s|@APP_ID@|$app_id|g" -e "s|@APP_NAME@|$app_name|g" -e "s|@VERSION@|$version|g" \
	-e "s|@MINIMUM_MACOS@|$minimum_macos|g" "$root/packaging/macos/Info.plist" >"$contents/Info.plist"
cp "$root/packaging/icon/AppIcon.icns" "$contents/Resources/AppIcon.icns"
licenses "$contents/Resources/Licenses" "${targets[@]}"
python3 -I "$root/packaging/macos/check.py" "$app" "$app_id" "$version" "$minimum_macos"

if ((dry_run)); then
	(cd "$work" && find "$app_name.app" | sort)
	exit 0
fi

sign
if ((notarize)); then
	ditto -c -k --keepParent "$app" "$work/app.zip"
	notarize "$work/app.zip"
	xcrun stapler staple "$app"
	# what Gatekeeper says of the app once downloaded
	spctl --assess --type execute --verbose=2 "$app"
fi
dmg=$dist/rbw-desktop-$version-macos-$arch.dmg
image
if ((notarize)); then
	notarize "$dmg"
	xcrun stapler staple "$dmg"
fi
rm -rf "${dist:?}/$app_name.app"
ditto "$app" "$dist/$app_name.app"
(cd "$dist" && shasum -a 256 "$(basename "$dmg")" >"$(basename "$dmg").sha256")
echo "$dist/$app_name.app"
echo "$dmg"
