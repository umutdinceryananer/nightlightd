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


---

## End-of-week readout

<!-- fill this in after 7 days -->

**Biggest recurring annoyance:**

**Did suspend/resume ever fail:**

**Did override-without-auto bite me:**

**What M5/M6 should build first, based on the above:**
