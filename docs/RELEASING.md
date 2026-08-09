# Releasing

The routine that has shipped every release so far, written down after
three from-memory runs. One pass, top to bottom; nothing here is
optional, because every line exists from forgetting it once.

## Which number

Patch (`0.2.x`) for fixes and for anything that only adds a knob to
what already exists. Minor (`0.3.0`) once the wire changes, once a
config field appears, or once an interface grows a surface people
would have to be told about. #39 and #45 together are a minor: the
config learned two fields, D-Bus learned two methods, and both the
panel's curve and the dashboard's keys became something else. Decide
this before the bump, not after, because the tag is the one thing
that cannot be taken back.

## Before the tag

- [ ] CI green on main. `cargo clippy --workspace --all-targets` and
      `cargo test --workspace` clean locally.
- [ ] The dogfooding gauntlet on the release build, not on trust: a
      real suspend cycle, a monitor unplug/replug (or at least a
      `xrandr --output <out> --gamma 1:1:1` wipe, healed within a
      tick), toggles at night with your own config. Stop *every*
      daemon first — `pkill -f 'nightlightd --daemon'`, not just the
      unit, or a tray-spawned one holds the bus name and the build you
      meant to test never starts.
- [ ] Bump `version` in the workspace `Cargo.toml`. One line; all five
      crates inherit it. Then `cargo build` so `Cargo.lock` follows,
      and commit both together.
- [ ] README: the Status paragraph's version and the `.deb` filename
      in Install.
- [ ] `docs/ISSUES.md`: every issue this release closes carries its
      Done mark.
- [ ] The showcase, against the build you are about to ship. The
      screenshots and the GIF are the first thing anyone sees, and
      they rot silently — a footer with the old version, a settings
      tab with rows that moved, a curve drawn the old way. Added
      2026-08 after #39 and #45 changed the dashboard's footer, its
      help overlay, both charts and the settings list, leaving a
      recording that shows a program nobody can download. Re-record
      if any of them moved; `DEMO_SCRIPT` in the dashboard drives the
      reel, so a new key worth showing goes in there too.
- [ ] Update your own install and live on it at least briefly:
      `cargo install --path cli --locked`, then tray, panel, tui the
      same way. `--locked` matters — a bare install resolves newer
      dependencies than CI ever tested.

## The tag

- [ ] Annotated tag, `vX.Y.Z`. The tag message is the release notes,
      verbatim — write it like the notes page, credit contributors by
      name with their issue.
- [ ] Push the branch first, then the tag. The tag push starts CI's
      `.deb` build and the release draft.

## After CI

- [ ] musl tarball: the static daemon + tui pair, built with the musl
      target, smoke-tested (`./nightlightd --status` against a live
      daemon), uploaded to the release.
- [ ] AUR: `pkgver` bump in the PKGBUILD, `updpkgsums`, regenerate
      `.SRCINFO`, push over ssh. Mirror the final files into
      `dist/aur/` in this repo. The AUR goes down for maintenance
      often enough that this step outlives the session it belongs to;
      0.2.1's push waited on one. If it is down, the release is not
      finished — leave the checklist open rather than calling it
      shipped, because nothing else will remind you.
- [ ] The release page renders, the `.deb` installs on a stock Mint,
      `systemctl --user restart nightlightd` picks the new binary up.

## Afterwards

- [ ] Close or comment the GitHub issues the release answers, with a
      link to the notes.
- [ ] Announce only when the release deserves it. Plain words; the
      README's voice.
