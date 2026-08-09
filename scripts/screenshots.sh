#!/usr/bin/env bash
# Capture the README's screenshots — the dashboard's tabs in a terminal, and
# the panel's in a window — cropped to drop the chrome, into docs/screenshots/.
#
# Needs: xfce4-terminal, xfce4-screenshooter, wmctrl, xdotool, python3 (Pillow).
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
#   scripts/screenshots.sh tray       # the tray menu — see #51, not reliable yet
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$repo/docs/screenshots"
bin="$repo/target/release/nightlight-tui"
title="nl-shot-$$"           # unique title so wmctrl targets our window only

for tool in xfce4-terminal xfce4-screenshooter wmctrl xdotool python3; do
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

# tray_icon
#
# Where the tray icon is, printed as "x y". Found by taking it away: one
# frame of the panel with the tray running and one without, and the single
# thing that differs between them is our own icon.
#
# The roundabout route is the only honest one. A StatusNotifierItem has no
# window — the icon is drawn inside the panel by its systray — so there is
# nothing for wmctrl to raise or xdotool to search for, and asking the item
# over D-Bus does not help either: `ContextMenu` tells the application, and
# the application is not what draws it. Hunting for the icon by clicking
# along the panel is worse still: the neighbours are somebody else's icons
# and a left click on one of those does whatever it does.
tray_icon() {
    xfce4-screenshooter -f -s "$out/.tray-with.png" >/dev/null 2>&1
    sleep 1
    pkill -x nightlight-tray
    sleep 3
    xfce4-screenshooter -f -s "$out/.tray-without.png" >/dev/null 2>&1
    sleep 1
    setsid -f nightlight-tray >/dev/null 2>&1 < /dev/null
    sleep 3
    python3 - "$out/.tray-with.png" "$out/.tray-without.png" "$panel_strip" <<'PY'
import sys
from PIL import Image, ImageChops
a = Image.open(sys.argv[1]).convert("RGB")
b = Image.open(sys.argv[2]).convert("RGB")
top = int(sys.argv[3])
strip = (0, top, a.size[0], a.size[1])
box = ImageChops.difference(a.crop(strip), b.crop(strip)).getbbox()
if box is None:
    sys.exit("the panel strip did not change when the tray went away")
left, up, right, down = box
print((left + right) // 2, (up + down) // 2 + top)
PY
    rm -f "$out/.tray-with.png" "$out/.tray-without.png"
}

# tray_shot <output-file>
#
# The icon found, right-clicked, and the menu cropped out of the frame.
# Right rather than left because `activate` toggles the filter — but that is
# not enough on its own: xfce4-panel's systray delivers Activate on the right
# button too, so the shot turns the filter off on its way past. Measured, not
# guessed; the first run left the screen neutral. Hence the state is read
# before and put back after.
tray_shot() {
    local file="$1"
    # -x, matching the process name exactly, because `pgrep -f` reads whole
    # command lines and finds the shell running this script — the script's own
    # text contains the name, so that check could never fail.
    #
    # It works by one character. Linux truncates a process name to 15 and
    # `pgrep -x` compares against the truncation, so any name of 16 or more
    # matches nothing, silently: `pgrep -x nightlight-panel` is always false.
    # "nightlight-tray" is exactly 15. Rename the binary and this line stops
    # working without saying so.
    pgrep -x nightlight-tray >/dev/null || {
        echo "  the tray is not running: nightlight-tray &" >&2
        return 1
    }
    echo "  finding the icon…"
    local xy
    xy="$(tray_icon)" || return 1
    echo "  icon at $xy"

    local was_on=no
    nightlightd --status 2>/dev/null | head -1 | grep -q ': on,' && was_on=yes

    xfce4-screenshooter -f -s "$out/$file.shut.png" >/dev/null 2>&1
    sleep 1
    # shellcheck disable=SC2086
    xdotool mousemove $xy click 3
    # The click both opens the menu and toggles the filter, so the menu it
    # opens is a picture of the wrong state — the readout says "off" and the
    # switch offers to turn it on. Putting the state back *while the menu is
    # up* fixes both at once: dbusmenu updates live, so a second later the
    # open menu is showing the truth again, and the net effect of taking this
    # photograph is nothing.
    sleep 1
    [ "$was_on" = yes ] && nightlightd --on >/dev/null 2>&1 || true
    # The pointer is still over the icon, and the icon has a tooltip, which
    # duly appears on top of the menu we are trying to photograph. Parked far
    # away — a jump, not a drag, so it crosses nothing and highlights nothing
    # — and given a moment for the balloon to go.
    xdotool mousemove 5 5
    sleep 3
    xfce4-screenshooter -f -s "$out/$file.open.png" >/dev/null 2>&1
    xdotool key Escape
    sleep 1
    # The crop may miss; the frames are kept until it does not, so a retry
    # costs a rerun of the maths rather than another click.
    set +e

    python3 - "$out/$file.shut.png" "$out/$file.open.png" "$out/$file" <<'PY'
import sys
from PIL import Image, ImageChops

shut = Image.open(sys.argv[1]).convert("RGB")
opened = Image.open(sys.argv[2]).convert("RGB")
mask = ImageChops.difference(shut, opened).convert("L").point(lambda v: 1 if v > 24 else 0)
w, h = mask.size
px = mask.load()

# Not the bounding box of everything that changed: between two full-screen
# frames a second apart, the panel clock ticks, a caret blinks and whatever
# is behind repaints, so that box comes out as half the screen. The menu is
# the one *dense* block of change, so it is found by profile — the longest
# run of rows that are broadly changed, then the same across the columns
# inside it.
# Grown from the densest line outwards rather than taken as the longest run
# of lines above a threshold. A menu has quiet rows inside it — a separator,
# a band of background that happens to match what was behind — and a run
# breaks on the first of them, which is how the first working version of
# this cropped away the readout line and kept only the bottom four items.
def spread(counts):
    peak = max(range(len(counts)), key=lambda i: counts[i])
    floor = max(counts[peak] * 0.15, 6)
    lo = hi = peak
    while lo > 0 and counts[lo - 1] >= floor:
        lo -= 1
    while hi < len(counts) - 1 and counts[hi + 1] >= floor:
        hi += 1
    return lo, hi + 1

STEP = 2  # every other pixel; a menu is hundreds wide, this cannot miss it
rows = [sum(px[x, y] for x in range(0, w, STEP)) for y in range(h)]
top, bottom = spread(rows)
if bottom - top < 60:
    sys.exit(f"no block taller than {bottom - top}px changed — was the menu open?")

cols = [sum(px[x, y] for y in range(top, bottom)) for x in range(w)]
left, right = spread(cols)
if right - left < 80:
    sys.exit(f"the block is only {right - left}px wide — that is not a menu")

pad = 2
opened.crop(
    (max(left - pad, 0), max(top - pad, 0), min(right + pad, w), min(bottom + pad, h))
).save(sys.argv[3])
print(f"  menu found: {right - left}x{bottom - top}")
PY
    local cropped=$?
    set -e
    if [ $cropped -ne 0 ]; then
        echo "  the two frames are kept at $out/$file.{shut,open}.png" >&2
        return 1
    fi
    rm -f "$out/$file.shut.png" "$out/$file.open.png"
}

mkdir -p "$out"
panel="$repo/target/release/nightlight-panel"
# Where the desktop panel starts, so the icon hunt looks only at the bar.
# Read rather than assumed: the bar can be at the top, and its height is the
# user's business.
panel_strip="$(wmctrl -lG | awk '/xfce4-panel/{print $4; exit}')"
panel_strip="${panel_strip:-0}"
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
    tray) tray_shot tray-menu.png ;;
    *)
        echo "usage: $0 [all|now|today|location|outputs|settings|synthwave|panel|tray]" >&2
        exit 2
        ;;
esac

echo "done → $out"
