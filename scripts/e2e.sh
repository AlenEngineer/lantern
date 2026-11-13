#!/usr/bin/env bash
# Runs the real app on a script and screenshots both windows at each `shot NAME`.
#
#   scripts/e2e.sh <script> <outdir>
#
# Each window gets its own headless Hyprland output, so the fullscreen stage
# can't hide the dashboard and a locked or sleeping laptop screen doesn't
# matter. Writes <outdir>/NAME-stage.png, <outdir>/NAME-dashboard.png and
# <outdir>/app.log. The app reads scripts/e2e/config.toml unless
# LANTERN_CONFIG names another config. Plans are saved in a fresh
# <outdir>/data, or in E2E_DATA to share them between runs:
#
#   scripts/e2e.sh scripts/e2e/plans.script ~/.cache/lantern/e2e/plans
#   E2E_DATA=~/.cache/lantern/e2e/plans/data \
#       scripts/e2e.sh scripts/e2e/reopen.script ~/.cache/lantern/e2e/reopen
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
# E2E_RELEASE=1 runs an optimized build, to check timings as users see them.
if [[ -n ${E2E_RELEASE:-} ]]; then
    profile=release
    cargo build --release --manifest-path "$repo/Cargo.toml" -p lantern-app -j 4
else
    profile=debug
    cargo build --manifest-path "$repo/Cargo.toml" -p lantern-app -j 4
fi

rm -f "$out"/*.png "$out/app.log" "$out/windows.log"
if [[ -n ${E2E_DATA:-} ]]; then
    data=$(mkdir -p "$E2E_DATA" && realpath "$E2E_DATA")
else
    data=$out/data
    rm -rf "$data"
fi

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
# The dashboard gets a laptop-sized 1280x720 logical screen. A bigger
# E2E_DASHBOARD_SCALE makes it smaller, to check narrow layouts.
hyprctl eval "hl.monitor({ output = \"$E2E_DASHBOARD_OUTPUT\", mode = \"1920x1080\", position = \"auto\", scale = ${E2E_DASHBOARD_SCALE:-1.5} })" >/dev/null

LANTERN_CONFIG=${LANTERN_CONFIG:-$repo/scripts/e2e/config.toml} \
    LANTERN_SCRIPT=$script LANTERN_SHOT="$repo/scripts/e2e/shot.sh" XDG_DATA_HOME=$data \
    timeout 600 "$repo/target/$profile/lantern" 2>"$out/app.log" &
app=$!
status=0
wait "$app" || status=$?
app=
echo "app exited with status $status; log and shots in $out"
ls "$out"
exit "$status"
