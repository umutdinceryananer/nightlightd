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
#
# UBUNTU_MIRROR fetches Ubuntu's half of the packages from a mirror instead of
# archive.ubuntu.com and security.ubuntu.com, for the day those stop
# answering. 0.5.0's check sat ten minutes in apt-get update when they did,
# while the Mint server and the country mirrors answered at once:
#   UBUNTU_MIRROR=http://tr.archive.ubuntu.com/ubuntu scripts/deb-check.sh
# The packages are the same either way; only where apt fetches them changes.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image="${MINT_IMAGE:-linuxmintd/mint22.3-amd64}"
mirror="${UBUNTU_MIRROR:-}"
mirror="${mirror%/}"
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

# UBUNTU_MIRROR, when the caller set one: Ubuntu's two servers swapped for it
# in whichever sources file this release of Mint keeps them in.
if [ -n "${UBUNTU_MIRROR:-}" ]; then
    for list in /etc/apt/sources.list /etc/apt/sources.list.d/*.list; do
        [ -f "$list" ] || continue
        sed -i -e "s|http://archive.ubuntu.com/ubuntu/\?|$UBUNTU_MIRROR/|" \
            -e "s|http://security.ubuntu.com/ubuntu/\?|$UBUNTU_MIRROR/|" "$list"
    done
fi
# A server that stops answering makes apt wait rather than fail, so it gets
# three minutes. And an index apt could not fetch is only a warning to it,
# so --error-on=any: without it a dead server reads as "xvfb-run: not found"
# three checks later, a failure blamed on the packages.
if ! timeout 180 apt-get update -qq --error-on=any >/dev/null; then
    echo "apt-get update failed or timed out. If Ubuntu's servers are down, run" >&2
    echo "again with UBUNTU_MIRROR=http://<country>.archive.ubuntu.com/ubuntu" >&2
    exit 3
fi

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
status=0
docker run --rm -v "$debs:/debs:ro" -v "$work/inside.sh:/inside.sh:ro" \
    -e UBUNTU_MIRROR="$mirror" "$image" bash /inside.sh || status=$?
echo
case "$status" in
    0) echo "the packages install and run on $image${mirror:+, Ubuntu packages from $mirror}" ;;
    # inside.sh's own code for an apt that never reached its servers: not a
    # verdict on the packages, which were never installed.
    3) echo "nothing was checked: apt could not reach its servers on $image" >&2
       exit 1 ;;
    *) echo "the packages failed on $image; do not call this release finished" >&2
       exit 1 ;;
esac
