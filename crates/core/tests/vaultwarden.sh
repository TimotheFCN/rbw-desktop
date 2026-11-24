#!/bin/sh
# Runs the real backend's tests against Vaultwarden in Docker, with rbw-ng
# built from a checkout:
#
#     RBW_NG=~/src/rbw-ng crates/core/tests/vaultwarden.sh
#
# It starts a throwaway server on 127.0.0.1, makes three accounts (one with
# two-step login, one with an API key), and items shared through an
# organization, with the helpers of rbw-ng's tools/scripting-check (which
# needs Python 3.10+ and the `cryptography` package). It runs the tests with
# a private HOME, so that no profile of yours is touched. The server and the
# HOME go away at the end.
set -eu

: "${RBW_NG:?set RBW_NG to an rbw-ng checkout}"
core=$(cd "$(dirname "$0")/.." && pwd)
workspace=$(cd "$core/../.." && pwd)
name="rbw-desktop-core-test-$$"
port=$((20000 + $$ % 20000))
# short, for the agents' sockets
home=$(mktemp -d /tmp/rbwdt.XXXXXX)

cleanup() {
    docker rm -f "$name" >/dev/null 2>&1 || true
    # agents of a test that stopped halfway
    for pidfile in "$home"/run/rbw*/pidfile; do
        if [ -f "$pidfile" ]; then kill "$(cat "$pidfile")" 2>/dev/null || true; fi
    done
    rm -rf "$home"
}
trap cleanup EXIT INT TERM

(cd "$RBW_NG" && cargo build --quiet --bins)

docker run -d --name "$name" -p "127.0.0.1:$port:80" \
    -e SIGNUPS_ALLOWED=true -e I_REALLY_WANT_VOLATILE_STORAGE=true \
    -e LOGIN_RATELIMIT_MAX_BURST=1000 -e LOGIN_RATELIMIT_SECONDS=1 \
    -e UNAUTHENTICATED_RATELIMIT_MAX_BURST=1000 \
    -e UNAUTHENTICATED_RATELIMIT_SECONDS=1 \
    vaultwarden/server:latest >/dev/null
server="http://127.0.0.1:$port"
tries=0
until curl -sf "$server/alive" >/dev/null; do
    tries=$((tries + 1))
    [ "$tries" -lt 60 ] || { echo "Vaultwarden didn't start" >&2; exit 1; }
    sleep 1
done

password="correct horse battery staple"
suffix=$(od -An -N4 -tx1 /dev/urandom | tr -d ' \n')
email="core-$suffix@example.com"
two_step_email="core-2fa-$suffix@example.com"
api_key_email="core-key-$suffix@example.com"
# the two-step account's authenticator secret, and the third account's API
# key, one per line
setup=$(PYTHONDONTWRITEBYTECODE=1 python3 - "$RBW_NG/tools/scripting-check" \
    "$server" "$email" "$two_step_email" "$api_key_email" "$password" <<'PY'
import importlib.machinery, importlib.util, sys
path, server, email, two_step_email, api_key_email, password = sys.argv[1:]
loader = importlib.machinery.SourceFileLoader("scripting_check", path)
check = importlib.util.module_from_spec(importlib.util.spec_from_loader("scripting_check", loader))
loader.exec_module(check)
public_key = check.register(server, email, password)
# a shared login, and one that asks for the master password again
check.share_login(server, email, password, public_key)
check.register(server, two_step_email, password)
print(check.enable_authenticator(server, two_step_email, password))
check.register(server, api_key_email, password)
print("\n".join(check.api_key(server, api_key_email, password)))
PY
)
line() { printf '%s\n' "$setup" | sed -n "$1p"; }

mkdir -m 700 "$home/run"
cd "$workspace"
# cargo's own folders stay this user's, under the HOME of before
# shellcheck disable=SC2097,SC2098
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$workspace/target/core}" \
    HOME="$home" XDG_RUNTIME_DIR="$home/run" \
    XDG_CONFIG_HOME="$home/.config" XDG_CACHE_HOME="$home/.cache" \
    XDG_DATA_HOME="$home/.local/share" \
    RBW_DESKTOP_RBW="$RBW_NG/target/debug/rbw" \
    RBW_DESKTOP_TEST_SERVER="$server" RBW_DESKTOP_TEST_EMAIL="$email" \
    RBW_DESKTOP_TEST_TWO_STEP_EMAIL="$two_step_email" \
    RBW_DESKTOP_TEST_TOTP_SECRET="$(line 1)" \
    RBW_DESKTOP_TEST_API_KEY_EMAIL="$api_key_email" \
    RBW_DESKTOP_TEST_CLIENT_ID="$(line 2)" \
    RBW_DESKTOP_TEST_CLIENT_SECRET="$(line 3)" \
    RBW_DESKTOP_TEST_PASSWORD="$password" \
    cargo test -p rbw-desktop-core --test vaultwarden -- --ignored
