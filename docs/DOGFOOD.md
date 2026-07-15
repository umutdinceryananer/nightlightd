# Dogfooding log

One week of using nightlightd as my daily driver, before building anything else.

Rule: every time the tool annoys me, or I reach for the terminal, or something
looks wrong — one line here. Date, what happened. That's it. Don't polish.

At the end of the week, whatever is in this file decides what M5/M6 actually
build. Real friction beats guessed features.

---

## The questions I'm watching for

- On resume from suspend: did the screen ever stay the wrong colour? How long?
- After `--temp`, did I forget `--auto` and sit there wondering why it's still
  warm? (If yes → override should probably auto-expire after N hours.)
- Plugging in the external monitor: what happened?
- The sunset transition (~20:40): smooth, or did the shift catch my eye?
- Did checking `--status` in the terminal feel annoying? (If yes → the tray
  icon earns its place.)
- Did I ever need `journalctl`? Why?
- Anything else that made me go "huh, that's not right."

---

## Log

<!-- date — what happened. newest at the bottom. -->

- 2026-07-__ — installed, enabled, running. day one.
- 2026-07-13 — laptop battery died, rebooted, night light just gone. service was `enabled` but sitting there `inactive (dead)` — `graphical-session.target` never starts on this desktop, so nothing pulled the unit in. needed `journalctl` to even see it was dead. fixed the unit: `WantedBy=default.target`.
- 2026-07-13 19.43 - şu an sarı filtre gözükmüyor.
- 2026-07-13 21:01 — the 19.43 "no filter" scare was a false alarm. `--status` at 19.43 would have said `on, 6500 K` — correctly neutral, because it was still daytime. by 21:01 (past sunset) it was `on, 3250 K` and visibly warm. the real gap: **no ambient feedback**, so "correctly neutral because it's day" and "dead" look identical to me. enriched `--status` to show the *source* (auto/manual/off), the current *sun elevation*, and the *resolved location* — now one glance tells the two apart, and confirms the daemon's clock/timezone agree with reality.
- 2026-07-13 21:01 — decision, not yet built: a **system-tray icon** (click the bottom-right corner, see on/off + K, toggle from a menu) is the leading M5 candidate. architecturally it's a *separate* thin D-Bus client — new dependency (SNI via `ksni`, or GTK/appindicator) and its own event loop, so it lives outside the daemon's single-threaded poll loop, never inside it. deferring to the end-of-week readout rather than derailing the dogfooding week to build it.
- 2026-07-16 00:41 — **suspend/resume: answered, with hard data.** 5 suspends in the last 4 days, 5 resumes, every single one corrected in under a second (3 were real corrections, 2 were already the right colour). the strongest case: slept 15 Jul 20:37 at **4771 K**, mid-sunset-ramp, then 3h48m of sleep, woke 00:26:07 in deep night — **2800 K applied at 00:26:07.058. 58 ms.** the 60-second worst case never happened once. this is `#16` proven in real use, not in a test — the thing CLAUDE.md said could not be faked. daemon uptime 51 h on one process, zero warnings or errors in 4 days.
- 2026-07-16 — **two days (14–15 Jul) with no entries, and that IS the finding, not a gap.** nothing caught my eye, nothing annoyed me, I never reached for the terminal. the tool was invisible — which is the entire point of a night light. caveat for the readout: the laptop was suspended for most of that window, only ~6 h of actual awake time, so it had fewer chances to annoy me.
- 2026-07-16 00:50 — **open question I can't answer with my own eyes:** at full night it sits at 2800 K — is that warm *enough*? genuinely can't tell. suspect my eyes have adapted after hours of gradual ramping (which is the design working as intended, but it makes judging it in the moment impossible). for reference, 2800 K is already warmer than redshift/gammastep's 3500 K default, and warmer than our own 4500 K default — my config is a single line, `night_temp = 2800`. must A/B it with `--off`/`--on` before changing anything. also still unresolved: is the night-time discomfort *colour* or *brightness*? if it's brightness, no amount of yellow fixes it, and brightness is deliberately out of scope.

---

## End-of-week readout

<!-- fill this in after 7 days -->

**Biggest recurring annoyance:**

**Did suspend/resume ever fail:**

**Did override-without-auto bite me:**

**What M5/M6 should build first, based on the above:**
