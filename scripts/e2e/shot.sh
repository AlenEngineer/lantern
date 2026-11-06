#!/usr/bin/env bash
# Called by the app for `shot NAME`: moves each window onto its headless
# output if it isn't there yet, then captures it. Set up by scripts/e2e.sh.
set -euo pipefail
name=$1

client() { hyprctl clients -j | jq -c --arg class "$1" 'first(.[] | select(.class == $class)) // empty'; }
monitor_id() { hyprctl monitors -j | jq --arg name "$1" '.[] | select(.name == $name) | .id'; }
workspace_of() { hyprctl monitors -j | jq --arg name "$1" '.[] | select(.name == $name) | .activeWorkspace.id'; }

place() {
    local class=$1 output=$2 window
    # A debug build takes several seconds to map its first window.
    for _ in $(seq 300); do
        window=$(client "$class")
        [[ -n $window ]] && break
        sleep 0.1
    done
    if [[ -z $window ]]; then
        echo "no window with app id $class" >&2
        return 1
    fi
    if [[ $(jq .monitor <<<"$window") != "$(monitor_id "$output")" ]]; then
        hyprctl dispatch "hl.dsp.window.move({ workspace = $(workspace_of "$output"), window = \"class:$class\" })" >/dev/null
        moved=1
    fi
}

capture() {
    local class=$1 file=$2 geometry
    geometry=$(client "$class" | jq -r '"\(.at[0]),\(.at[1]) \(.size[0])x\(.size[1])"')
    grim -g "$geometry" "$file"
}

moved=0
place lantern-stage "$E2E_STAGE_OUTPUT"
place lantern-dashboard "$E2E_DASHBOARD_OUTPUT"
# Let the windows settle after a move and the stage redraw after the last command.
if (( moved )); then sleep 1; else sleep 0.3; fi
capture lantern-stage "$E2E_OUT/$name-stage.png"
capture lantern-dashboard "$E2E_OUT/$name-dashboard.png"
hyprctl clients -j | jq -c '.[] | select(.class | startswith("lantern")) | {class, monitor, fullscreen, at, size}' >>"$E2E_OUT/windows.log"
