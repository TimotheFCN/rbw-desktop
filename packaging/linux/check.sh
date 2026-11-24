#!/usr/bin/env bash
# Checks the Linux packages in target/packages as people would install
# them. It validates the desktop file and the AppStream metadata, then, in
# throwaway containers, installs the .deb with apt on Ubuntu 24.04 and
# Debian 13 and unpacks the tarball on Fedora 43, and in each checks that
# every library the binaries need is one a desktop has, that the bundled rbw
# runs and isn't on PATH, and that the app opens its window on a headless
# display with the demo vault. The containers download what they install.
# See docs/DEVELOPMENT.md.
#
#     packaging/linux/check.sh [FOLDER]    # default: target/packages
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
packages=$(realpath "${1:-$root/target/packages}")
arch=$(uname -m)
shopt -s nullglob
tarballs=("$packages"/rbw-desktop-*-"$arch"-linux.tar.xz)
debs=("$packages"/rbw-desktop_*.deb)
((${#tarballs[@]} == 1 && ${#debs[@]} == 1)) ||
	{ echo "build one tarball and one .deb first: packaging/linux/package.sh" >&2; exit 1; }
tarball=$(basename "${tarballs[0]}") deb=$(basename "${debs[0]}")

work=$(mktemp -d)
containers=()
cleanup() {
	for container in "${containers[@]}"; do docker rm -f "$container" >/dev/null 2>&1 || true; done
	rm -rf "$work"
}
trap cleanup EXIT

# The metadata, as software centres and app stores read it.
mkdir "$work/deb"
dpkg-deb -x "$packages/$deb" "$work/deb"
desktop-file-validate "$work"/deb/usr/share/applications/*.desktop
appstreamcli validate --no-net --pedantic --explain "$work"/deb/usr/share/metainfo/*.metainfo.xml
appstreamcli validate-tree --no-net "$work/deb"
echo "ok   desktop file and AppStream metadata"

# What runs in each container once the package is in: the binaries' and
# GPUI's libraries, rbw, and the window. $1 is where the package put the app.
cat >"$work/installed.sh" <<'EOF'
set -eu
bin=$1/bin/rbw-desktop libexec=$1/libexec/rbw-desktop
if ldd "$bin" "$libexec/rbw" "$libexec/rbw-agent" | grep 'not found'; then
	echo "FAIL libraries missing" >&2; exit 1
fi
for library in libvulkan.so.1 libwayland-client.so.0; do
	ldconfig -p | grep -q "$library" || { echo "FAIL no $library" >&2; exit 1; }
done
"$libexec/rbw" --version | grep -q '^rbw-ng ' || { echo "FAIL the bundled rbw doesn't run" >&2; exit 1; }
if command -v rbw >/dev/null; then echo "FAIL rbw is on PATH" >&2; exit 1; fi
export HOME=$(mktemp -d) RBW_DESKTOP_DEMO=unlocked
Xvfb :9 -screen 0 1600x1000x24 -nolisten tcp 2>/dev/null &
export DISPLAY=:9
sleep 1
"$bin" >"$HOME/app.log" 2>&1 &
if ! timeout 30 xdotool search --sync --onlyvisible --name '^Vault$' >/dev/null; then
	echo "FAIL no window; the app's log:" >&2; cat "$HOME/app.log" >&2; exit 1
fi
EOF

# Runs a check in a container of an image, after the commands that install.
in_container() {
	local name=$1 image=$2 install=$3 prefix=$4 container
	container=rbw-desktop-check-$$-$name
	containers+=("$container")
	if docker run --rm --name "$container" -v "$packages:/packages:ro" -v "$work:/check:ro" \
		-e DEBIAN_FRONTEND=noninteractive "$image" \
		sh -c "{ $install; } >/tmp/install.log 2>&1 || { cat /tmp/install.log; exit 1; }; sh /check/installed.sh $prefix"; then
		echo "ok   $name: installed, libraries found, rbw bundled, window shown"
	else
		echo "FAIL $name" >&2
		return 1
	fi
}

failed=0
for image in ubuntu:24.04 debian:13; do
	in_container "${image/:/-}" "$image" \
		"apt-get update && apt-get install -y --no-install-recommends /packages/$deb xvfb xdotool mesa-vulkan-drivers" \
		/usr || failed=1
done
# On Fedora, the tarball, and only the libraries a Fedora desktop has: what
# the app needs beyond them would fail ldd.
in_container fedora-43 fedora:43 \
	"dnf install -y --setopt=install_weak_deps=False tar xz xorg-x11-server-Xvfb xdotool \
		mesa-vulkan-drivers vulkan-loader libwayland-client libxkbcommon-x11 libxcb && tar -xJf /packages/$tarball -C /opt" \
	"/opt/${tarball%.tar.xz}" || failed=1
((failed == 0))
