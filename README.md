```
          _       __    __  ___       __    __      __
   ____  (_)___ _/ /_  / /_/ (_)___ _/ /_  / /_____/ /
  / __ \/ / __ `/ __ \/ __/ / / __ `/ __ \/ __/ __  /
 / / / / / /_/ / / / / /_/ / / /_/ / / / / /_/ /_/ /
/_/ /_/_/\__, /_/ /_/\__/_/_/\__, /_/ /_/\__/\__,_/
        /____/              /____/

  zero-config screen colour temperature for X11
  reads your timezone · refuses to run twice · survives suspend
```

[![CI](https://github.com/umutdinceryananer/nightlightd/actions/workflows/ci.yml/badge.svg)](https://github.com/umutdinceryananer/nightlightd/actions/workflows/ci.yml)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

> **Status: v0.1-alpha.** The daemon works — timezone-based location, a single-instance D-Bus lock, gamma ramps over XRandR, re-apply on resume from suspend, and a `--status` readout. Milestones M0–M4 are done; it is now in a week of dogfooding. There are no packaged releases yet, so if you need a night light you can lean on today, install [gammastep](https://gitlab.com/chinstrap/gammastep) — this is not that yet.

---

## X11 only

This tool writes gamma ramps through XRandR. That mechanism does not exist under GNOME's or KDE's Wayland sessions, and it never will. Wayland support, if it ever lands, will cover wlroots compositors only (Sway, Hyprland, river) through a separate backend.

If you are on Wayland today, use [`wl-gammarelay-rs`](https://github.com/MaxVerevkin/wl-gammarelay-rs).

---

## Why this exists

redshift was archived in April 2026. gammastep took its place and is maintained, packaged everywhere, and works. This project is not "a maintained redshift" — that already exists.

It exists to fix three defects that gammastep inherited from redshift's architecture and cannot easily shed. Each one was measured, not assumed. The evidence, with commands and outputs, is in [`docs/PRIOR-ART.md`](docs/PRIOR-ART.md).

**1. It will not start without being told where you live.**
With no config file and no `-l`, gammastep prints its settings, hangs at location acquisition, and applies nothing — with no error message. Geoclue2, its only automatic provider, is unavailable on most desktops.

`nightlightd` reads `/etc/localtime` and looks the coordinate up in the timezone database that every Linux system already ships. No network, no permissions, no questions. Sunset lands within a few minutes of correct, which is all the transition curve needs.

**2. Two copies can run at once, and the screen flickers.**
Nothing prevents it. On a stock Mint Xfce install, four redshift instances had accumulated from three autostart mechanisms that do not know about each other.

`nightlightd` claims a DBus name on startup. A second instance finds the name taken and exits. The failure mode is architecturally impossible.

**3. It does not react when the ramp is wiped.**
`nm -D` on the gammastep binary shows no `xcb_randr_select_input`. It never subscribes to RandR events, so it cannot notice a resume from suspend, a resolution change, or a monitor being plugged in. It recovers on its next polling tick, if at all. It reads `get_screen_resources_current`, so hotplugged monitors are likely never seen.

`nightlightd` subscribes to screen events and rewrites the ramp when they fire.

Everything else — packaging, systemd units, solar-elevation scheduling — gammastep already does well. Those are not selling points here.

---

## Design

A daemon does the work; thin clients talk to it over DBus.

```
tray icon  ─┐
             ├─► DBus ─► nightlightd ─► gamma ramp
CLI        ─┘              ▲    ▲
                           │    └─ RandR events
                           └────── timer
```

The daemon has no interface. If the tray icon dies, the filter lives. One brain, many remotes.

Read [`docs/HOW-IT-WORKS.md`](docs/HOW-IT-WORKS.md) for the long version, written for someone who has never heard of a gamma ramp.

---

## Roadmap

Tracked in [`docs/ISSUES.md`](docs/ISSUES.md).

| | | |
|---|---|---|
| M-1 | Upstream fix to gammastep first | not started |
| M0 | Skeleton | ✅ done |
| M1 | Core library — colour, sun, timezone | ✅ done |
| M2 | X11 backend | ✅ done |
| M3 | Daemon and event loop | ✅ done |
| M4 | DBus, CLI, systemd, suspend | ✅ done |
| M5 | Tray icon and settings | not started |
| M6 | Packaging and release | not started |

Before writing a line of Rust here, the timezone fallback is going upstream to gammastep as a merge request. It helps far more people there, and the review will say whether defects 2 and 3 can be patched in place — in which case this repository should not exist. See [`docs/UPSTREAM-MR.md`](docs/UPSTREAM-MR.md).

---

## Licence

See [`LICENSE`](LICENSE).
