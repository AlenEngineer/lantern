#!/usr/bin/env bash
# Called by the app for `shot NAME`: moves each window onto its headless
# output if it isn't there yet, then captures it. Set up by scripts/e2e.sh.
# The stage window is closed while planning: then there is no stage shot,
# only the dashboard's, and windows.log says so.
#
# Windows are found by app id *and* the app's process (this script's
# parent), so a Lantern the user has open is never moved or captured.
set -euo pipefail
name=$1
app=$PPID

# Every window as one JSON object a line, the same shape on either
# compositor: {class, title, pid, id, output, fullscreen, at: [x, y], size: [w, h]}.
if [[ ${E2E_COMPOSITOR:-hyprland} == sway ]]; then
    windows() {
        swaymsg -t get_tree | jq -c '.nodes[] | select(.type == "output") | .name as $output
            | .. | objects | select(.pid? and (.type == "con" or .type == "floating_con"))
            | {class: (.app_id // .window_properties.class), title: .name, pid, id, output: $output,
               fullscreen: (.fullscreen_mode != 0), at: [.rect.x, .rect.y], size: [.rect.width, .rect.height]}'
    }
    move_to() { swaymsg "[con_id=$1] move container to output $2, focus" >/dev/null; }
else
    windows() {
        local names
        names=$(hyprctl monitors -j | jq -c 'map({key: (.id | tostring), value: .name}) | from_entries')
        hyprctl clients -j | jq -c --argjson names "$names" '.[]
            | {class, title, pid, id: .address, output: $names[.monitor | tostring],
               fullscreen: (.fullscreen != 0), at, size}'
    }
    move_to() {
        local workspace
        workspace=$(hyprctl monitors -j | jq --arg name "$2" '.[] | select(.name == $name) | .activeWorkspace.id')
        hyprctl dispatch "hl.dsp.window.move({ workspace = $workspace, window = \"address:$1\" })" >/dev/null
    }
fi

client() {
    windows | jq -c --arg class "$1" --argjson pid "$app" 'select(.class == $class and .pid == $pid)' | head -n 1
}

place() {
    local class=$1 output=$2 tries=${3:-300} window
    # A debug build takes several seconds to map its first window.
    for _ in $(seq "$tries"); do
        window=$(client "$class")
        [[ -n $window ]] && break
        sleep 0.1
    done
    if [[ -z $window ]]; then
        echo "no window with app id $class" >&2
        return 1
    fi
    if [[ $(jq -r .output <<<"$window") != "$output" ]]; then
        move_to "$(jq -r .id <<<"$window")" "$output"
        moved=1
    fi
}

capture() {
    local class=$1 file=$2 geometry
    geometry=$(client "$class" | jq -r '"\(.at[0]),\(.at[1]) \(.size[0])x\(.size[1])"')
    grim -g "$geometry" "$file"
}

moved=0
place lantern-dashboard "$E2E_DASHBOARD_OUTPUT"
# The dashboard is up, so the app is: a stage not there within two seconds
# is closed (planning), not slow.
stage=1
place lantern-stage "$E2E_STAGE_OUTPUT" 20 2>/dev/null || stage=0
# Let the windows settle after a move and the stage redraw after the last command.
if (( moved )); then sleep 1; else sleep 0.3; fi
if (( stage )); then
    capture lantern-stage "$E2E_OUT/$name-stage.png"
else
    echo "$name: no stage window (closed while planning)" >>"$E2E_OUT/windows.log"
fi
capture lantern-dashboard "$E2E_OUT/$name-dashboard.png"
# The picker (lantern-web --pick, started by the app) gets a shot of its own.
picker=$(windows | jq -c 'select(.title == "YouTube · lantern")' |
    while read -r client; do
        pid=$(jq .pid <<<"$client")
        [[ $(ps -o ppid= -p "$pid" | tr -d ' ') == "$app" ]] && echo "$client"
    done | head -n 1)
if [[ -n $picker ]]; then
    grim -g "$(jq -r '"\(.at[0]),\(.at[1]) \(.size[0])x\(.size[1])"' <<<"$picker")" "$E2E_OUT/$name-picker.png"
fi
windows | jq -c --argjson pid "$app" \
    'select(.pid == $pid) | {class, output, fullscreen, at, size}' >>"$E2E_OUT/windows.log"
