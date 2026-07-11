# Issue backlog

A screen colour temperature tool for X11. Written in Rust.

Each issue below follows the same shape:

- **What:** the work to be done.
- **Why:** why it exists.
- **Done when:** the acceptance criterion.
- **Difficulty:** Easy / Medium / Hard
- **Depends on:** which issue must land first.

Milestones run in order. At the end of M1 you have a testable library. At the end of M2 you have a program that changes your screen colour. At the end of M4 you have a tool you can actually use every day. M5 and M6 turn it into a product.

**Name.** The crate, the repository, the Debian package, the Flatpak and the binary are all `nightlightd`. The DBus name is `org.nightlightd.Daemon`. The Flatpak application id is `io.github.<user>.nightlightd`.

Verified free on crates.io and on GitHub (no repository with that exact name). Check the AUR and Flathub before committing. The bare name `nightlight` is not available: it is taken on crates.io by a macOS Night Shift CLI, thirty-five GitHub repositories carry it, and it collides with the built-in feature name used by GNOME, KDE, Cinnamon and Windows — which would bury this tool in search results rather than surfacing it. The `d` suffix disambiguates, signals "daemon" to any packager, and is free everywhere.

A second `nightlight` binary can be added later as the client, if the daemon-and-client split ever warrants two names. Binary names are not reserved by crates.io; only package names are.

**Read `PRIOR-ART.md` first.** It records what the incumbent (gammastep 2.0.9) actually does and does not do, measured rather than assumed. Three issues below are marked **Verified** — they close defects that were reproduced on real hardware. Everything else is table stakes: gammastep already does it, and doing it too is not a selling point.

The three verified differentiators are **#7**, **#19** and **#13**. If you cut scope, cut anything else first.

---

## M-1 — Upstream first

Do this before opening your own repository. It is small, isolated, and it buys you information you cannot get any other way.

### #0 Submit a timezone location provider to gammastep

- **What:** Fork `gitlab.com/chinstrap/gammastep`. Add `src/location-timezone.c`: read the `/etc/localtime` symlink, extract the zone name, look up the coordinate in `/usr/share/zoneinfo/zone.tab`. Register it in the provider list in `redshift.c`, ordered after `geoclue2` and before `manual`. Open a merge request.
- **Why:** This closes defect 1 (see `PRIOR-ART.md`) for every existing gammastep user, which is orders of magnitude more people than will ever install your tool. It is roughly 80 lines of C against an interface that already exists. It is hard to argue against.
- **What you get either way:**
  - *Merged* → your name is in the history of the tool that replaced redshift. Better signal than a solo repo with nine stars.
  - *Rejected or ignored* → you now have a documented, public reason why this needs rewriting rather than patching, and you can cite it in your own README.
  - *Discussed* → the maintainer tells you why defects 2 and 3 cannot be patched into a `sleep`-loop architecture. That is the conversation that either validates or kills your project, and it costs you a week instead of two months.
- **Detail:** Keep the MR title boring. "Add timezone-based location provider as fallback." Describe the observed behaviour (silent hang with no config) rather than editorialising about it.
- **Done when:** The MR is open.
- **Difficulty:** Medium (C, unfamiliar codebase)
- **Depends on:** —

---

## M0 — Skeleton

### #1 Set up the Cargo workspace

- **What:** Two crates: `core` (pure logic) and `cli` (the binary). A workspace `Cargo.toml` at the root.
- **Why:** Separating pure logic from the screen and the bus is what makes testing possible. It is also the only part of this project where the borrow checker will leave you alone.
- **Done when:** `cargo build` and `cargo test` both run and pass, even if empty.
- **Difficulty:** Easy
- **Depends on:** —

### #2 Licence and README stub

- **What:** Pick GPL-3.0 or MIT. README with a one-line description, an "X11 only" notice, and an empty install section.
- **Why:** Nobody contributes to an unlicensed repo, and nobody packages one. Flathub requires a licence.
- **Done when:** `LICENSE` and `README.md` exist.
- **Difficulty:** Easy
- **Depends on:** —

