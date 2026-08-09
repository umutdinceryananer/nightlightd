#!/usr/bin/env bash
# Capture the README's screenshots — the dashboard's tabs in a terminal, and
# the panel's in a window — cropped to drop the chrome, into docs/screenshots/.
#
# Needs: xfce4-terminal, xfce4-screenshooter, wmctrl, python3 (Pillow).
# Run it from anywhere; it locates the repo from its own path. You must be at
# the machine (the X server blanks the screen when idle, which blacks shots).
#
# The panel shots want a daemon running: the window draws what the daemon
# says, and against a stopped one every tab is a notice instead of a picture.
#
# Usage:
#   scripts/screenshots.sh            # every shot, dashboard and panel
#   scripts/screenshots.sh now        # just the dashboard's "now" tab
#   scripts/screenshots.sh panel      # just the panel's four
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$repo/docs/screenshots"
bin="$repo/target/release/nightlight-tui"
title="nl-shot-$$"           # unique title so wmctrl targets our window only

for tool in xfce4-terminal xfce4-screenshooter wmctrl python3; do
    command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }
done

echo "building release binary…"
cargo build --release -p nightlight-tui --manifest-path "$repo/Cargo.toml" -q

# A compact window, not a maximised one: README shots should read like a
# terminal — small, dense, characters with presence — not like a fullscreen
# app floating in empty space. 90x30 lands near 1000px wide, which is the
# sweet spot for GitHub and the ratatui showcase (1200px is the hard max
# before compression makes it mush).
geometry="${GEOMETRY:-90x30}"

# shot <theme> <tab> <output-file>
shot() {
    local theme="$1" tab="$2" file="$3"
    echo "  $file  (theme $theme, tab $tab, $geometry)"
    xfce4-terminal --disable-server --hide-menubar --hide-toolbar \
        --geometry="$geometry" \
        --title="$title" --command="$bin --theme $theme --tab $tab" &
    sleep 4
    wmctrl -a "$title"; sleep 1
    xfce4-screenshooter -w -s "$out/$file.raw.png" >/dev/null 2>&1
    sleep 1
    wmctrl -c "$title"; sleep 1

    # Crop the title bar (first near-black row down a chrome-free column) and
    # trim the right scrollbar + window borders.
    python3 - "$out/$file.raw.png" "$out/$file" <<'PY'
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
w, h = im.size
px = im.load()
top = next((y for y in range(8, 60) if sum(px[120, y]) < 100), 24)
im.crop((3, top, w - 16, h - 2)).save(sys.argv[2])
PY
    rm -f "$out/$file.raw.png"
}

# panel_shot <theme> <tab> <output-file>
#
# The window instead of the terminal. Three differences from `shot` above:
# the panel is single-instance, so any open one must go first or the launch
# only raises it; it takes its theme from a file rather than a flag, because
# a window remembers what it wore; and it has no scrollbar to trim, so the
# crop takes the decoration and nothing else.
panel_shot() {
    local theme="$1" tab="$2" file="$3"
    echo "  $file  (theme $theme, tab $tab)"
    pkill -f 'nightlight-panel' 2>/dev/null || true
    sleep 1
    echo "$theme" > "$HOME/.config/nightlightd/panel-theme"
    "$panel" --tab "$tab" &
    sleep 4
    # -F, and not optional: without it wmctrl matches any window whose title
    # merely *contains* the word, and an editor with this repository open is
    # called "… - nightlightd - Visual Studio Code". The first run of this
    # function photographed the editor four times.
    wmctrl -F -a nightlightd; sleep 1
    xfce4-screenshooter -w -s "$out/$file.raw.png" >/dev/null 2>&1
    sleep 1
    pkill -f 'nightlight-panel' 2>/dev/null || true
    sleep 1

    python3 - "$out/$file.raw.png" "$out/$file" <<'PY'
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
w, h = im.size
px = im.load()
# The title bar ends at the first row that is the window's own dark ground
# all the way across the middle. Searched rather than assumed: decoration
# height is the window manager's business and changes with the theme.
top = next((y for y in range(4, 80) if sum(px[w // 2, y]) < 120), 28)
im.crop((1, top, w - 1, h - 1)).save(sys.argv[2])
PY
    rm -f "$out/$file.raw.png"
}

mkdir -p "$out"
panel="$repo/target/release/nightlight-panel"
# The panel's own remembered theme, put back whatever happens: the showcase
# borrows it for a minute and does not get to keep it.
theme_file="$HOME/.config/nightlightd/panel-theme"
saved_theme="$(cat "$theme_file" 2>/dev/null || true)"
restore_theme() {
    if [ -n "$saved_theme" ]; then printf '%s' "$saved_theme" > "$theme_file"
    else rm -f "$theme_file"; fi
}
trap restore_theme EXIT

case "${1:-all}" in
    all)
        cargo build --release -p nightlight-panel --manifest-path "$repo/Cargo.toml" -q
        shot live      now      01-now.png
        shot live      today    02-today.png
        shot live      location 03-location.png
        shot live      outputs  04-outputs.png
        shot live      settings 05-settings.png
        shot synth     now      06-now-synthwave.png
        panel_shot live  now      panel-now.png
        panel_shot live  today    panel-today.png
        panel_shot live  location panel-location.png
        panel_shot nord  settings panel-settings.png
        ;;
    now)       shot live  now      01-now.png ;;
    today)     shot live  today    02-today.png ;;
    location)  shot live  location 03-location.png ;;
    outputs)   shot live  outputs  04-outputs.png ;;
    settings)  shot live  settings 05-settings.png ;;
    synthwave) shot synth now      06-now-synthwave.png ;;
    panel)
        cargo build --release -p nightlight-panel --manifest-path "$repo/Cargo.toml" -q
        panel_shot live  now      panel-now.png
        panel_shot live  today    panel-today.png
        panel_shot live  location panel-location.png
        panel_shot nord  settings panel-settings.png
        ;;
    *)
        echo "usage: $0 [all|now|today|location|outputs|settings|synthwave|panel]" >&2
        exit 2
        ;;
esac

echo "done → $out"
