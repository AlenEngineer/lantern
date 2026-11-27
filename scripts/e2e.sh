#!/usr/bin/env bash
# Runs the real app on a script and screenshots both windows at each `shot NAME`.
#
#   scripts/e2e.sh <script> <outdir>
#
# Each window gets its own headless output, so the fullscreen stage can't
# hide the dashboard and a locked or sleeping laptop screen doesn't matter. Writes <outdir>/NAME-stage.png, <outdir>/NAME-dashboard.png and
# <outdir>/app.log. The app reads scripts/e2e/config.toml unless
# LANTERN_CONFIG names another config. It has a fresh home of its own,
# <outdir>/home, so what it saves or caches (an export, a download, a
# thumbnail) never lands in yours. Plans are saved in a fresh
# <outdir>/data, or in E2E_DATA to share them between runs:
#
#   scripts/e2e.sh scripts/e2e/plans.script ~/.cache/lantern/e2e/plans
#   E2E_DATA=~/.cache/lantern/e2e/plans/data \
#       scripts/e2e.sh scripts/e2e/reopen.script ~/.cache/lantern/e2e/reopen
#
# E2E_COMPOSITOR picks where the app runs:
#
# - sway (the default when sway is installed): a headless sway of its own,
#   with a stage output and a dashboard output. Nothing shows on your screen and your session isn't touched.
#   The app doesn't see Hyprland there, so it leaves placing its windows
#   to the shots. A virtual mouse (scripts/e2e/mouse.py) is plugged in for
#   the scripts' pointer commands, which only work here.
# - hyprland: two headless outputs added to your Hyprland
#   session and removed at the end. Other apps see the screens come and go,
#   and some don't take it well: Steam's window has crashed and reopened
#   during runs. The stage window can also show on your own screen for a
#   moment before it's moved to its headless one.
#
# Sound plays through your speakers on either.
#
# E2E_NO_BUILD=1 runs the binary already built, without building it first.
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: $0 <script> <outdir>" >&2
    exit 2
fi
script=$(realpath "$1")
out=$(mkdir -p "$2" && realpath "$2")
repo=$(cd "$(dirname "$0")/.." && pwd)

# E2E_RELEASE=1 runs an optimized build, to check timings as users see them.
profile=debug
[[ -n ${E2E_RELEASE:-} ]] && profile=release
if [[ -z ${E2E_NO_BUILD:-} ]]; then
    available_mb=$(free -m | awk '/^Mem:/ {print $7}')
    if (( available_mb < 3072 )); then
        echo "only ${available_mb} MB of memory available, need 3 GB to build" >&2
        exit 1
    fi
    release=()
    [[ $profile == release ]] && release=(--release)
    cargo build "${release[@]}" --manifest-path "$repo/Cargo.toml" -p lantern-app -p lantern-web -j 4
fi