### #3 CI (GitHub Actions)

- **What:** On every push: `cargo build`, `cargo test`, `cargo clippy`, `cargo fmt --check`.
- **Why:** While learning Rust, clippy is the best teacher you will get for free. It also stops you shipping a release that doesn't compile.
- **Done when:** The green tick shows up.
- **Difficulty:** Easy
- **Depends on:** #1

---

## M1 — Core library (`core`)

Nothing in this milestone touches the screen. It is all numbers, and all of it is testable.

### #4 Colour temperature to RGB gains

- **What:** `fn temperature_to_rgb(kelvin: u32) -> (f64, f64, f64)`, valid for 1000K–10000K. Returns three gains in the range 0.0–1.0.
- **Why:** The heart of the program. When the user says "2800K", this function decides how much green and blue to hold back while leaving red alone.
- **Detail:** Redshift precomputes 1000K–10000K in 100K steps and interpolates between them. Do the same. Evaluating the Planck curve on every call buys you nothing and costs you precision bugs.
- **Done when:** 6500K returns something very close to (1.0, 1.0, 1.0). 2800K returns roughly (1.0, 0.75, 0.55). Tests exist.
- **Difficulty:** Easy
- **Depends on:** #1

### #5 Gamma ramp construction

- **What:** `fn build_ramp(size: u16, gains: (f64, f64, f64)) -> Ramp`. One `u16` array of length `size` per channel.
- **Why:** This is the data format the graphics card expects. You are turning three gains into a lookup table.
- **Detail:** `ramp[i] = (i as f64 / (size - 1) as f64) * gain * 65535.0`. Note that `size` is not a constant — see #10.
- **Done when:** Gains of (1.0, 1.0, 1.0) produce a linear ramp (the identity transform). Tests exist.
- **Difficulty:** Easy
- **Depends on:** #4

### #6 Solar elevation

- **What:** `fn solar_elevation(lat: f64, lon: f64, time: DateTime) -> f64`, returning the sun's angle above the horizon in degrees.
- **Why:** This is how you answer "has the sun set?". Using an angle rather than a clock time handles seasons and latitudes for free.
- **Detail:** The NOAA solar position algorithm. No crate needed — about 40 lines of trigonometry.
- **Done when:** For Ankara, midday on June 21 gives roughly 72°, midday on December 21 gives roughly 26°. Tests cover a few known dates.
- **Difficulty:** Medium (heavy on maths, light on Rust)
- **Depends on:** #1

### #7 Derive location from the timezone

- **What:** `fn location_from_timezone() -> Option<(f64, f64)>`. Read `/etc/timezone` or the `/etc/localtime` symlink, get a string like `Europe/Istanbul`, look up coordinates.
- **Why:** **This is the single most important thing that sets this tool apart.** The most common reason Redshift "just doesn't work" is that Geoclue fails to resolve a location, so sunset can't be computed, so nothing happens. This tool makes no network calls, asks for no permissions, and asks the user nothing. It reads a file.
- **Detail:** The IANA timezone database ships a `zone.tab` file with a representative coordinate for every zone, and it is already installed on every Linux system (`/usr/share/zoneinfo/zone.tab`). Read that first; fall back to a small embedded table if it's missing. An error of ±1° shifts sunset by a few minutes. Nobody will notice.
- **Verified:** gammastep 2.0.9 with no config and no `-l` prints its settings, then hangs at location acquisition and applies nothing. No error is emitted. See `PRIOR-ART.md`, defect 1.
- **Done when:** Tests pass across several `TZ` values. Returns `None` rather than panicking when the file is absent.
- **Difficulty:** Easy
- **Depends on:** #1

### #8 Transition curve

- **What:** `fn target_temperature(elevation: f64, day_temp: u32, night_temp: u32) -> u32`. Smoothly interpolate the target temperature from the sun's angle.
- **Why:** Snapping on and off is jarring and looks cheap. Redshift treats anything above +3° as full daylight and anything below -6° as full night, interpolating linearly between them.
- **Done when:** The function is monotonic (higher sun, higher temperature) and returns the exact endpoints at the extremes.
- **Difficulty:** Easy
- **Depends on:** #6

