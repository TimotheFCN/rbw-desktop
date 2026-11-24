# shellcheck shell=bash
# What the Linux and macOS packaging scripts share: the app's version and
# id, the rbw and rbw-agent built from an rbw-ng checkout, and the licence
# files every package carries. Source it from bash with $root set to the
# repository and $RBW_NG to the rbw-ng checkout, which is only read.
# shellcheck disable=SC2154 # root comes from the script that sources this

# A string constant of the Rust code.
rust_const() { sed -n "s/^pub const $1: &str = \"\(.*\)\";\$/\1/p" "$root/$2"; }
app_id=$(rust_const APP_ID crates/core/src/lib.rs)
app_name=$(rust_const APP_NAME crates/core/src/lib.rs)
required_rbw=$(rust_const REQUIRED_RBW crates/core/src/rbw.rs)
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)
[[ -n $app_id && -n $app_name && -n $required_rbw && -n $version ]] || {
	echo "can't read the app's id, name, version or REQUIRED_RBW" >&2
	exit 1
}
: "${RBW_NG:?set RBW_NG to an rbw-ng checkout}"
RBW_NG=$(cd "$RBW_NG" && pwd)

# The rbw-ng revision the release workflow bundles, and the one at hand.
pinned_rbw_ng=$(<"$root/packaging/rbw-ng-revision")
rbw_ng_revision=$(git -C "$RBW_NG" rev-parse HEAD 2>/dev/null || echo unknown)
if [[ $rbw_ng_revision != "$pinned_rbw_ng" ]]; then
	echo "note: $RBW_NG is at $rbw_ng_revision, not the pinned $pinned_rbw_ng" >&2
fi

# Builds rbw and rbw-agent for a target, stripped, into rbw_ng_dir.
rbw_ng_build() {
	CARGO_TARGET_DIR=$root/target/package/rbw-ng cargo build --quiet --locked --release --bins \
		--config 'profile.release.strip = true' --target "$1" --manifest-path "$RBW_NG/Cargo.toml"
}
rbw_ng_dir() { echo "$root/target/package/rbw-ng/$1/release"; }

# Fails unless the rbw in this folder is rbw-ng REQUIRED_RBW or later, for
# the binaries this machine can run.
rbw_ng_check() {
	local found
	found=$("$1/rbw" --version)
	if [[ $found != "rbw-ng "* ]] ||
		[[ $(printf '%s\n' "$required_rbw" "${found#rbw-ng }" | sort -V | head -1) != "$required_rbw" ]]; then
		echo "$RBW_NG builds $found; the app needs rbw-ng $required_rbw or later" >&2
		exit 1
	fi
}

# Writes into a folder the licences: the app's, rbw-ng's, the bundled Inter
# font's, those of the Rust crates built into each for these targets, which
# cargo-about lists, and a copyright file that says which is which.
licenses() {
	local out=$1 about=$root/packaging/licenses
	shift
	command -v cargo-about >/dev/null || {
		echo "the licences need cargo-about: cargo install --locked --features cli cargo-about" >&2
		exit 1
	}
	mkdir -p "$out"
	install -m 644 "$root/LICENSE" "$out/LICENSE"
	install -m 644 "$RBW_NG/LICENSE" "$out/LICENSE.rbw-ng"
	install -m 644 "$root/crates/ui/assets/fonts/OFL.txt" "$out/LICENSE.Inter"
	local manifest listing target targets=()
	for target in "$@"; do targets+=(--target "$target"); done
	for manifest in "$root/Cargo.toml" "$RBW_NG/Cargo.toml"; do
		listing=third-party-licenses.txt
		[[ $manifest == "$RBW_NG"/* ]] && listing=third-party-licenses.rbw-ng.txt
		# cargo-about reads the crates of every platform, which building
		# for one doesn't download
		cargo fetch --quiet --locked --manifest-path "$manifest"
		cargo about -L error generate --frozen --config "$about/about.toml" "${targets[@]}" \
			--manifest-path "$manifest" --output-file "$out/$listing" "$about/about.hbs"
	done
	cat >"$out/copyright" <<EOF
$app_name $version
https://github.com/TimotheFCN/rbw-desktop

$app_name: MIT, in LICENSE.

It bundles rbw and rbw-agent from rbw-ng, built from revision
$rbw_ng_revision of https://github.com/TimotheFCN/rbw-ng: MIT, in
LICENSE.rbw-ng.

It bundles the Inter font: SIL Open Font License 1.1, in LICENSE.Inter.

The Rust crates built into the app and into rbw-ng, with their licences, are
listed in third-party-licenses.txt and third-party-licenses.rbw-ng.txt.
EOF
}