rm -f "$out"/*.png "$out/app.log" "$out/windows.log"
if [[ -n ${E2E_DATA:-} ]]; then
    data=$(mkdir -p "$E2E_DATA" && realpath "$E2E_DATA")
else
    data=$out/data
    rm -rf "$data"
fi
# The folders a home usually has, so the path suggestions and an export
# find what they would. GStreamer's list of its plugins is copied from
# yours: making it again takes it ten seconds.
home=$out/home
rm -rf "$home"
mkdir -p "$home"/{Documents,Downloads,Music,Pictures,Videos} "$home/.cache/gstreamer-1.0"
for registry in "${XDG_CACHE_HOME:-$HOME/.cache}"/gstreamer-1.0/registry.*.bin; do
    if [[ -f $registry ]]; then cp "$registry" "$home/.cache/gstreamer-1.0/"; fi
done

# The dashboard gets a laptop-sized 1280x720 logical screen. A bigger
# E2E_DASHBOARD_SCALE makes it smaller, to check narrow layouts.
scale=${E2E_DASHBOARD_SCALE:-1.5}
default=hyprland
command -v sway >/dev/null && default=sway
export E2E_COMPOSITOR=${E2E_COMPOSITOR:-$default} E2E_OUT=$out
app=
sway=
mouse=
run=(env)
case $E2E_COMPOSITOR in
sway)
    export E2E_STAGE_OUTPUT=HEADLESS-1 E2E_DASHBOARD_OUTPUT=HEADLESS-2
    # shellcheck disable=SC2329  # called by the trap below
    cleanup() {
        [[ -n $app ]] && kill "$app" 2>/dev/null || true
        [[ -n $mouse ]] && kill "$mouse" 2>/dev/null || true
        [[ -n $sway ]] && kill "$sway" 2>/dev/null || true
    }
    trap cleanup EXIT
    cat >"$out/sway.conf" <<CONF
output HEADLESS-1 mode 1920x1080 position 0 0
output HEADLESS-2 mode 1920x1080 position 1920 0 scale $scale
workspace stage output HEADLESS-1
workspace dashboard output HEADLESS-2
default_border none
default_floating_border none
focus_follows_mouse no
mouse_warping none
xwayland disable
CONF
    runtime=${XDG_RUNTIME_DIR:-/run/user/$UID}
    # The Wayland sockets in $runtime, one name a line.
    wayland_sockets() {
        local f
        for f in "$runtime"/wayland-*; do
            if [[ ${f##*/} =~ ^wayland-[0-9]+$ ]]; then echo "${f##*/}"; fi
        done
    }
    before=$(wayland_sockets)
    env -u WAYLAND_DISPLAY -u DISPLAY -u HYPRLAND_INSTANCE_SIGNATURE -u SWAYSOCK \
        WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=2 WLR_LIBINPUT_NO_DEVICES=1 \
        sway -c "$out/sway.conf" >"$out/sway.log" 2>&1 &
    sway=$!
    export SWAYSOCK=$runtime/sway-ipc.$UID.$sway.sock
    display=
    for _ in $(seq 100); do
        display=$(wayland_sockets | grep -vxF "$before" | head -n 1 || true)
        [[ -S $SWAYSOCK && -n $display ]] && break
        sleep 0.1
    done
    if [[ ! -S $SWAYSOCK || -z $display ]]; then
        echo "sway didn't start; see $out/sway.log" >&2
        exit 1
    fi
    swaymsg 'workspace dashboard' >/dev/null
    # Headless, sway has no mouse: one is plugged in, which the scripts'
    # pointer commands move and press through its FIFO. It waits on the
    # stage's output, where its hover can't take a drop from the dashboard.
    rm -f "$out/mouse.fifo"
    mkfifo "$out/mouse.fifo"
    WAYLAND_DISPLAY=$display python3 "$repo/scripts/e2e/mouse.py" "$out/mouse.fifo" >"$out/mouse.log" 2>&1 &
    mouse=$!
    for _ in $(seq 50); do
        grep -q "plugged in" "$out/mouse.log" && break
        sleep 0.1
    done
    if ! grep -q "plugged in" "$out/mouse.log"; then
        echo "no mouse in sway; see $out/mouse.log" >&2
        exit 1
    fi
    layout=$(swaymsg -t get_tree | jq -r '"\(.rect.width) \(.rect.height)"')
    echo "move 1 1 $layout" >"$out/mouse.fifo"
    run=(env -u DISPLAY -u HYPRLAND_INSTANCE_SIGNATURE WAYLAND_DISPLAY="$display" E2E_MOUSE="$out/mouse.fifo")
    ;;
hyprland)
    export E2E_STAGE_OUTPUT=CQ-E2E-STAGE E2E_DASHBOARD_OUTPUT=CQ-E2E-DASH
    cleanup() {
        [[ -n $app ]] && kill "$app" 2>/dev/null || true
        hyprctl output remove "$E2E_STAGE_OUTPUT" >/dev/null 2>&1 || true
        hyprctl output remove "$E2E_DASHBOARD_OUTPUT" >/dev/null 2>&1 || true
    }
    trap cleanup EXIT
    cleanup
    hyprctl output create headless "$E2E_STAGE_OUTPUT" >/dev/null
    hyprctl output create headless "$E2E_DASHBOARD_OUTPUT" >/dev/null
    hyprctl eval "hl.monitor({ output = \"$E2E_DASHBOARD_OUTPUT\", mode = \"1920x1080\", position = \"auto\", scale = $scale })" >/dev/null
    ;;
*)
    echo "E2E_COMPOSITOR must be sway or hyprland, got $E2E_COMPOSITOR" >&2
    exit 2
    ;;
esac

"${run[@]}" LANTERN_CONFIG="${LANTERN_CONFIG:-$repo/scripts/e2e/config.toml}" \
    LANTERN_SCRIPT="$script" LANTERN_SHOT="$repo/scripts/e2e/shot.sh" XDG_DATA_HOME="$data" \
    HOME="$home" XDG_CONFIG_HOME="$home/.config" XDG_CACHE_HOME="$home/.cache" \
    XDG_STATE_HOME="$home/.local/state" \
    timeout 600 "$repo/target/$profile/lantern" 2>"$out/app.log" &
app=$!
status=0
wait "$app" || status=$?
app=
echo "app exited with status $status; log and shots in $out"
ls "$out"
exit "$status"
