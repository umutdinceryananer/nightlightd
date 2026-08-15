#!/usr/bin/env bash
# Build the release's static tarball: the daemon and the terminal dashboard,
# linked against musl so they run on any x86_64 Linux of any age.
#
# A script because the hand-assembled version drifted. 0.3.0 shipped without
# the INSTALL file that every release before it carried, while the README went
# on telling people to follow it — and the INSTALL that did ship said the
# bundled unit starts ~/.local/bin/nightlightd when the unit actually said
# ~/.cargo/bin, so anyone who followed it literally got a service that would
# not start. Both are the same defect: a step done by hand every few weeks.
#
# Needs: the musl target (rustup target add x86_64-unknown-linux-musl).
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target=x86_64-unknown-linux-musl
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo/Cargo.toml" | head -1)"
# The archive drops rust's "unknown" vendor field. Every release since 0.1.0
# has been named this way and people link to those files.
name="nightlightd-$version-x86_64-linux-musl"
out="${1:-$repo/target/dist}"
stage="$out/$name"

rustup target list --installed | grep -qx "$target" || {
    echo "missing target: rustup target add $target" >&2
    exit 1
}

echo "building $version for $target…"
cargo build --release --target "$target" -p nightlightd -p nightlight-tui \
    --manifest-path "$repo/Cargo.toml" -q

rm -rf "$stage"
mkdir -p "$stage"
install -m755 "$repo/target/$target/release/nightlightd" "$stage/"
install -m755 "$repo/target/$target/release/nightlight-tui" "$stage/"
install -m644 "$repo/dist/musl/INSTALL" "$stage/"
install -m644 "$repo/LICENSE" "$stage/"
# The unit in dist/ is the from-source one and starts ~/.cargo/bin, which is
# where `cargo install` puts things. A tarball is not a cargo install, and
# INSTALL tells the reader to use ~/.local/bin, so the copy that ships beside
# it has to say the same. Rewritten rather than kept as a second file, so
# there is still only one unit to edit.
sed 's|%h/\.cargo/bin/nightlightd|%h/.local/bin/nightlightd|' \
    "$repo/dist/nightlightd.service" > "$stage/nightlightd.service"
chmod 644 "$stage/nightlightd.service"
strip "$stage/nightlightd" "$stage/nightlight-tui" 2>/dev/null || true

# What the README promises is in here, checked rather than assumed.
for f in nightlightd nightlight-tui INSTALL LICENSE nightlightd.service; do
    [ -f "$stage/$f" ] || { echo "missing from the tarball: $f" >&2; exit 1; }
done
grep -q '\.local/bin/nightlightd' "$stage/nightlightd.service" || {
    echo "the bundled unit does not point where INSTALL says it does" >&2
    exit 1
}
ldd "$stage/nightlightd" 2>&1 | grep -q "statically linked" || {
    echo "nightlightd is not static" >&2
    exit 1
}

tar czf "$out/$name.tar.gz" -C "$out" "$name"
rm -rf "$stage"
echo
tar tzf "$out/$name.tar.gz" | sed 's/^/  /'
echo
echo "$out/$name.tar.gz"
sha256sum "$out/$name.tar.gz" | cut -d' ' -f1 | sed 's/^/sha256 /'