### #9 Manual location and manual mode

- **What:** Let the user supply latitude and longitude by hand. Also let them ignore the sun entirely and pin a fixed temperature.
- **Why:** Everything automatic needs an escape hatch. It also makes testing far easier.
- **Done when:** `core` supports all three modes: automatic, manual location, fixed temperature.
- **Difficulty:** Easy
- **Depends on:** #7, #8

---

## M2 — The X11 layer

This is where you touch a real system resource for the first time. Rust's ownership model will suddenly start making sense.

### #10 XRandR connection and CRTC discovery

- **What:** Connect to the X server via `x11rb`, enumerate the CRTCs (screens), and query each one's gamma ramp size.
- **Why:** Every screen holds its own ramp, and **the sizes differ** — 256, 1024, 2048. Assume one size and you will crash on somebody's machine.
- **Done when:** The program can print "found 2 screens, ramp sizes 1024 and 256".
- **Difficulty:** Medium
- **Depends on:** #1

### #11 Write the ramp

- **What:** Push a correctly-sized ramp to each CRTC.
- **Why:** This is the actual job.
- **Detail:** No root required. You are writing at the scanout stage, not to the framebuffer — which is why screenshots come out clean and the filter is invisible to screen capture.
- **Done when:** `nightlightd --temp 2800` turns the screen warm. `nightlightd --temp 6500` returns it to normal.
- **Difficulty:** Medium
- **Depends on:** #5, #10

### #12 Restore the ramp on exit

- **What:** On a clean shutdown (SIGINT, SIGTERM), reset the ramp to linear.
- **Why:** The user should not press Ctrl+C and be left with an orange screen. Redshift does this; so should you.
- **Detail:** Add a `--no-reset` flag — some people want the ramp to persist.
- **Done when:** The screen is normal after Ctrl+C. It is not after `kill -9`, and that is fine — nothing can be done about it.
- **Difficulty:** Medium (signal handling)
- **Depends on:** #11

### #13 Listen for RandR events

- **What:** Subscribe to `RRScreenChangeNotify` and `RRCrtcChangeNotify`.
- **Why:** **The third verified differentiator.** Waking from suspend, changing resolution, plugging or unplugging a monitor, exiting a fullscreen game — all of these silently wipe the ramp. The alternatives don't notice; they recover on their next polling tick, if at all.
- **Verified:** `nm -D /usr/bin/gammastep | grep randr` shows no `xcb_randr_select_input`. It cannot be subscribed to screen events. It also calls `get_screen_resources_current` rather than `get_screen_resources`, so a monitor attached after startup is very likely never seen. See `PRIOR-ART.md`, defect 3.
- **Honest scope:** the suspend half of this is worth a few seconds of neutral screen, not a permanent failure. The monitor hotplug half is the serious one. Don't oversell it in the README.
- **Done when:** Unplugging and replugging a monitor restores the filter without user action, and clearing the ramp by hand with `xrandr --gamma 1:1:1` is corrected in under 100ms rather than on the next tick.
- **Correction latency (measured):** X11 emits no RandR event for a bare gamma write, so it cannot be caught by events alone. The watcher therefore combines both: event-emitting wipes (hotplug, mode change) are corrected immediately; silent wipes (bare gamma writes) are corrected on the next verification tick, worst case 60s. Verified with `xrandr --output DisplayPort-0 --gamma 1:1:1` on one output.
- **Difficulty:** Hard
- **Depends on:** #10

### #14 Handle screens appearing and disappearing

- **What:** Apply the ramp to a newly attached monitor. Drop screens that go away.
- **Why:** Laptop plus external monitor is the most common setup on earth. "It broke three months later when I plugged in a monitor" comes from exactly this.
- **Done when:** Attaching an external monitor leaves both screens at the correct temperature.
- **Difficulty:** Medium
- **Depends on:** #13

---

## M3 — The daemon

### #15 Event loop

