# Audit — 2026-07-18

Full-codebase audit at v0.1-alpha, after M5 (tray + panel) and the first M6 step
(.deb). Method: 8 parallel reviewers, one per dimension (daemon state machine,
event loop, D-Bus wire contract, panel, tray, config, core maths, packaging),
findings deduplicated, then each medium/high finding adversarially verified by
3 independent reviewers with different lenses (code-trace, design-intent,
user-impact); kept only on a ≥2/3 vote. Two findings were additionally
reproduced live against the running daemon before the sweep. **Zero findings
were refuted** — everything below survived its jury.

Severity: **high** = a user hits it and the product promise breaks;
**medium** = real defect in a plausible path; **low** = latent risk, noted.

---

## High — these can strand a user

### H1. A single X error during reapply kills the daemon and skips restore
`cli/src/x11.rs` (active_crtcs / write_ramps / daemon_loop) → `cli/src/main.rs` (fail).
Unplug a monitor whose CRTC XIDs are destroyed (DisplayLink/evdi, DP-MST, eGPU):
between fetching screen resources and the per-CRTC round trips the server can
return BadCrtc; the `?` chain exits the process **without running restore()**,
leaving a warm screen and no daemon. Violates "degrade quietly" and #13's
acceptance criterion (survive a real unplug). **Fix:** per-apply errors are
logged and retried next tick; only connection loss is fatal.

### H2. `Restart=on-failure` hits systemd's start limit in ~0.5 s at login
`dist/nightlightd.service`, `dist/deb/nightlightd.service`.
At login, default.target can be reached before DISPLAY is imported; the daemon
dies instantly, systemd restarts it with RestartSec=100ms, burns
StartLimitBurst=5 in ~half a second, and the unit lands in permanent
`start-limit-hit` — the comment "retries until the X server is up" is wrong.
Our own machine survived on attempt 2 by luck. **Fix:** `RestartSec=2s` +
`StartLimitIntervalSec=0` in both unit variants.

### H3. Panel opened during a manual override shows 2800 K, not the override
`panel/src/main.rs`. The warm slider is only mirrored while `following`; with
an active override the slider sits at its compile-time default (2800) while the
screen is at, say, 4250. First glance says the tool is lying. **Fix:** seed the
slider from `status.temperature` on first status, regardless of mode.

---

## Medium — confirmed defects

**Semantics / state machine**
- **M1.** CLI `--auto` is a silent no-op when the filter is off: it sends only
  SetMode("auto"), which never enables. Both GUI clients compensate by pairing
  SetEnabled(true) — same intent, three behaviours. **Fix:** make the daemon's
  SetMode("auto") also enable; delete the client-side compensation.
- **M2.** SetMode("auto") replaces `Mode::ManualLocation` with `Automatic`,
  discarding configured coordinates for the session. *(reproduced live)*
- **M3.** persist() then writes that state back, **deleting `latitude`/
  `longitude` from config.toml permanently.** *(reproduced live)* **Fix for
  M2+M3:** keep the configured mode in State; "auto" returns to it; persist
  preserves the configured coordinates.
- **M4.** SetDayTemp/SetNightTemp accept and persist an inverted band
  (day < night); the panel's slider ranges overlap (4000–6500 vs 1500–4500), so
  it is reachable by drag. **Fix:** clamp in the daemon so day ≥ night holds.
- **M5.** Unticking the tray's "Automatic" acts on status up to 5 s stale — it
  can pin an outdated temperature, or re-enable a filter turned off moments ago
  from another surface. **Fix:** refresh status before acting on a click.
- **M6.** Tray menu says "Turn off"/"Turn on" but sends directionless Toggle —
  with stale status the click does the opposite of its label. **Fix:** send
  SetEnabled(!shown_state) so the action matches the label.

**Config durability**
- **M7.** save() truncates config.toml in place — a crash mid-write leaves a
  corrupt file. **Fix:** write to a temp file, then rename.
- **M8.** One typo in a hand-edited config silently drops ALL settings to
  defaults, and the next persist() cements the defaults over the user's file.
  **Fix:** log loudly; refuse to persist over a file that failed to parse.
- **M9.** persist() regenerates the whole file from memory — comments and hand
  edits made while the daemon runs are lost. Accepted for now (daemon owns the
  file), but documented here as a deliberate single-writer decision.

**Status / D-Bus plumbing**
- **M10.** The Status struct is declared three times with only a comment as
  guard, and drift already shipped once (commit c177090 added `following` to
  the daemon while the tray still declared 7 fields — caught one commit later).
  On mismatch, reads fail (icon shows "daemon not running") while writes still
  work — the most confusing half-broken state possible. Also hits users via
  binary version skew. **Fix:** pin `<Status as zvariant::Type>::SIGNATURE` to
  the literal `(busdbddbuu)` with a test in each of the three crates.
