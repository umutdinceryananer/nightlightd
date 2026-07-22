#!/usr/bin/env bash
# Capture the TUI's tabs as README screenshots — one maximised terminal per
# shot, cropped to drop the window chrome, written to docs/screenshots/.
#
# Needs: xfce4-terminal, xfce4-screenshooter, wmctrl, python3 (Pillow).
# Run it from anywhere; it locates the repo from its own path. You must be at
# the machine (the X server blanks the screen when idle, which blacks shots).
#
# Usage:
#   scripts/screenshots.sh            # all five README shots
#   scripts/screenshots.sh now        # just the "now" tab (live theme)
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

mkdir -p "$out"
case "${1:-all}" in
    all)
        shot live      now      01-now.png
        shot live      today    02-today.png
        shot live      location 03-location.png
        shot live      settings 04-settings.png
        shot synthwave now      05-now-synthwave.png
        ;;
    now)      shot live      now      01-now.png ;;
    today)    shot live      today    02-today.png ;;
    location) shot live      location 03-location.png ;;
    settings) shot live      settings 04-settings.png ;;
    synthwave) shot synthwave now     05-now-synthwave.png ;;
    *) echo "usage: $0 [all|now|today|location|settings|synthwave]" >&2; exit 2 ;;
esac

echo "done → $out"
