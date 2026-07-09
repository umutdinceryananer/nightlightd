# Prior art

Everything below was measured, not assumed. Commands and outputs are reproducible.

**Test environment**

| | |
|---|---|
| Distro | Linux Mint (Ubuntu 24.04 "noble" base) |
| Desktop | Xfce |
| Session | X11 |
| gammastep (packaged) | 2.0.9-1build2 (Ubuntu `noble/universe`) |
| gammastep (upstream) | 2.0.11, built from source at `main` |
| Date tested | 2026-07-09 |

**All three defects were reproduced twice:** once against the distribution package (2.0.9), and again against upstream `main` (2.0.11) built from source. Two patch releases have shipped in between and none of them touch these behaviours.

---

## Summary

| Tool | Status | Relevant to us |
|---|---|---|
| redshift | Repository archived April 2026 | The incumbent, now unmaintained |
| gammastep | Maintained redshift fork, in Debian/Ubuntu/Mint repos | **The real incumbent.** Three measured defects. |
| wl-gammarelay-rs | Maintained, Rust, DBus daemon + client | Same architecture we want. Wayland only. |
| Gammy | Requires building from source on Linux | Brightness-focused. Effectively unavailable. |
| f.lux (fluxgui) | PPA last built 2019, supports up to Ubuntu 18.04 | Dead. |
| Xfce native night light | Announced for 4.16 in 2019, still absent | Feature request open since 2019 |

The gap this project fills: **nobody has built the `wl-gammarelay-rs` architecture for X11**, and **nobody solves the location problem without asking the user.**

---

## redshift

The upstream repository was archived in April 2026. The Debian wiki, the Arch wiki, and Linux Mint's own discussions all now point users to gammastep instead.

Wikipedia's entry notes that since redshift's development was abandoned, Linux Mint is looking for a replacement. Mint's own discussion thread confirms that gammastep only works under wlroots-based Wayland compositors, so redshift is still required for Cinnamon's X11 session.

That is an open institutional gap, not a hypothetical one.

### Multi-instance flicker, observed in the wild

On a stock Mint Xfce install, after using redshift normally for a while:

```
$ pgrep -af 'redshift'
25337 /usr/bin/python3 /usr/bin/redshift-gtk
25348 /usr/bin/redshift -v
36328 /usr/bin/python3 /usr/bin/redshift-gtk
36332 /usr/bin/redshift -v
36340 /usr/bin/python3 /usr/bin/redshift-gtk
36345 /usr/bin/redshift -v
36349 /usr/bin/python3 /usr/bin/redshift-gtk
36353 /usr/bin/redshift -v
```

Four independent instances, each spawning its own worker. Eight processes, all writing to the same gamma ramp with different targets. The screen visibly oscillates.

They accumulated from three sources that do not know about each other:

1. `/etc/xdg/autostart/` (shipped by the package)
2. `~/.config/autostart/redshift-gtk.desktop` (added by the desktop's own session settings UI)
3. Manual invocations from a terminal

Note also that `apt purge redshift` removed the binaries but left the running processes alive and left `~/.config/autostart/redshift-gtk.desktop` in place.

Xfce's own issue tracker documents this class of bug: it is not possible to start the app multiple times, they interfere with one another, and the screen flickers.

---

## gammastep 2.0.9

The maintained fork. Already in the Ubuntu, Debian and Mint repositories. Ships two systemd user units and enables them globally on install:

```
Created symlink /etc/systemd/user/graphical-session.target.wants/gammastep-indicator.service
Created symlink /etc/systemd/user/graphical-session.target.wants/gammastep.service
```

**What it already does well,** and therefore what is *not* a differentiator for us:

- Packaged in every relevant distro. Users do not compile it.
- Ships a systemd user service. Autostart is handled correctly.
- Sun-elevation-based transition, with the same thresholds we planned (`> 3.0` day, `< -6.0` night).
- randr, wlroots-Wayland and drm backends.
- A tray indicator, equivalent to `redshift-gtk`.

Three defects follow. All three were measured on the machine described above.

### Defect 1 — will not start without a manually supplied location

Run with no config file, no arguments:

```
$ gammastep -m randr -v
Notice: Solar elevations: > 3.0 (Day), < -6.0 (Night)
Notice: Temperatures: 6500K (Day), 4500K (Night)
Notice: Brightness: 1.00:1.00
Notice: Gamma (Day): 1.000, 1.000, 1.000
Notice: Gamma (Night): 1.000, 1.000, 1.000
```

It stops there. No `Location`, no `Period`, no `Color temperature`. Nothing is applied.

It does not time out. Under `timeout 180`, the process was still blocked after three minutes and had to be killed:

```
$ timeout 180 ./src/gammastep -m randr -v ; echo "exit=$?"
Notice: Solar elevations: > 3.0 (Day), < -6.0 (Night)
Notice: Temperatures: 6500K (Day), 4500K (Night)
Notice: Brightness: 1.00:1.00
Notice: Gamma (Day): 1.000, 1.000, 1.000
Notice: Gamma (Night): 1.000, 1.000, 1.000
poll: Interrupted system call
Error: Unable to get location from provider.
exit=124
```

**Read that error message carefully — it is misleading.** It is not the program diagnosing a failure. It is emitted only because `timeout` sent `SIGTERM`, `poll()` returned `EINTR`, and the caller interpreted the interrupted syscall as provider failure. Left alone, the process blocks in `poll()` forever and prints nothing further.

An MR description that says "it emits no error" will be refuted by a maintainer who runs it and presses Ctrl+C. The accurate claim is: **it blocks indefinitely and applies nothing; the error surfaces only when the process is signalled.**

Supply coordinates by hand and everything works:

```
$ gammastep -m randr -l 39.9:32.8 -v
Notice: Solar elevations: > 3.0 (Day), < -6.0 (Night)
Notice: Temperatures: 6500K (Day), 4500K (Night)
Notice: Brightness: 1.00:1.00
Notice: Gamma (Day): 1.000, 1.000, 1.000
Notice: Gamma (Night): 1.000, 1.000, 1.000
Notice: Location: 39.90 N, 32.80 E
Notice: Color temperature: 6500K
Notice: Brightness: 1.00
Notice: Status: Enabled
Notice: Period: Night
Notice: Color temperature: 4500K
```

The location providers are geoclue2 and manual. Geoclue is unreliable to the point of being effectively absent on most desktops. When it fails, the user's only remaining option is to find their own latitude and longitude and write a config file.

The Arch wiki states this plainly: redshift needs your location to start, it tries several routines to obtain it, and if none work you must enter it manually. Gammastep inherited this.

**A user installs it, runs it, and nothing happens. That is the single largest defect in this software.**

Separately: on an X11 session with no `WAYLAND_DISPLAY`, the default backend selection tries Wayland and exits rather than falling back to randr:

```
Error: Could not connect to wayland display, exiting.
Error: Failed to start adjustment method: wayland
```

This is a smaller bug but compounds the first: a naive user gets nothing, twice.

### Defect 2 — no single-instance lock

```
$ gammastep -m randr -l 39.9:32.8 -t 2500:2500 &
$ sleep 2
$ gammastep -m randr -l 39.9:32.8 -t 6500:6500 &
$ sleep 8
$ pgrep -c gammastep
2
```

Both instances live. Both write the ramp on their own schedule with conflicting targets. The screen oscillates between warm and neutral, indefinitely.

Worse: on a machine where gammastep's own systemd user service was already running, the same experiment produced

```
$ pgrep -c gammastep
3
```

Two manual invocations plus the daemon the tool itself installed. **Starting gammastep by hand while gammastep is running does not warn, does not refuse, and does not deduplicate.** Three processes fought over one gamma ramp.

Nothing in the program prevents this. Nothing warns about it. The same defect as redshift, inherited unchanged.

### Defect 3 — does not subscribe to RandR events

The full list of RandR symbols the binary imports:

```
$ nm -D /usr/bin/gammastep | grep -i randr
                 U xcb_randr_get_crtc_gamma
                 U xcb_randr_get_crtc_gamma_blue
                 U xcb_randr_get_crtc_gamma_green
                 U xcb_randr_get_crtc_gamma_red
                 U xcb_randr_get_crtc_gamma_reply
                 U xcb_randr_get_crtc_gamma_size
                 U xcb_randr_get_crtc_gamma_size_reply
                 U xcb_randr_get_screen_resources_current
                 U xcb_randr_get_screen_resources_current_crtcs
                 U xcb_randr_get_screen_resources_current_reply
                 U xcb_randr_query_version
                 U xcb_randr_query_version_reply
                 U xcb_randr_set_crtc_gamma_checked
```

**`xcb_randr_select_input` is absent.** That call is the only way to ask the X server to deliver `RRScreenChangeNotify` and `RRCrtcChangeNotify`. Gammastep never makes it. It is not subscribed to screen events and cannot be.

The symbol list is byte-for-byte identical in upstream 2.0.11 built from source. This has not been fixed and is not being fixed.

Consequences:

- When something wipes the gamma ramp — resume from suspend, resolution change, exiting a fullscreen application — gammastep does not notice. The screen stays neutral until its next polling tick.
- It calls `get_screen_resources_current` rather than `get_screen_resources`. The `_current` variant reads the server's cached view and does not re-poll the hardware. A monitor attached after startup is therefore very likely never picked up at all.

Empirically, clearing the ramp by hand while gammastep is running:

```
$ xrandr --output eDP-1 --gamma 1:1:1
```

The screen flashes neutral and returns to warm. The recovery is fast, because the polling interval is short — but it is a poll, not a reaction. On a laptop resuming from suspend the window is longer, and monitor hotplug is not handled at all.

**Honest scoping:** defect 3 is real but its user-visible severity is "the screen is neutral for a few seconds," not "the filter is permanently lost." The monitor hotplug half of it is the serious part.

### A note for later

```
$ xrandr --verbose | grep GAMMA_LUT_SIZE
	GAMMA_LUT_SIZE: 4096
```

The hardware exposes a 4096-entry LUT. The legacy `crtc_gamma` path gammastep uses is typically capped far below that. Not a v0.1 concern, but a potential future differentiator: fewer banding artefacts at low temperatures.

---

## wl-gammarelay-rs

The closest thing to this project that already exists. Rust, single-threaded, zero runtime dependencies. Exposes a DBus interface at `rs.wl-gammarelay`, with `UpdateTemperature`, `UpdateBrightness`, `UpdateGamma` and `ToggleInverted` methods and writable properties. It ships a systemd user unit typed `dbus` with `BusName=rs.wl-gammarelay`. It is packaged in the AUR, on crates.io, and in FreeBSD ports. A separate tray applet exists for it.

This is precisely the daemon-plus-client architecture described in `HOW-IT-WORKS.md`, including the DBus name as an implicit single-instance lock.

**It is Wayland only.** No X11 backend. It also has no solar logic at all — it is a knob for keybindings, not an automatic night filter.

Two conclusions:

1. The architecture is validated. Someone competent built it and it works.
2. Nobody has built it for X11, and nobody has combined it with automatic sun-based scheduling.

---

## Xfce

Native night light was announced as a target for Xfce 4.16 during the 2019 development cycle, planned to land in the power manager. It did not ship. Reviews of 4.18 in 2022 still list its absence as a gap, and direct users to redshift.

The feature request on `xfce4-power-manager` has been open since 2022 and cites redshift's own README: because redshift adjusts colour through gamma ramps, many of its problems cannot be solved properly, and desktop-integrated implementations are preferable.

Two readings of this:

- **Against us:** upstream considers gamma ramps the wrong layer. We are building on a layer its own maintainers call a dead end.
- **For us:** seven years have passed and nothing has shipped. In the meantime, X11 users have gamma ramps or nothing. There is a subscribed, waiting audience on that issue.

---

## What this establishes

The three differentiators, restated as measured claims rather than intentions:

1. **Zero-configuration location.** Gammastep hangs silently without `-l`. We read the timezone and never ask.
2. **Guaranteed single instance.** Gammastep runs two copies and flickers; `pgrep -c` returns 2. We claim a DBus name.
3. **Event-driven ramp restoration.** Gammastep does not import `xcb_randr_select_input`. We subscribe to RandR events and handle hotplug.

Things that are **not** differentiators, contrary to first assumptions:

- Being packaged. Gammastep is already in `apt`.
- Shipping a systemd unit. Gammastep already does.
- Being a "maintained redshift." Gammastep already is.
- Sun-elevation scheduling. Gammastep already does it, with the same thresholds.

The pitch is narrower than it first appeared, and better evidenced.