- **M11.** GetStatus does a fresh timezone lookup on every call: it can
  disagree with what the loop actually applied (boot-time flicker of
  "Waiting for location…"), and it does file I/O per call. **Fix:** the loop's
  cached location moves into State; GetStatus reads the cache.
- **M12.** GetStatus holds the state mutex across that file I/O and the solar
  maths, contending with the apply loop. Fixed by M11's cache.
- **M13.** The panel issues a blocking get_status D-Bus round trip every egui
  frame (~60/s while interacting). **Fix:** poll on a timer (1 s), remember
  the last snapshot between frames.
- **M14.** Day/night sliders fire set_day_temp/set_night_temp on every drag
  frame — one drag is tens of config-file writes. **Fix:** update the curve
  live, but send/persist on drag-release.

**Environment / packaging**
- **M15.** Timezone lookup only matches canonical IANA names — backward links
  (`TZ=Turkey`, `US/Eastern`) silently fail to resolve and the screen stays
  neutral. Core is M1 territory; fix deferred and tracked here.
- **M16.** The .deb depends list is missing `libxkbcommon-x11-0`, which the
  panel dlopens at startup — on a minimal install the panel dies on launch.
  **Fix:** add it to `depends`.
- **M17.** The panel's "Start at login" checkbox shows success even when
  `systemctl --user enable` fails (unit not installed — e.g. cargo-install
  users who skipped the cp step). **Fix:** re-read `is-enabled` after the call
  and show the truth.
- **M18.** After a daemon restart with a different config, an open panel's
  sliders and Revert baseline go stale (anchors seed once). **Fix:** re-adopt
  daemon values when they change externally.
- **M19.** RandR events arriving during apply's round trips sit in x11rb's
  buffer and trigger an immediate redundant re-apply. Harmless but wasteful;
  drain once more after applying.
- **M20.** open_panel's PATH fallback is dead code (current_exe virtually never
  fails); if the panel binary isn't beside the tray, the click silently does
  nothing. **Fix:** check existence, then genuinely fall back to PATH.
- **M21.** A fresh .deb install gives no way to start the daemon without a
  terminal or re-login (unit not enabled, tray autostart applies from next
  login). Packaging UX; address in M6 (postinst note, or README quick-start).

---

## Low — noted, not acted on (16)

current_temp records pre-clamp targets · SetMode ignores unknown strings
silently · "Back to automatic" is two non-atomic calls · suspend watcher thread
never restarts if the signal stream ends · panel's UTC offset is read once
(DST flip while open shifts the curve) · single-instance acquire() conflates
"bus down" with "already running" · config values outside slider ranges clamp
silently in panel memory · panel spawn leaks a zombie (no wait()) · icon names
assume Adwaita in the theme chain · tray outlives its own service if the host
dies · load() swallows all read errors (not just missing file) · save()
reports success when no config path resolves · zone_name() commits to the
first env var even if unresolvable · non-finite coordinates → 1000 K red
(panel can't produce them; CLI could) · .deb autostarts the tray for every
user of the machine · .deb ships no .desktop launcher for the panel (only
autostart), so it's invisible in app menus.

---

## Big picture

1. **The architecture held.** Daemon-owns-everything + thin clients survived
   M5 intact: the tray and panel genuinely hold no state, the poll loop is
   still the only screen writer, and suspend/hotplug design was validated by
   field data (5/5 resumes corrected in <1 s). Core is clean, clamped, and
   still dependency-free. None of the confirmed findings requires an
   architectural change — they are all seams, not beams.
2. **The docs have drifted from the code.** CLAUDE.md still says "two crates"
   (there are four); the README roadmap still shows M5 unstarted; ISSUES.md
   #24 specifies GTK but the panel shipped as egui. The documents that define
   the project no longer describe it. Worth a truth pass before any announce.
3. **The D-Bus contract is the softest spot.** Three hand-copied Status
   structs, no signature test, and the drift failure mode (reads break, writes
   work) actively misleads. M10's three-line tests are the cheapest insurance
   in this list.
4. **"Auto" semantics leaked into the clients.** The daemon exposes primitives
   (SetEnabled/SetMode) and each client composes them differently — M1 is the
   visible symptom. The composite semantic ("follow the sun" implies "on")
   belongs in the daemon; clients should not need compensation logic.
5. **The config file became shared mutable state** (daemon persist vs hand
   edits vs panel-driven writes). The single-writer answer is right, but then
   the writer must be durable and honest: M7 + M8 are the price of that
   decision, and M14 currently multiplies the write traffic.

## Recommended order

1. **P1 — stranding bugs:** H1, H2, H3, M1 (+ client cleanup), M2+M3, M7, M8.
2. **P2 — consistency & cost:** M10 signature tests, M11+M12 location cache,
   M13, M14, M4, M5+M6, M16, M17.
3. **P3 — with M6 packaging:** M15, M18–M21, doc truth pass (CLAUDE.md crate
   count, README roadmap, ISSUES #24 wording), low notes as they surface in
   dogfooding.
