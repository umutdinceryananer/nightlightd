#!/usr/bin/env bash
# Photograph the tray's menu (#51), inside a desktop built to be thrown away.
#
# Two earlier attempts did this against the live session and both did damage.
# The first found the icon by diffing frames and right-clicked it — and
# xfce4-panel's systray delivers Activate on the right button as well as the
# left, so every attempt toggled the real filter; four runs, four flips of
# somebody's screen. The second tried a nested X server but leaked the real
# session bus into it, so the throwaway xfce4-panel opened the *real*
# ~/.config panel configuration and cleared 23 values out of it.
#
# So the isolation is the point of this script, and it is built from four
# things rather than one:
#
#   a nested X server        the click lands on :9, not on your screen
#   a private D-Bus session  our daemon, our tray, nobody else's
#   a private HOME           set before dbus-run-session, because services
#                            D-Bus activates (xfconfd) inherit the *bus's*
#                            environment, not the script's — that single
#                            detail is what went wrong last time
#   a checksum guard         the real config is fingerprinted first and
#                            re-checked between phases; anything that moves
#                            it stops the run
#
# HOME is reset as well as XDG_CONFIG_HOME because a program that ignores the
# XDG variables still writes to $HOME/.config, and one that does is exactly
# the kind of thing this is defending against.
#
# Needs: Xephyr, xfce4-panel, xfce4-screenshooter, xdotool, python3 (Pillow).
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$repo/docs/screenshots"
work="$(mktemp -d)"
display=":9"
# Small enough that the menu fills the frame, tall enough to hold it above a
# bottom panel.
geometry="520x360"

for tool in Xephyr xfce4-panel xfce4-screenshooter xdotool python3; do
    command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }
done

cleanup() {
    [ -n "${xephyr_pid:-}" ] && kill "$xephyr_pid" 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT

echo "fingerprinting the real xfce4 config…"
find "$HOME/.config/xfce4" -type f -name '*.xml' -exec sha256sum {} \; \
    | sort > "$work/before.sums"
echo "  $(wc -l < "$work/before.sums") files"

# One panel, one plugin. Nothing else to draw and nothing else to click, so
# the icon is always hard against the panel's left edge.
mkdir -p "$work/home/.config/xfce4/xfconf/xfce-perchannel-xml" \
         "$work/home/.config/nightlightd"
cat > "$work/home/.config/nightlightd/config.toml" <<'CONFIG'
day_temp = 6500
night_temp = 2700
CONFIG
cat > "$work/home/.config/xfce4/xfconf/xfce-perchannel-xml/xfce4-panel.xml" <<'PANEL'
<?xml version="1.0" encoding="UTF-8"?>
<channel name="xfce4-panel" version="1.0">
  <property name="configver" type="int" value="2"/>
  <property name="panels" type="array">
    <value type="int" value="1"/>
    <property name="dark-mode" type="bool" value="true"/>
    <property name="panel-1" type="empty">
      <property name="position" type="string" value="p=10;x=0;y=0"/>
      <property name="length" type="uint" value="100"/>
      <property name="position-locked" type="bool" value="true"/>
      <property name="size" type="uint" value="30"/>
      <property name="plugin-ids" type="array">
        <value type="int" value="1"/>
      </property>
    </property>
  </property>
  <property name="plugins" type="empty">
    <property name="plugin-1" type="string" value="systray">
      <property name="square-icons" type="bool" value="true"/>
      <property name="show-frame" type="bool" value="false"/>
    </property>
  </property>
</channel>
PANEL

cargo build --release -p nightlightd -p nightlight-tray \
    --manifest-path "$repo/Cargo.toml" -q

echo "starting a $geometry X server on $display…"
Xephyr "$display" -screen "$geometry" -br -ac -noreset > "$work/xephyr.log" 2>&1 &
xephyr_pid=$!
sleep 3

cat > "$work/session.sh" <<SESSION
#!/usr/bin/env bash
set -u
export GTK_THEME=Adwaita:dark
guard() {
  find "$HOME/.config/xfce4" -type f -name '*.xml' -exec sha256sum {} \\; | sort \\
    | diff -q "$work/before.sums" - > /dev/null \\
    || { echo "!! the real config moved at \$1 — stopping"; exit 9; }
  echo "  [\$1] real config untouched"
}
guard start
"$repo/target/release/nightlightd" --daemon > "$work/daemon.log" 2>&1 &
daemon=\$!
sleep 2
xfce4-panel --disable-wm-check > "$work/panel.log" 2>&1 &
panel=\$!
sleep 5
"$repo/target/release/nightlight-tray" > "$work/tray.log" 2>&1 &
tray=\$!
sleep 5
guard "everything up"
# The capture is armed first, with a delay: a screenshot tool that takes
# focus closes the very menu it was aimed at.
xfce4-screenshooter -f -d 4 -s "$work/frame.png" > /dev/null 2>&1 &
sleep 1
xdotool mousemove 15 344 click 3
sleep 6
guard "after the click"
xdotool key Escape
# Killed by hand rather than left to the bus going away: none of the three
# exits when its session bus does, so without this the run leaves a daemon
# and a tray behind every time.
kill \$daemon \$panel \$tray 2>/dev/null
SESSION
chmod +x "$work/session.sh"

echo "opening the menu…"
env -i HOME="$work/home" \
    XDG_CONFIG_HOME="$work/home/.config" \
    XDG_CACHE_HOME="$work/home/.cache" \
    XDG_DATA_HOME="$work/home/.local/share" \
    XDG_STATE_HOME="$work/home/.local/state" \
    PATH=/usr/bin:/bin DISPLAY="$display" \
    dbus-run-session -- "$work/session.sh" 2>&1 | grep -E '^\s+\[|moved at'

[ -f "$work/frame.png" ] || { echo "no frame was captured" >&2; exit 1; }

mkdir -p "$out"
python3 - "$work/frame.png" "$out/tray-menu.png" <<'CROP'
import sys
from PIL import Image

im = Image.open(sys.argv[1]).convert("RGB")
w, h = im.size
px = im.load()
# The nested root is pure black, so anything drawn is anything not black.
# The menu's own left edge is the frame's, and the panel strip runs the whole
# width — so the width is taken from the menu's rows only, above the panel.
drawn = [x for y in range(0, h - 40) for x in range(w) if px[x, y] != (0, 0, 0)]
if not drawn:
    sys.exit("nothing was drawn above the panel — did the menu open?")
right = max(drawn) + 12
top = min(y for y in range(h) for x in range(w) if px[x, y] != (0, 0, 0))
if right < 80:
    sys.exit(f"the drawn block is only {right}px wide — that is not a menu")
im.crop((0, top, right, h)).save(sys.argv[2])
print(f"  menu cropped to {right}x{h - top}")
CROP

echo
echo "$out/tray-menu.png"
