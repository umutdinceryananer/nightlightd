# Releasing

The routine that has shipped every release so far, written down after
three from-memory runs. One pass, top to bottom; nothing here is
optional, because every line exists from forgetting it once.

## Before the tag

- [ ] CI green on main. `cargo clippy --workspace --all-targets` and
      `cargo test --workspace` clean locally.
- [ ] The dogfooding gauntlet on the release build, not on trust: a
      real suspend cycle, a monitor unplug/replug (or at least a
      `xrandr --gamma 1:1:1` wipe, healed within a tick), toggles at
      night with your own config.
- [ ] Bump `version` in the workspace `Cargo.toml`. One line; all five
      crates inherit it. Then `cargo build` so `Cargo.lock` follows,
      and commit both together.
- [ ] README: the Status paragraph's version and the `.deb` filename
      in Install.
- [ ] `docs/ISSUES.md`: every issue this release closes carries its
      Done mark.
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
      `dist/aur/` in this repo.
- [ ] The release page renders, the `.deb` installs on a stock Mint,
      `systemctl --user restart nightlightd` picks the new binary up.

## Afterwards

- [ ] Close or comment the GitHub issues the release answers, with a
      link to the notes.
- [ ] Announce only when the release deserves it. Plain words; the
      README's voice.
