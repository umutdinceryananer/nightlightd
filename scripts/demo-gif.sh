#!/usr/bin/env bash
# Record the README's demo reels straight over the committed GIFs: each
# interface opened in --demo and captured for exactly one compressed day, so
# the recording loops seamlessly wherever it starts. The durations must match
# DEMO_DAY_SECONDS in tui/src/main.rs and panel/src/main.rs, or the loop
# seams. They are the two constants below.
#
# Needs: xfce4-terminal, byzanz-record, wmctrl, xwininfo.
# You must be at the machine, and keep hands off until it prints done.
#
# Usage:
#   scripts/demo-gif.sh          # both reels
#   scripts/demo-gif.sh tui
#   scripts/demo-gif.sh panel
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$repo/docs/screenshots"

tui_day=34
panel_day=30

for tool in xfce4-terminal byzanz-record wmctrl xwininfo; do
    command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }
done

# record <window-title> <seconds> <trim-right> <output>
#
# byzanz captures a screen region rather than a window, so whatever is on top
# is what gets recorded: the window is raised and pinned above everything for
# the length of the take.
record() {
    local title="$1" secs="$2" trim="$3" file="$4"
    # -F, and not optional: without it wmctrl matches any window whose title
    # merely contains the word, and an editor with this repository open is
    # called "… - nightlightd - Visual Studio Code".
    wmctrl -F -a "$title"; sleep 1
    wmctrl -F -r "$title" -b add,above

    local info x y w h
    info="$(xwininfo -name "$title")"
    x=$(awk '/Absolute upper-left X/{print $NF}' <<<"$info")
    y=$(awk '/Absolute upper-left Y/{print $NF}' <<<"$info")
    w=$(awk '/^ *Width:/{print $NF}' <<<"$info")
    h=$(awk '/^ *Height:/{print $NF}' <<<"$info")

    echo "recording ${secs} s at $((w - trim))x${h}+${x}+${y} — hands off…"
    byzanz-record --duration="$secs" --x="$x" --y="$y" \
        --width=$((w - trim)) --height="$h" "$out/$file"
    echo "  → $out/$file"
}

record_tui() {
    local title="nl-gif-$$"
    cargo build --release -p nightlight-tui --manifest-path "$repo/Cargo.toml" -q
    xfce4-terminal --disable-server --hide-menubar --hide-toolbar \
        --geometry=90x30 --title="$title" \
        --command="$repo/target/release/nightlight-tui --demo" &
    sleep 3
    # The client area's absolute position already excludes the decorations,
    # so only the scrollbar on the right needs trimming.
    record "$title" "$tui_day" 14 nightlight-tui.gif
    wmctrl -F -c "$title"
}

record_panel() {
    cargo build --release -p nightlight-panel --manifest-path "$repo/Cargo.toml" -q
    # Single instance: an open panel would swallow the launch and leave the
    # old window — without the demo clock — in front of the camera.
    pkill -f 'nightlight-panel' 2>/dev/null || true
    sleep 1
    "$repo/target/release/nightlight-panel" --demo &
    sleep 3
    record nightlightd "$panel_day" 0 nightlight-panel.gif
    pkill -f 'nightlight-panel' 2>/dev/null || true
}

case "${1:-all}" in
    all)   record_tui; sleep 2; record_panel ;;
    tui)   record_tui ;;
    panel) record_panel ;;
    *) echo "usage: $0 [all|tui|panel]" >&2; exit 2 ;;
esac

echo "done → $out"