- **What:** A single thread waiting on two sources: the X11 socket and a timer. Whichever fires, handle it.
- **Why:** The heart of the daemon. Because it never does two things at once, there are no race conditions to reason about.
- **Detail:** The X11 connection exposes a file descriptor. Create a timer descriptor with `timerfd`. Wait on both with `poll`. Alternatively, the `calloop` crate abstracts this away — or `tokio`, which may make sense if DBus (#18) pushes you toward async anyway. Pick one model and commit to it.
- **Done when:** The program loops forever, wakes on the minute, reacts instantly to X11 events, and uses 0% CPU while idle.
- **Difficulty:** Hard (the hardest part of the project, and the place a first Rust project will hurt)
- **Depends on:** #13

### #16 Suspend/resume signal

- **What:** Listen for `systemd-logind`'s `PrepareForSleep` DBus signal. Rewrite the ramp immediately on resume.
- **Why:** On some drivers no RandR event arrives after waking. This is the safety belt.
- **Done when:** Closing and opening the laptop lid leaves the filter intact.
- **Difficulty:** Medium
- **Depends on:** #15, #18

### #17 Config file

- **What:** `~/.config/nightlightd/config.toml`. Day temperature, night temperature, manual location, transition span.
- **Why:** Settings have to live somewhere. Use TOML — it is the Rust ecosystem's default, and `serde` handles it in five lines.
- **Detail:** **The program must run with no config file at all,** on sensible defaults. A program that requires a config file is a program nobody uses.
- **Done when:** Runs without a config, reads one when present, gives a clear error and falls back to defaults when it's malformed.
- **Difficulty:** Easy
- **Depends on:** #9

---

## M4 — DBus and the client

At the end of this milestone the tool becomes genuinely usable.

### #18 DBus interface

- **What:** Expose a service named `org.nightlightd.Daemon` via `zbus`. Methods: `SetTemperature(u32)`, `Toggle()`, `GetStatus() -> (bool, u32)`, `SetMode(String)`.
- **Why:** The only channel between the client and the daemon. The tray icon, the CLI, and anything you write later all knock on the same door.
- **Done when:** Calling the methods by hand with `busctl --user call ...` works.
- **Difficulty:** Medium
- **Depends on:** #15

### #19 Single-instance lock

- **What:** The daemon claims ownership of the DBus name on startup. If the name is taken, it prints "already running" and exits.
- **Why:** **The most visible bug in every competitor.** Two copies fight over the ramp and the screen flickers. Here it becomes architecturally impossible.
- **Verified:** launching gammastep twice with conflicting targets leaves `pgrep -c gammastep` at 2, and the screen oscillates indefinitely. On a stock Mint Xfce install, four redshift instances had accumulated from three autostart sources that don't know about each other. See `PRIOR-ART.md`, defect 2.
- **Done when:** Running `nightlightd --daemon` twice exits the second cleanly. No flicker.
- **Difficulty:** Easy
- **Depends on:** #18

### #20 CLI client

- **What:** `nightlightd --temp 2800`, `nightlightd --toggle`, `nightlightd --status`, `nightlightd --off`. A clear error when the daemon isn't running.
- **Why:** Scripters love it, and it's the only interface you'll have until the tray icon lands.
- **Detail:** Use `clap`. One binary, two modes: no flag means client, `--daemon` means daemon.
- **Done when:** All four commands work.
- **Difficulty:** Easy
- **Depends on:** #18

### #21 systemd user service

- **What:** Write `nightlightd.service` and ship it with the package.
- **Why:** The correct way to autostart. Dropping a file into `/etc/xdg/autostart/` and praying is precisely what causes redshift's flicker. systemd restarts on crash and gives you `systemctl --user status` when a user reports a problem.
- **Not a differentiator:** gammastep already ships two systemd user units. Do this because it's right, not because it's a selling point. Unlike gammastep, enable them in *user* scope, not global — a globally enabled unit cannot be disabled by the user with `systemctl --user disable`, which is surprising and hostile.
- **Detail:**
  ```ini
  [Unit]
  Description=Screen colour temperature
  PartOf=graphical-session.target

  [Service]
  ExecStart=/usr/bin/nightlightd --daemon
  Restart=on-failure

  [Install]
  WantedBy=graphical-session.target
  ```
- **Done when:** After `systemctl --user enable --now nightlightd`, the daemon starts on every login.
- **Difficulty:** Easy
- **Depends on:** #19

### #22 Structured logging

- **What:** `tracing`, or `log` plus `env_logger`. Error, warn, info, debug.
- **Why:** When a user says "it doesn't work", what do you ask them for? The output of `journalctl --user -u nightlightd`. Without logs you cannot debug anything remotely.
- **Done when:** `RUST_LOG=debug` produces detailed output; the default is quiet.
- **Difficulty:** Easy
- **Depends on:** #15

---

## M5 — The interface

**Deliberately last, because this is Rust's weakest area.** By the time you get here you will already be using the tool daily.

### #23 Tray icon

- **What:** An icon next to the clock. Left click toggles. Right click opens a menu.
- **Why:** The only entry point for people who won't open a terminal.
- **Detail:** Several options exist (`ksni` for StatusNotifierItem, `tray-icon`, or `gtk-rs` directly). None are smooth. XFCE's tray supports StatusNotifierItem. **Prototype first, decide second** — step one of this issue is research, not code.
- **Done when:** The icon appears and responds in the XFCE, MATE and Cinnamon trays.
- **Difficulty:** Hard
- **Depends on:** #20

### #24 Settings window

- **What:** A small window: day temperature, night temperature, transition span, "start at login" checkbox.
- **Why:** Anyone willing to edit a config file is already using the CLI. This window is for everyone else.
- **Detail:** Keep the scope tiny. Five controls and a Save button. No tabs. No advanced section.
- **Done when:** The window opens, settings persist, and the daemon applies them immediately.
- **Difficulty:** Hard
- **Depends on:** #23

### #25 "Start at login" wires up systemd

- **What:** Ticking the box runs `systemctl --user enable nightlightd`.
- **Why:** So the user never sees a terminal.
- **Done when:** Ticking the box means the daemon is running after the next login.
- **Difficulty:** Medium
- **Depends on:** #21, #24

---

## M6 — Packaging and distribution

**When the code is finished, maybe a third of the work is done.** Nobody knows the thing exists.

### #26 .deb package

- **What:** Build with `cargo-deb`, attach to GitHub Releases.
- **Why:** Most Mint/Ubuntu/Debian users want a binary. The reason nobody uses Gammy is that it makes you compile it.
- **Not a differentiator:** gammastep is already in `apt` on every relevant distro. Shipping a `.deb` gets you to parity, not ahead.
- **Done when:** `sudo apt install ./nightlightd.deb` works, and the systemd unit and `.desktop` file land in the right places.
- **Difficulty:** Medium
- **Depends on:** #21

### #27 Flatpak and Flathub

- **What:** Write the Flatpak manifest, submit to Flathub.
- **Why:** **This is the real storefront.** It appears directly inside Mint's Software Manager. No maintainer has to approve you. It is the only genuine discovery channel.
- **Detail:** The sandbox will need X11 and session-bus access (`--socket=x11`, `--socket=session-bus`). A systemd unit cannot be installed from inside a sandbox — this is a real problem, research it early. You may end up falling back on `.desktop` autostart, which is ironic.
- **Done when:** Live on Flathub.
- **Difficulty:** Hard
- **Depends on:** #26

### #28 AUR package

- **What:** Write a `PKGBUILD`, submit to the AUR.
- **Why:** Free, easy, and Arch users are the best early testers you will find. When something breaks they send you the GPU model, the driver version, and the exact error. Most tiling-WM users are on Arch.
- **Done when:** `yay -S nightlightd` works.
- **Difficulty:** Easy
- **Depends on:** #26

### #29 README, screenshot, GIF

- **What:** What it is, what it isn't, how to install it, how to use it. One screenshot. A short section on how it differs from Redshift.
- **Why:** People read a README and decide in three seconds.
- **Detail:** Put "X11 only" at the very top, without apology. List the differences plainly: zero configuration, guaranteed single instance, survives suspend, actually packaged.
- **Done when:** A stranger can read the README and install it.
- **Difficulty:** Easy
- **Depends on:** #26

### #30 Announce

- **What:** r/linux, r/xfce, r/unixporn, the Linux Mint forums, the XFCE forums.
- **Why:** There is a waiting audience. The night light feature request on `xfce4-power-manager` has been open since 2019, with people still subscribed to it. Leave a comment there: "I wrote this; use it until the native version lands."
- **Detail:** Don't oversell on Reddit. "I couldn't get Redshift working so I wrote this" lands far better than "a revolutionary new tool."
- **Done when:** Announced.
- **Difficulty:** Easy
- **Depends on:** #27, #29

---

## M7 — Later (v0.2 and beyond)

Keep all of these **out of v0.1.** Scope creep is what kills projects like this.

### #31 Wayland support (wlroots)

- **What:** The `wlr-gamma-control-unstable-v1` protocol. Sway, Hyprland, river.
- **Why:** X11 has a finite shelf life. But GNOME and KDE's Wayland sessions expose no such protocol, so there is nothing you can do there at all.
- **Difficulty:** Hard
- **Note:** Design it as a separate backend. Because `core` is already clean, this is only a new output layer — the Wayland equivalent of #10–#14.

### #32 ICC colour profile compatibility

- **What:** Compose with the user's colour profile instead of overwriting it.
- **Why:** People who use colour profiles (photographers, designers) cannot run Redshift at all, because it wipes them. A legitimate request. Also an expensive one.
- **Difficulty:** Hard

### #33 NVIDIA proprietary driver quirks

- **What:** "Invalid gamma ramp size" and friends.
- **Why:** This shows up as a footnote even in Gammy's README. Wait for the reports; don't chase it.
- **Difficulty:** Unknown
- **Note:** Prepare an issue template that asks for driver version and `xrandr --verbose` output.

### #34 Per-monitor temperature

- **What:** Different settings for the external monitor.
- **Why:** Someone will ask. But it is needless complexity for v0.1.
- **Difficulty:** Medium

### #35 Brightness control

- **What:** —
- **Why:** **Don't.** This is exactly where Gammy drowned. Do colour. Leave brightness alone. This issue exists so it can be closed.
- **Difficulty:** —

### #36 Deep gamma LUT

- **What:** Investigate whether the modern DRM `GAMMA_LUT` property can be used instead of the legacy `crtc_gamma` path.
- **Why:** `xrandr --verbose` reports `GAMMA_LUT_SIZE: 4096` on this hardware. The legacy path gammastep uses is typically capped far lower. A deeper table means less colour banding at low temperatures — a real, visible quality difference, and a fourth differentiator if it pans out.
- **Difficulty:** Unknown
- **Note:** Measure the banding first. If nobody can see it, don't build it.

---

## Ordering summary

```
M-1 (#0)      →  one week          ← do this before anything else
M0 (#1-3)     →  half a day
M1 (#4-9)     →  one weekend       ← this is where you learn Rust
M2 (#10-14)   →  a few evenings    ← this is where the screen turns warm
M3 (#15-17)   →  one weekend       ← #15 is the hardest issue
M4 (#18-22)   →  a few evenings    ← this is where it becomes usable
─────────────────────────────────── v0.1-alpha, use it yourself
M5 (#23-25)   →  unknown           ← Rust's weak spot
M6 (#26-30)   →  three weeks       ← the real work
─────────────────────────────────── v0.1, announce
M7 (#31-36)   →  don't (yet)
```

**Do #0 before #1.** If gammastep's maintainer tells you defects 2 and 3 are patchable in place, this entire repository is unnecessary and you have saved yourself two months.

**Run gammastep daily until M4 lands.** Using the incumbent every evening is the cheapest research available, and you will find defect 4 by accident.

**Stop after M4 and use your own tool for a week.** You cannot ship something you don't use.

**Freeze after M6.** Bug fixes only. No new features. An abandoned repo looks worse than one that was never written.
