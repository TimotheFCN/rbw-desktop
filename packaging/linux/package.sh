#!/usr/bin/env bash
# Builds the Linux packages of this machine's architecture into
# target/packages:
#
# - rbw-desktop-VERSION-ARCH-linux.tar.xz, laid out as an install prefix
#   (bin, libexec, share), which runs from wherever it is unpacked, such as
#   ~/.local;
# - rbw-desktop_VERSION_DEBARCH.deb, the same files under /usr.
#
# Both bundle rbw and rbw-agent, built from the rbw-ng checkout in $RBW_NG,
# in libexec/rbw-desktop: off PATH, where the app finds them. The binaries
# link only to what every desktop has, and to the glibc of the machine that
# builds them or a later one. See docs/DEVELOPMENT.md.
#
#     RBW_NG=path/to/rbw-ng packaging/linux/package.sh [--no-build]
set -euo pipefail

build=1
case ${1:-} in
"") ;;
--no-build) build=0 ;;
*) echo "usage: RBW_NG=path/to/rbw-ng packaging/linux/package.sh [--no-build]" >&2; exit 2 ;;
esac
[[ $(uname -s) == Linux ]] || { echo "packaging/linux/package.sh runs on Linux" >&2; exit 1; }

root=$(cd "$(dirname "$0")/../.." && pwd)
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-8}
# shellcheck source=packaging/lib.sh
source "$root/packaging/lib.sh"

arch=$(uname -m)
case $arch in
x86_64) deb_arch=amd64 ;;
aarch64) deb_arch=arm64 ;;
*) echo "no package for $arch" >&2; exit 1 ;;
esac
target=$arch-unknown-linux-gnu
if ((build)); then
	cargo build --quiet --locked --profile dist --manifest-path "$root/Cargo.toml" -p rbw-desktop
	rbw_ng_build "$target"
fi
rbw_ng=$(rbw_ng_dir "$target")
rbw_ng_check "$rbw_ng"

# File times from the last commit, for packages that are the same each time.
SOURCE_DATE_EPOCH=$(git -C "$root" log -1 --format=%ct)
export SOURCE_DATE_EPOCH
dist=$root/target/packages
work=$root/target/package/linux
name=rbw-desktop-$version-$arch-linux
rm -rf "$work" && mkdir -p "$work" "$dist"
rm -f "$dist"/rbw-desktop-*-"$arch"-linux.tar.* "$dist"/rbw-desktop_*_"$deb_arch".deb*

# The prefix: what both packages hold.
prefix=$work/$name
install -D -m 755 "$root/target/dist/rbw-desktop" "$prefix/bin/rbw-desktop"
install -D -m 755 -t "$prefix/libexec/rbw-desktop" "$rbw_ng/rbw" "$rbw_ng/rbw-agent"
install -D -m 644 -t "$prefix/share/applications" "$root/packaging/linux/$app_id.desktop"
install -D -m 644 -t "$prefix/share/metainfo" "$root/packaging/linux/$app_id.metainfo.xml"
for icon in "$root"/packaging/icon/hicolor/*/apps/"$app_id.png"; do
	size=$(basename "$(dirname "$(dirname "$icon")")")
	install -D -m 644 "$icon" "$prefix/share/icons/hicolor/$size/apps/$app_id.png"
done
install -D -m 644 "$root/packaging/icon/icon.svg" "$prefix/share/icons/hicolor/scalable/apps/$app_id.svg"
install -D -m 644 "$root/README.md" "$prefix/share/doc/rbw-desktop/README.md"
licenses "$prefix/share/doc/rbw-desktop" "$target"
chmod -R u=rwX,go=rX "$prefix"

# The same tree, byte for byte, whatever the machine's user and umask.
tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner \
	-C "$work" -cf - "$name" | xz -9 -T0 --quiet >"$dist/$name.tar.xz"

# The .deb: the prefix under /usr, and a control file whose dependencies
# are the libraries the binaries link to, from dpkg-shlibdeps, and those
# that GPUI opens as it starts: Vulkan, or EGL without it, and Wayland's
# client library.
deb=$work/deb
mkdir -p "$deb/DEBIAN"
cp -a "$prefix" "$deb/usr"
mkdir -p "$work/shlibs/debian"
printf 'Source: rbw-desktop\n\nPackage: rbw-desktop\nArchitecture: any\n' >"$work/shlibs/debian/control"
shlibs=$(cd "$work/shlibs" && dpkg-shlibdeps -O -e"$deb/usr/bin/rbw-desktop" \
	-e"$deb/usr/libexec/rbw-desktop/rbw" -e"$deb/usr/libexec/rbw-desktop/rbw-agent" 2>/dev/null |
	sed -n 's/^shlibs:Depends=//p')
[[ -n $shlibs ]] || { echo "dpkg-shlibdeps found no libraries" >&2; exit 1; }
cat >"$deb/DEBIAN/control" <<EOF
Package: rbw-desktop
Version: $version
Architecture: $deb_arch
Maintainer: Timothe F <7504193+TimotheFCN@users.noreply.github.com>
Installed-Size: $(du -sk --apparent-size "$deb/usr" | cut -f1)
Depends: $shlibs, libvulkan1 | libegl1, libwayland-client0
Recommends: pinentry-gnome3 | pinentry
Section: utils
Priority: optional
Homepage: https://github.com/TimotheFCN/rbw-desktop
Description: unofficial desktop app for Bitwarden and Vaultwarden vaults
 $app_name finds, copies and saves the passwords of Bitwarden and
 Vaultwarden vaults, in a window that opens at once. It is built on rbw-ng,
 whose rbw and rbw-agent it bundles in /usr/libexec/rbw-desktop, off PATH.
EOF
(cd "$deb" && find usr -type f -print0 | sort -z | xargs -0 md5sum >DEBIAN/md5sums)
chmod -R u=rwX,go=rX "$deb"
deb_file=rbw-desktop_${version}_$deb_arch.deb
dpkg-deb --root-owner-group -Zxz -z9 --build "$deb" "$dist/$deb_file" >/dev/null

cd "$dist"
for file in "$name.tar.xz" "$deb_file"; do
	sha256sum "$file" >"$file.sha256"
	echo "$dist/$file"
done
