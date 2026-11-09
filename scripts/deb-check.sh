#!/usr/bin/env bash
# Install the released .debs on a clean Linux Mint and open the panel.
#
# This is the step that caught 0.3.2. ubuntu-latest had moved to 26.04 under
# the release workflow, the panel came out asking for glibc 2.43, and no Mint
# 22 could install it; CI built and published it without complaint. Only an
# install on the system the package is for can say so, and a container is one
# command where a spare machine is a chore that gets skipped.
#
# Usage: scripts/deb-check.sh [TAG | DIR]
#   TAG  a release whose .debs are fetched from GitHub (default: the newest tag)
#   DIR  a directory of .debs built locally, checked before they are uploaded
#
# Needs: docker, gh. MINT_IMAGE picks another release of Mint.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image="${MINT_IMAGE:-linuxmintd/mint22.3-amd64}"
target="${1:-$(git -C "$repo" describe --tags --abbrev=0)}"

for tool in docker gh; do
    command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }
done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

if [ -d "$target" ]; then
    debs="$(cd "$target" && pwd)"
else
    debs="$work/debs"
    mkdir "$debs"
    gh release download "$target" --repo umutdinceryananer/nightlightd \
        --pattern '*.deb' --dir "$debs"
fi

cat > "$work/inside.sh" <<'INSIDE'
set -u
export DEBIAN_FRONTEND=noninteractive
failed=0

# check CLAIM COMMAND...: runs the command, and shows why when it fails.
check() {
    local claim="$1" output
    shift
    if output="$("$@" 2>&1)"; then
        echo "  ok    $claim"
    else
        echo "  FAIL  $claim"
        echo "$output" | tail -8 | sed 's/^/        /'
        failed=1
    fi
}

# A package that will not install leaves nothing for the later checks to say.
stop_if_failed() {
    [ "$failed" -eq 0 ] || exit 1
}

main="$(ls /debs/nightlightd_*_amd64.deb)"
panel="$(ls /debs/nightlightd-panel_*_amd64.deb)"
version="$(dpkg-deb -f "$main" Version | cut -d- -f1)"
apt-get update -qq >/dev/null

echo "nightlightd $version, by itself"
check "installs" apt-get install -y -q "$main"
stop_if_failed
for program in nightlightd nightlight-tray nightlight-tui; do
    check "$program reports $version" sh -c "$program --version | grep -qx '$program $version'"
done
check "the unit starts /usr/bin/nightlightd" \
    grep -qx 'ExecStart=/usr/bin/nightlightd --daemon' /usr/lib/systemd/user/nightlightd.service
check "the tray starts with the session" test -f /etc/xdg/autostart/nightlight-tray.desktop
check "the panel is not in this package" sh -c '! command -v nightlight-panel'

echo "nightlightd-panel, beside it"
check "installs" apt-get install -y -q "$panel"
stop_if_failed
check "nightlight-panel reports $version" \
    sh -c "nightlight-panel --version | grep -qx 'nightlight-panel $version'"
apt-get install -y -qq desktop-file-utils xvfb xauth dbus-x11 >/dev/null 2>&1
entry=/usr/share/applications/nightlight-panel.desktop
check "the menu entry is valid" desktop-file-validate "$entry"

# Run what the menu would run. `timeout` ending it means the window was still
# up after eight seconds; anything else means it did not stay open.
command="$(sed -n 's/^Exec=//p' "$entry")"
check "the panel opens and stays open" sh -c \
    "xvfb-run -a dbus-run-session -- timeout 8 $command; [ \$? -eq 124 ]"

exit "$failed"
INSIDE

docker pull -q "$image" >/dev/null
if docker run --rm -v "$debs:/debs:ro" -v "$work/inside.sh:/inside.sh:ro" \
    "$image" bash /inside.sh; then
    echo
    echo "the packages install and run on $image"
else
    echo
    echo "the packages failed on $image; do not call this release finished" >&2
    exit 1
fi
