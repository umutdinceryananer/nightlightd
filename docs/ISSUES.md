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

**Done.** Opened as [chinstrap/gammastep!28](https://gitlab.com/chinstrap/gammastep/-/merge_requests/28)
on 2026-07-10, before the Rust port of the same logic landed in `core`.
Awaiting review since; upstream has merged nothing from anyone since
March 2025, so no conclusion can be drawn from the silence. What the
attempt revealed — the provider chain never advances past a hanging
geoclue2, and merge-request pipelines are broken for every new
contributor — is recorded in `PRIOR-ART.md`, "Upstream attempt".

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
- **Done when:** Unplugging and replugging a monitor restores the filter without user action, and clearing the ramp by hand with `xrandr --output <out> --gamma 1:1:1` is corrected by the next tick (there is no event to react to for a bare gamma write — see the measured note below; #40 will shrink the window).
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
- **Shipped (2026-07-16):** `tray/` crate via `ksni` (SNI, pure Rust) — the
  research step measured XFCE's own systray already hosting
  StatusNotifierWatcher, ruling out GTK/appindicator. Verified on XFCE; MATE
  and Cinnamon still untested.

### #24 Settings window

- **What:** A small window: day temperature, night temperature, transition span, "start at login" checkbox.
- **Why:** Anyone willing to edit a config file is already using the CLI. This window is for everyone else.
- **Detail:** Keep the scope tiny. Five controls and a Save button. No tabs. No advanced section.
- **Done when:** The window opens, settings persist, and the daemon applies them immediately.
- **Difficulty:** Hard
- **Depends on:** #23
- **Shipped (2026-07-17):** `panel/` crate as an **egui** window, not GTK —
  pure Rust, no `libgtk-4-dev` to install, and its canvas draws the f.lux-style
  day/night curve. Changes apply live over D-Bus and persist via the daemon; no
  Save button needed. A "transition span" control was not built (the curve
  follows solar elevation, redshift-style).

**Done (2026-08-09).** "No tabs" did not survive contact with the
window. One scrolling column of every control the daemon has said
nothing about which of them mattered now, and it was the least useful
of the three interfaces despite having the most room. It is now the
dashboard's shape: a strip that always says whether the filter is on, a
state card leading with the kelvin the screen is actually at, and five
tabs — now, today, location, outputs, settings. Nothing scrolls; each
tab is sized to fit and the chart absorbs whatever the frame measures
as overflow, so the window cannot cut off a control it forgot to
count.

The scope note above was right about small windows and wrong about
this one: the controls were never the problem, the absence of a
schedule, a map and a readout was. What arrived with the tabs was a
world map that takes a pin (drawn from a coastline committed to this
repository, never fetched), the day's milestones with the next one
lit, how long the day is and how that compares with yesterday, the
screens the ramp is reaching, and the eight themes the dashboard
carries — each window remembering its own.

The "transition span" control finally exists, twice: as the curve's
own draggable ramp (#45) and as the band the dashboard edits (#39).

### #25 "Start at login" wires up systemd

- **What:** Ticking the box runs `systemctl --user enable nightlightd`.
- **Why:** So the user never sees a terminal.
- **Done when:** Ticking the box means the daemon is running after the next login.
- **Difficulty:** Medium
- **Depends on:** #21, #24
- **Shipped (2026-07-17):** a checkbox in the panel; it re-reads systemd's
  actual answer after acting, so a failed enable shows as unticked.

### #35 TUI client (`nightlight-tui`, ratatui)

- **What:** A fifth crate: a ratatui terminal dashboard speaking the same D-Bus
  interface — the day/night curve drawn in braille, live kelvin + sun elevation,
  keybindings (`t` toggle, `a` auto, arrows nudge the night temperature).
- **Why:** The showcase piece for #30. Terminal screenshots and GIFs travel:
  ratatui has a curated `awesome-ratatui` list and an active showcase community,
  and a polished TUI is the most shareable artifact this project can produce.
- **Positioning:** SunReactor (a sun-driven *brightness* daemon) already ships a
  ratatui TUI. The overlap is cosmetic, not functional — we drive colour, it
  drives the backlight; zero collision in what the two tools touch. But any
  announce must say "colour temperature, not brightness" in its first line.
- **Detail:** Thin client only, like the tray and panel: no state, reads
  `GetStatus`, sends the same methods the tray does. It re-declares `Status` a
  *fourth* time, so the signature-pin test (AUDIT M10) must be copied in. It
  should land after the daemon-side "auto also enables" fix (AUDIT M1) so it
  needs no client-side compensation logic.
- **Done when:** `nightlight-tui` renders the live curve and controls the
  daemon; a GIF of it is in the README; submitted to `awesome-ratatui`.
- **Difficulty:** Medium
- **Depends on:** #18–#20, AUDIT P1

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

**Done.** GitHub topics, the GitLab comment on xfce4-settings #111,
r/xfce, r/unixporn, awesome-ratatui (merged), and the ratatui
showcase issue. Ratatui's account and its author shared the project
on LinkedIn unprompted. r/linux bounced off a karma automod and may
be retried later; the Mint forum and Discord were skipped, their
audiences already reached through the channels above.

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
- **Design note (2026-08):** QRedshift compiles its wlr support as a
  dlopen'd plugin so the X11 build links no Wayland libraries, and
  picks its backend by checking the Wayland socket actually exists
  rather than trusting the environment. Both choices are worth
  copying; the second avoids gammastep's wrong-backend failure.

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
  v0.2 went to ramp shaping (GitHub #2) instead. Both the dashboard's
  and the panel's outputs tabs now point here — without naming a
  version, because the first one named came and went.
- **Difficulty:** Medium
- **Design note (2026-08):** QRedshift 1.0 shipped exactly this, keyed
  by RandR output XID and name rather than list position, so a target
  survives replug and reboot. That is the right design and ours should
  match it. Its pipelined XCB writes (batch the gets, batch the sets,
  sync once) are the pattern for touching many CRTCs cheaply.
- **Wire note (2026-08):** #44's fade switch reads through an additive
  `GetFade` method so a patch release does not grow the pinned Status
  signature. That is a parking spot, not an address. When this issue
  reworks the wire anyway, fold `fade` into Status, retire the side
  getter, and ship all four clients together. This line is the
  promise; do not close #34 without honouring it.
- **The parking spot filled up (2026-08):** #39 parked
  `GetTransitionBand` beside `GetFade` on the same reasoning, and the
  bill is now visible. A dashboard poll is five round trips a second —
  `GetStatus`, `GetOutputs`, `GetFade`, `GetTransitionBand`, and
  `NameHasOwner` for #42 — times however many clients are open. Worse
  than the cost is the seam: a client can read a status and a band
  that disagree, because they arrived separately. Two fields fold into
  Status when this lands. A third would be one too many; if something
  else needs the wire before #34 is ready, do the consolidation first.
  #47 and #49 are both queued behind exactly that: one turns the band
  into four numbers, the other into a list. Neither should be parked
  beside the other two.

### #37 Brightness control

**Done.** Shipped in v0.2.0 as part of the ramp shaping work
(GitHub #2): a day and a night bound easing on the sun's own curve,
set from config, D-Bus, the CLI, and now every interface. The fence
below stands unchanged — nothing adaptive, ever.

- **What:** A static multiplier only, folded into ramp shaping with gamma
  — tracked as GitHub #2, target v0.2, requested by the first daily
  driver (a monitor with no OSD controls).
- **Why:** The original ban was really about *adaptive* brightness:
  screen sampling, feedback loops, DDC. That stays banned forever. A
  fixed factor from config shares its code path with gamma, so refusing
  one while shipping the other would be arbitrary.
- **Difficulty:** Easy (rides GitHub #2)

### #36 Deep gamma LUT

- **What:** Investigate whether the modern DRM `GAMMA_LUT` property can be used instead of the legacy `crtc_gamma` path.
- **Why:** `xrandr --verbose` reports `GAMMA_LUT_SIZE: 4096` on this hardware. The legacy path gammastep uses is typically capped far lower. A deeper table means less colour banding at low temperatures — a real, visible quality difference, and a fourth differentiator if it pans out.
- **Difficulty:** Unknown
- **Note:** Measure the banding first. If nobody can see it, don't build it.

### #38 Fade between targets

**Done.** Shipped in four slices ending 2026-08-02: pure blend maths in
`core/fade.rs`, a time-based state machine in `cli/fade.rs`, the event
loop wiring, then mired-space walking at a 50 ms tick after the first
live test showed visible stepping on a 6500 to 1500 span. Retargeting
mid walk continues from the point reached. Exit restore and resume
repair stay single writes on purpose. Verified live on the machine
that asked for it.

- **What:** When the applied target changes by more than a small threshold
  (toggle, manual set, daemon start, resume), walk the ramp there over a
  few seconds of intermediate writes instead of one write. The event
  loop's timer already exists; a short-lived fast tick drives the walk.
- **Why:** Both Reddit reviews of v0.2.0 called out the hard switch,
  independently. f.lux animates every change. redshift and gammastep fade
  on start and stop, so users arriving from them expect it (gammastep's
  own MR queue has a draft about its quit fade). The per-minute twilight
  steps are also visible at aggressive night temperatures, where 5000 K
  of span means roughly 125 K per tick.
- **Detail:** Static easing with a fixed duration. Nothing adaptive. The
  restore on exit may ease briefly but must always complete.
- **Done when:** Toggling the filter at night eases over a couple of
  seconds instead of snapping, and `--temp` changes do the same.
- **Difficulty:** Medium (core loop change)
- **Target:** v0.2.1

### #39 Configurable transition band

**Done.** Shipped in five slices ending 2026-08-06: the band as a
parameter through core, config and state carrying the pair verbatim,
`SetTransitionBand`/`GetTransitionBand` on the bus and `--band
DAY:NIGHT` at the door, then the clients drawing the schedule from the
daemon's band instead of their built-in constants. The now tab's
square wave opens into a staircase when the band is wide, and the dot
reads its row off the drawn line rather than recomputing it. A silly
pair (inverted, non-finite) degrades to the default where it is spent,
never where it is stored, so what the user wrote survives in the
config. Verified live twice against `--band 3:-25`, at dawn by
arithmetic and at dusk by screenshot.

**The tail it left (2026-08-08).** Making the band configurable turned
every hardcoded `+3` and `-6` into a potential lie, and two of them
survived the five slices. The first was caught by eye: the schedule
drew from constants while the dot drew from the daemon, which put the
dot in mid-air. The second was caught by audit and is the same defect
one layer up — the *word* beside the sun's angle. `sun_phase` had been
hand-copied into the CLI, the tray and the dashboard, all three with
the old pair inside, so a daemon reporting `night below -14.0°` and a
sun at `-12.9°` printed `(night)` over a screen that was still
warming. It now lives once, in `core::transition::phase`, derived from
the same alpha the temperature is, so the word cannot disagree with
the colour. The tray took a dependency on `core` to get it; core
carries no dependencies of its own, so that costs the tray nothing and
ends the fourth copy. **The lesson worth keeping: when a constant
becomes a setting, grep for the constant, not for the feature.**

- **What:** Expose the transition band's elevation bounds in the config
  (`day_elevation`, `night_elevation`, today fixed at +3 and -6).
  Lowering the night bound to -12 holds daylight longer and lands the
  full night value deeper into dusk.
- **Why:** Asked for on the v0.2.0 announcement thread. The sky stays
  bright for a while after sunset and eyes adapt slowly, so one fixed
  band cannot suit everyone. Elevation stays the right axis (it adapts
  to latitude and season where clock offsets do not), but the endpoints
  need not be constants.
- **Done when:** The pair round-trips through the config, an inverted
  pair degrades to the defaults quietly, and the TUI's schedule reflects
  the configured band.
- **Difficulty:** Easy
- **Target:** v0.3

### #40 Verify the ramp on the tick

- **What:** On each timer tick, read the live ramp and compare before
  writing. Skip the write when nothing changed; rewrite when something
  else wiped it. QRedshift calls this a smart delta write. For a
  stateless tool it is the whole recovery story; for us it is a cheap
  safety net under the RandR events (#13), catching any wipe that
  fires no event.
- **Why:** The events cover suspend, hotplug and resolution changes,
  but nothing guarantees every driver reports every wipe. A poll that
  only writes on difference costs one read per minute and turns the
  worst case from "wiped until the next event" into "wiped for under
  a minute".
- **Done when:** `xrandr --output <out> --gamma 1:1:1` by hand is
  corrected on the next tick, and an unchanged minute produces no set
  call (visible at debug log level). The bare `--gamma` form this line
  used to carry does not run: xrandr wants the output first.
- **Difficulty:** Easy
- **Target:** v0.3

**Done.** The tick reads each CRTC's ramp before writing it, and the
comparison is a three-way answer rather than a delta: already ours
(skip, debug), a difference we asked for (write, the existing log), or
a difference nobody asked for on a screen we had already written to
(write, and say so at info — the only line that shows the safety net
catching something).

The two rows that mattered are the ones a plain "write only on
difference" would have got wrong, and loudly. A fade moves the ramp
every step while deliberately staying quiet in the log, so `changed`
is false throughout — reading a wipe off that flag would have cried
wolf on every transition. And a daemon starting on a screen somebody
else had left coloured would have called its own first apply a wipe.
So the predicate is "did we ask for this" (`target != applied`) gated
on having written here before, not "is it different"; `classify` holds
the table and a test walks all four rows.

A failed gamma read counts as a mismatch and falls through to the
write: the worst an unnecessary write costs is a round trip, the worst
a skipped one costs is a screen left wrong.

### #41 Temperatures past neutral

**Done.** Shipped 2026-08-15, and the shape of it was not what the issue
assumed. The daemon never had a ceiling — `day_temp = 8000` in the config
applied before any of this was written, and `SetDayTemp` only ever held the
pair in order. Every clamp was in an interface, in five places, each with
6500 written into it by hand. So the work was to publish one range from
core (`UI_TEMPERATURE_RANGE`, 1500–10000 K) and have all of them lean on it.

10000 rather than the 25000 the issue named. The table can draw to 25000
and the config and the wire still accept that far, but 6500 K to 10000 K
takes red from 1.00 to 0.79, a step anyone can see, while 10000 K to
25000 K spends another 15000 K moving it 0.16 — a control reaching that far
would put everything worth choosing in its first fifth.

Two things the sweep turned up that the issue did not ask about. The
panel's "Hold at" slider stopped at neutral, so anyone on a bluish day
would have read 6500 K on it while the screen wore 8000. And the daemon
reported a hand-written `day_temp = 90000` as 90000 all day while quietly
applying 25000 — reproduced live, then closed by holding what the *state*
accepts to what the table can render, at both doors, leaving the file
itself verbatim.

The first pass closed only half the hand-written hole: it held each bound's
*magnitude* and never checked the pair's *order*, which only a hand-edited
file can cross — the setters have kept it since AUDIT M4. Run as written,
`night_temp = 12000` (a typo for 1200) is a schedule in reverse: the screen
turns bluish at night, silently. A crossed pair now drops to the defaults
whole, the same answer `Band::sane()` gives a nonsense elevation pair (#39)
— keeping either half would be a guess about which line holds the typo —
with one warning in the log and the file left as its author wrote it.
Verified live: day 6500 / night 12000 in the file ran as 6500/4500.

The two charts that draw against a fixed vertical scale — the panel's curve
axis and the dashboard's settings rails — grow only when a day bound above
neutral needs the room, so nobody still on 6500 watches their picture shrink
to make space for a range they never use. The curve's axis is fed the bound
the *daemon* holds, never the one a drag is proposing, or it would rescale
under the hand mid-gesture. The live accent deliberately does not follow
past neutral: that colour answers "how warm is the screen", and above 6500 K
the answer is "not at all".

Verified live: 8000 K from the config applied and survived a restart, 9000 K
over D-Bus applied and persisted, 90000 K clamped to 25000 at both doors.

- **What:** Raise the ceiling above 6500 K, bluish instead of warm, in
  the config, the D-Bus door and the sliders. QRedshift accepts 25000;
  redshift's table, which ours is ported from, runs to 25100. Core
  already computes it (`MAX_TEMPERATURE` is 25000); only the interface
  clamps say no.
- **Why:** Some people run a cool screen by day and warm by night.
  The maths is already paid for.
- **Done when:** `day_temp = 8000` applies, survives a restart, and
  every client shows and sets it.
- **Difficulty:** Easy
- **Target:** v0.3, shipped in v0.3.1

### #42 A version mismatch must not impersonate a stopped daemon

**Done.** Shipped 2026-08-04, and it grew: every surface asks the bus
whether the name is owned when the status is unreadable, the tray and
the panel silently re-exec themselves once (healing a process older
than its own file), and what remains shows a short update needed
notice with a restart action beside it — a button in the panel, a
menu item in the tray, the r key in the TUI, words in the CLI. The
daemon is never restarted automatically by a client. Verified against
a live v0.1.2 daemon, including the harmless restart while the disk
was old and the healing one the moment the new binary landed.

- **What:** When `GetStatus` fails to deserialize, every client falls
  into its "daemon not running" state. The tray then shows the
  disabled icon while the screen is visibly warm, and its menu offers
  Turn on, which sends `set_enabled` to a daemon that is already on
  and so appears dead. On the failure path, ask the bus whether
  `org.nightlightd.Daemon` has an owner. Owned but unreadable means
  this client and the daemon are different versions; the tooltip and
  the TUI banner should say so, and say the fix, update them together.
- **Why:** Found by dogfooding on 2026-08-01. A source install had
  updated the daemon and the TUI but left the tray weeks behind; the
  status format grew in v0.2.0 and the stale tray reported the daemon
  as off. Packages carry all four binaries together, but a running
  session keeps its old client processes in memory until the next
  login, so every wire-growing upgrade opens the same window for
  every user.
- **Done when:** An old tray against a new daemon shows a mismatch
  message instead of "daemon not running", and the panel and the TUI
  do the same.
- **Difficulty:** Easy
- **Target:** v0.2.1

### #43 The tray should offer to start a stopped daemon

**Done.** Shipped 2026-08-04. With nothing on the bus the tray's menu
shrinks to Start the daemon, Settings and Quit, and a left click on
the struck icon starts one too; the TUI's banner offers the same
through r. systemd first, a direct spawn beside the binary when the
unit is absent. Verified live: service stopped, one click, the filter
faded back in.

- **What:** When nothing owns the bus name, the tray's menu still
  shows Turn on, which sends a D-Bus call to nobody. In that state
  the menu should offer "Start the daemon" instead, running
  `systemctl --user start nightlightd` and falling back to spawning
  `nightlightd --daemon` when the unit is not installed.
- **Why:** Found minutes after #42, again by dogfooding: daemon
  stopped during a test, the tray truthfully said so, and its only
  offered action was inert. A thin client cannot be the daemon, but
  it can ask systemd for one.
- **Done when:** With the service stopped, the tray offers Start the
  daemon; clicking it brings the filter up within seconds and the
  menu returns to Turn on/off.
- **Difficulty:** Easy
- **Target:** v0.2.1

### #44 A switch for the fade

**Done.** Shipped 2026-08-03 in four slices: config/state/import, the
additive `SetFade`/`GetFade` pair plus `--fade on|off`, the loop
honouring the switch (a walk in flight dies on the next wake), and
the three interfaces, each hiding the control against a daemon that
cannot answer for it. Verified live: instant with it off, eased with
it on, one surface's toggle following in the others within a poll.

- **What:** `fade = true` in the config, default on; off means every
  target change lands in one write, as before #38. Exposed in the
  TUI's settings tab, the tray menu and the panel, through additive
  `SetFade` and `GetFade` methods so the pinned Status signature does
  not grow in a patch release (the consolidation promise lives under
  #34). The import translates gammastep's own `fade` key, so a
  hand-written `fade=0` arrives here still off.
- **Why:** Asked for after a day of living with #38. redshift and
  gammastep carry the same switch, so arriving users expect it, and
  some people simply do not want animation.
- **Done when:** Toggling from any interface changes the behaviour
  live, survives a restart, and an imported gammastep config with
  `fade=0` starts with it off.
- **Difficulty:** Easy
- **Target:** v0.2.1

### #45 Drag the curve's incline

- **What:** Grab the transition edge of the day/night curve and drag
  it. The incline is the transition band, so this is #39 wearing an
  interface: dragging widens or narrows the elevation window the
  daemon eases across, instead of asking anyone to think in degrees.
- **Why:** Asked for right after v0.2.1 shipped, and it completes the
  #39 story — a hand on the curve beats a pair of numeric fields for
  a quantity nobody has intuition for.
- **Detail:** Panel first; egui already owns the pointer over the
  painted curve. The dashboard needs mouse capture from crossterm, a
  whole new input surface, so it starts with keys on a settings row
  and earns the mouse later. The whole line ended up a handle, not
  just the incline: the plateaus are the day and night temperatures
  and drag vertically. That forced the curve's vertical axis to stop
  fitting itself to the current bounds — a self-scaling axis draws
  every pair as the same picture, leaving a plateau nowhere to be
  dragged to. Edits stage behind Apply and Revert rather than going
  out per frame, because a drag crosses hundreds of values on its
  way to the one that was meant.
- **Done when:** Dragging the edge in the panel visibly reshapes the
  curve, changes when full night lands, survives a daemon restart,
  and the dashboard's schedule shows the same band.
- **Test debt, paid (2026-08):** the rules that keep a drag or a
  keypress from producing nonsense were buried inside the widget code
  and so untested. They are pure functions now — `nudged_band` in the
  dashboard, `held_band` and the two plateau clamps in the panel — and
  each carries a test: the pair cannot cross however long a key is
  held, a bound cannot leave the window its rail draws, a ramp dragged
  across all 24 hours never hands the daemon a band it would have to
  repair, and `BandEdit::touched` is about difference rather than
  history, so a draft walked back to where it started asks nothing on
  escape. What is left needing eyes is only what eyes are for: whether
  the thing looks right and feels right under the hand.
- **Difficulty:** Medium
- **Depends on:** #39
- **Target:** v0.3

**Done.** The whole line is a handle in the panel: the ramps set the
band, the plateaus set the two temperatures, and each end moves without
the other. Changes stage rather than fire — Apply and Revert appear the
moment the shape differs from the daemon's — because a drag that sent
on every frame would have written the config file forty times crossing
the window. The dashboard has the same band on `b`, with the keys it
already speaks; the mouse is still #49's problem.

What it taught us is filed: the two ramps read their bounds off one
pair, so dragging dusk moves dawn with it (#47), and once the ramp is
something you hold, its straightness is the next thing you notice
(#49).

### #46 A nicer tray menu

**Done.** Shipped 2026-08-08. Three blocks in a fixed order — what is
happening, what you can do, the application — so the menu keeps one
shape whatever the daemon is doing. A readout line on top carrying the
applied temperature and the sun's phase, and the band under it when it
is not the default, both unclickable and `Informative`. `Disposition`
turned out to be the one lever the protocol gives for how a line
should feel: a stopped daemon reads `Warning`, a version mismatch
`Alert`. Start and restart collapsed into one slot that changes label
and icon. Controls unreachable during a mismatch are disabled rather
than hidden, since the line above says why; the fade switch stays
hidden against a daemon that has never heard of it, because a greyed
checkbox there invites a click that can never work. The readout's
words are a pure function with tests — a `MenuItem` carries closures
and cannot be read back.

- **What:** The menu grew items (#42, #43, #44) faster than it grew
  looks. The levers SNI actually offers: an icon per item, a readout
  line wearing the current temperature and sun phase, grouping with
  separators, disabled states instead of vanishing items.
- **Why:** Asked for after v0.2.1. First impressions live here — the
  tray is the surface most people meet first.
- **Detail:** Honest ceiling, stated up front: the menu is rendered
  by the desktop's own panel, so fonts, colours and spacing belong to
  the GTK theme and cannot be touched from here. Where the ceiling
  disappoints, the answer is the panel, one click away.
- **The band belongs in the readout (2026-08):** the tray now polls
  `GetTransitionBand`, but only so its tooltip can name the sun's
  phase honestly (#39's tail). Nothing in the menu says what the band
  is, and a tray is the wrong place to edit one. The readout line
  this issue is already adding is the right home for it: the applied
  temperature, the sun phase, and — when the band is not the default
  — the pair, the same "earns a line" rule `--status` uses.
- **Difficulty:** Easy
- **Target:** v0.3

### #47 Dawn and dusk as separate bands

- **What:** Let the morning transition and the evening one be set
  apart from each other, instead of sharing the one band #39 added.
  Four elevations where there are two today.
- **Why:** Found by dragging #45's curve: moving the evening ramp
  moves the morning ramp with it, because they are not two things.
  The sun crosses -6 degrees once on the way up and once on the way
  down, and the curve reads both crossings off the same pair. Wanting
  them apart is a real preference — warm slowly into the evening,
  snap back quickly at breakfast — and f.lux has always allowed it.
- **Detail:** This is an architecture change, not a widget. Core's
  `target_temperature(elevation, band, ...)` is a pure function of
  the sun's angle: it cannot tell morning from evening, which is
  exactly why it needs no clock, tests without a display, and lets
  every client draw the schedule by sampling. Separate bands mean
  the function must also be told whether the sun is rising, and two
  numbers become four in the config, on the bus and in all four
  clients. Note that the direction is cheap to derive (the hour
  angle's sign) and can be passed in rather than computed from a
  clock inside core, which is the version of this that keeps core
  pure. Land it with #34's wire consolidation: both change the
  contract, and one breaking pass is cheaper than two.
- **Done when:** An evening band and a morning band can differ, a
  config carrying only the old pair still behaves exactly as it does
  today, and dragging one ramp in the panel leaves the other alone.
- **Difficulty:** Medium
- **Depends on:** #39, #45
- **Target:** v0.3

### #48 The band, readable and undoable

**Done.** Shipped 2026-08-08 with #45 still warm. The panel carries a
line under its curve naming the pair, silent at the default; beside it
a `Default band` button. The dashboard's `b` editor gets `d`, which
fills the draft rather than sending, so returning is a change like any
other and can be escaped. Both ends tested: pressing it on a band
already at the default leaves nothing to confirm.

- **What:** Two small gaps #45 left. The panel shows the band only
  while a ramp is under the pointer, so a band set from the dashboard
  or the config is a shape with no number anywhere in the window. And
  nothing anywhere puts it back: `--band 3:-6` is the only road home
  from a band dragged somewhere regrettable.
- **Why:** Found by using it. A setting you can reach from three
  places and read from one is a setting people will be unsure they
  changed. The undo matters more than it sounds — the drag and the
  arrows both clamp to a minimum band width, so it is easy to pinch
  the transition down to half a degree, which is a hard switch, which
  is the exact complaint #38 and #39 exist to answer.
- **Detail:** A quiet line under the panel's curve carrying the pair,
  by the same "earns a line" rule as `--status`: silence at the
  default, a reading otherwise. For the undo, the smallest honest
  thing is a "default band" entry wherever the band is edited — the
  panel's line and the dashboard's `b` panel — rather than a general
  reset that would raise the question of what else it resets.
- **Done when:** A band set in the dashboard is readable in the panel
  without touching anything, and one action in either returns the
  screen to +3 / -6.
- **Difficulty:** Easy
- **Depends on:** #45
- **Target:** v0.3

### #49 Shape the ramp, not just its ends

- **What:** Handles *inside* the transition, two to four of them, so
  the ramp is a shape the user draws rather than a straight line
  between two bounds. Today #39 says when the transition starts and
  how wide it is; nothing says what it does in between, because in
  between is a division. Someone who wants dusk to start gently and
  finish quickly, or to hold a landing halfway down and then drop,
  has no way to say so.
- **Why:** Asked for while using #45's drag. Once the ramp is a thing
  you take hold of, its straightness is the next thing you notice.
- **Detail:** The points must be `(elevation, fraction)`, not
  `(elevation, kelvin)`, and this is the whole design. Three things
  ride `daylight_alpha` — the temperature, the brightness bounds
  (GitHub #2) and the phase word — precisely so they move as one. A
  curve in kelvin would shape the colour and leave the brightness on
  the old straight line, which is the coupling this project has kept
  on purpose since v0.2. Store fractions, draw kelvin.
  The rest follows. `daylight_alpha` stops being a division and
  becomes a walk along a piecewise curve. The current band is the
  two-point case — `[(night, 0.0), (day, 1.0)]` — so #39 is not
  replaced, it is the degenerate form, and a config carrying only
  `day_elevation` and `night_elevation` keeps meaning exactly what it
  means today. `Band::sane` grows into the same guarantee for a list:
  elevations strictly increasing, fractions non-decreasing, at least
  two points, anything else quietly the straight line. Non-decreasing
  matters more than it sounds — a curve that dips would warm the
  screen, cool it, then warm it again through one dusk.
  Drawing is already free: both charts sample the curve rather than
  assume its shape, which is what #45's staircase work bought. The
  work is editing (N handles in the panel, N rows behind the
  dashboard's `b`, which has no mouse) and the wire, where a list is
  the third thing that would want a side getter.
- **Ordering:** After #34's consolidation, and designed together with
  #47. All three change the same contract, and #47 decides whether
  this is one curve or two — do it in the other order and dawn and
  dusk each need their own list bolted on afterwards.
- **The risk, stated plainly:** this is the feature that turns a night
  light into a curve editor. The defence is that nobody should ever
  have to open it: the two-point default is redshift's band and stays
  the shipped behaviour, and the handles are something you find only
  if you go looking.
- **Done when:** A dusk ramp can be bent to start slowly and finish
  quickly, the shape survives a daemon restart, every client draws
  the same curve, and a config written before this existed behaves
  identically.
- **Difficulty:** Hard
- **Depends on:** #34, #45, #47
- **Target:** v0.3 or later

### #50 Interfaces you did not ask for

- **What:** Stop making one install carry all four binaries. Split the
  `.deb` into `nightlightd` (daemon, CLI, systemd unit),
  `nightlightd-tui` (the dashboard) and `nightlightd-gui` (the panel
  and the tray, plus the autostart entry).
- **Why:** `cli/Cargo.toml` declares, for everybody:

  ```
  depends = "$auto, libgl1, libegl1, libxkbcommon0, libxkbcommon-x11-0,
             libx11-6, libxcursor1, libxrandr2, libxi6"
  ```

  Eight libraries, every one of them there because eframe and winit
  `dlopen` them for the *panel*. They cannot be discovered by `ldd`,
  which is why they are hand-written. So today someone who wants only
  the daemon, or only the terminal dashboard, drags in the whole GL
  stack — and a Debian package's dependencies are declared per package,
  so a single package can only declare the union.
- **Not the AUR, or not yet.** Arch supports split packages from one
  PKGBUILD, but the win there is mostly cosmetic: an Arch desktop
  already has the GL stack, and the minority without it builds from
  source anyway. What a split would triple is the maintenance surface
  of a package that has already proved easy to leave behind — 0.2.1
  never reached it and it served 0.2.0 for a month. It is current as
  of 0.3.0, and `scripts/aur-push.sh` is what keeps it that way; do
  not add package names until that has held for a release or two.
- **Detail:** `cargo deb` does this with
  `[package.metadata.deb.variants]` — three invocations in the release
  workflow instead of one, each with its own `depends`. The GUI and TUI
  packages `Depends: nightlightd`, so the daemon arrives either way.
  The musl tarball already ships daemon + dashboard only, which is
  half of this story shipping today without anyone being told.
- **Done when:** `apt install nightlightd` on a headless box installs
  no GL library, and `apt install nightlightd-gui` still gets a working
  panel on a minimal Mint.
- **Difficulty:** Medium
- **Target:** v0.4

### #51 Photograph the tray menu

- **What:** A screenshot of the tray's menu (#46) for the README. It is
  the only interface with no picture at all, and the first thing a
  desktop user sees.
- **Why it is not done:** Attempted for 0.3.0 and abandoned, which is
  worth writing down so the next attempt starts further along. A
  StatusNotifierItem has no window: the icon is drawn inside the panel
  by its systray and so is the menu, so there is nothing for `wmctrl`
  to raise, nothing for `xdotool search` to find, and the open menu is
  override-redirect — absent from `xwininfo -root -children`. Asking
  the item over D-Bus does not help either, because `ContextMenu` tells
  the application and the application is not what draws it.
- **What did work:** finding the icon by taking it away — one frame of
  the panel with the tray running and one without, and the only thing
  that differs is our own icon. `scripts/screenshots.sh` keeps that
  (`tray_icon`), and it is exact.
- **What did not:** cropping the menu out of the frame by diffing
  against a shot with it closed. The menu is dark and so is what is
  behind it, so its upper rows — the readout line, which is the whole
  point — fall under any threshold that excludes the panel clock
  ticking. The one run that captured the whole menu only did so because
  a tooltip was sitting on top of it.
- **Also:** right-clicking the icon toggles the filter. xfce4-panel's
  systray delivers Activate on the right button as well as the left,
  so the shot turns the screen neutral on its way past. The script puts
  the state back while the menu is still up, and dbusmenu updates live,
  so the photograph shows the truth — but any future approach has to
  keep doing that.
- **Ideas for next time:** ask the compositor for the override-redirect
  window (`xwininfo -root -tree` rather than `-children`), or drive a
  second systray host in a nested X server where the geometry is known.
- **Difficulty:** Easy to describe, fiddly to do
- **Target:** v0.4

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
M7 (#31-37)   →  don't (yet)
```

**Do #0 before #1.** If gammastep's maintainer tells you defects 2 and 3 are patchable in place, this entire repository is unnecessary and you have saved yourself two months.

**Run gammastep daily until M4 lands.** Using the incumbent every evening is the cheapest research available, and you will find defect 4 by accident.

**Stop after M4 and use your own tool for a week.** You cannot ship something you don't use.

**Freeze after M6.** Bug fixes only. No new features. An abandoned repo looks worse than one that was never written.
