#!/usr/bin/env bash
# Runs the real app on a script and screenshots both windows at each `shot NAME`.
#
#   scripts/e2e.sh <script> <outdir>
#
# Each window gets its own headless Hyprland output, so the fullscreen stage
# can't hide the dashboard and a locked or sleeping laptop screen doesn't
# matter. Writes <outdir>/NAME-stage.png, <outdir>/NAME-dashboard.png and
# <outdir>/app.log. The app reads scripts/e2e/config.toml unless
# CHORQUEST_CONFIG names another config.
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: $0 <script> <outdir>" >&2
    exit 2
fi
script=$(realpath "$1")
out=$(mkdir -p "$2" && realpath "$2")
repo=$(cd "$(dirname "$0")/.." && pwd)

available_mb=$(free -m | awk '/^Mem:/ {print $7}')
if (( available_mb < 3072 )); then
    echo "only ${available_mb} MB of memory available, need 3 GB to build" >&2
    exit 1
fi
cargo build --manifest-path "$repo/Cargo.toml" -p chorquest-app -j 4

rm -f "$out"/*.png "$out/app.log" "$out/windows.log"

export E2E_OUT=$out E2E_STAGE_OUTPUT=CQ-E2E-STAGE E2E_DASHBOARD_OUTPUT=CQ-E2E-DASH
app=
cleanup() {
    [[ -n $app ]] && kill "$app" 2>/dev/null || true
    hyprctl output remove "$E2E_STAGE_OUTPUT" >/dev/null 2>&1 || true
    hyprctl output remove "$E2E_DASHBOARD_OUTPUT" >/dev/null 2>&1 || true
}
trap cleanup EXIT
cleanup
hyprctl output create headless "$E2E_STAGE_OUTPUT" >/dev/null
hyprctl output create headless "$E2E_DASHBOARD_OUTPUT" >/dev/null

CHORQUEST_CONFIG=${CHORQUEST_CONFIG:-$repo/scripts/e2e/config.toml} \
    CHORQUEST_SCRIPT=$script CHORQUEST_SHOT="$repo/scripts/e2e/shot.sh" \
    timeout 600 "$repo/target/debug/chorquest" 2>"$out/app.log" &
app=$!
status=0
wait "$app" || status=$?
app=
echo "app exited with status $status; log and shots in $out"
ls "$out"
exit "$status"
