#!/usr/bin/env bash
# Push dist/aur/PKGBUILD to the AUR, checking first that it describes the
# release it claims to.
#
# One command because the last time this was five, it was skipped: 0.2.1
# never reached the AUR, the package sat a release behind for a month, and
# the checklist note that was supposed to prevent that did not. The AUR goes
# down for maintenance often enough that this step routinely outlives the
# session it belongs to, so it has to be cheap to come back to.
#
# Needs: git with your AUR ssh key, makepkg (for .SRCINFO), curl, sha256sum.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pkgbuild="$repo/dist/aur/PKGBUILD"
work="${TMPDIR:-/tmp}/aur-nightlightd"

for tool in git makepkg curl sha256sum; do
    command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }
done

pkgver="$(sed -n 's/^pkgver=//p' "$pkgbuild")"
declared="$(sed -n "s/^sha256sums=('\\(.*\\)')/\\1/p" "$pkgbuild")"
[ -n "$pkgver" ] && [ -n "$declared" ] || { echo "cannot read pkgver/sha256sums" >&2; exit 1; }
echo "PKGBUILD says $pkgver"

# The checksum is the one thing here that fails silently and late: a bumped
# pkgver with last release's hash builds nothing and is only discovered by
# whoever tried to install it. So the archive is fetched and hashed before
# anything is pushed.
url="https://github.com/umutdinceryananer/nightlightd/archive/v$pkgver.tar.gz"
echo "checking $url"
actual="$(curl -sSL "$url" | sha256sum | cut -d' ' -f1)"
if [ "$actual" != "$declared" ]; then
    echo "checksum mismatch for v$pkgver" >&2
    echo "  PKGBUILD: $declared" >&2
    echo "  archive:  $actual" >&2
    exit 1
fi
echo "checksum matches"

rm -rf "$work"
if ! git clone --quiet "ssh://aur@aur.archlinux.org/nightlightd.git" "$work" 2>"$work.err"; then
    sed 's/^/  /' "$work.err" >&2
    rm -f "$work.err"
    echo >&2
    echo "The AUR is unreachable. The release is not finished — run this again" >&2
    echo "when it is back, and do not close the checklist in the meantime." >&2
    exit 1
fi
rm -f "$work.err"

cp "$pkgbuild" "$work/PKGBUILD"
( cd "$work" && makepkg --printsrcinfo > .SRCINFO )
if ( cd "$work" && git diff --quiet ); then
    echo "the AUR already carries this; nothing to push"
    exit 0
fi
( cd "$work" && git add PKGBUILD .SRCINFO && git commit --quiet -m "$pkgver" && git push --quiet )
echo "pushed $pkgver to the AUR"
