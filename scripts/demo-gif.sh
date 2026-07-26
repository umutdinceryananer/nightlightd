#!/usr/bin/env bash
# Record the README's demo reel: open the TUI in --demo and capture exactly
# one compressed day (28 s) with byzanz, straight over the committed GIF.
# One day is one scripted lap, so the recording loops seamlessly wherever
# it starts.
#
# Needs: xfce4-terminal, byzanz-record, wmctrl, xwininfo.
# You must be at the machine, and keep hands off until it prints done.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$repo/docs/screenshots/nightlight-tui.gif"
bin="$repo/target/release/nightlight-tui"
title="nl-gif-$$"

for tool in xfce4-terminal byzanz-record wmctrl xwininfo; do
    command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }
done

echo "building release binary…"
cargo build --release -p nightlight-tui --manifest-path "$repo/Cargo.toml" -q

xfce4-terminal --disable-server --hide-menubar --hide-toolbar \
    --geometry=90x30 --title="$title" --command="$bin --demo" &
sleep 3
wmctrl -a "$title"; sleep 1
# Keep the window above everything for the whole take: byzanz records the
# screen region, so anything that covers the window would end up in the reel.
wmctrl -r "$title" -b add,above

# The client area's absolute position excludes the window decorations, so
# only the scrollbar on the right needs trimming, as the screenshots do.
info="$(xwininfo -name "$title")"
x=$(awk '/Absolute upper-left X/{print $NF}' <<<"$info")
y=$(awk '/Absolute upper-left Y/{print $NF}' <<<"$info")
w=$(awk '/^ *Width:/{print $NF}' <<<"$info")
h=$(awk '/^ *Height:/{print $NF}' <<<"$info")

echo "recording 28 s at $((w - 14))x${h}+${x}+${y} — hands off…"
byzanz-record --duration=28 --x="$x" --y="$y" \
    --width=$((w - 14)) --height="$h" "$out"

wmctrl -c "$title"
echo "done → $out"
