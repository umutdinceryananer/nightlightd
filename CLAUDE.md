# Working in this repository

`nightlightd` — a screen colour temperature daemon for X11, written in Rust.

Read `docs/HOW-IT-WORKS.md` and `docs/ISSUES.md` before touching anything. `docs/PRIOR-ART.md` records what the incumbent tool actually does, measured on real hardware; it defines what this project is for.

---

## Architecture, decided

These are settled. Do not propose alternatives unless something concrete forces a change.

- **One binary, two modes.** `nightlightd --daemon` runs the daemon. `nightlightd --temp 2800` acts as a client and messages the daemon. A separate `nightlight` client binary may be added later.
- **Daemon plus thin client**, talking over DBus. The daemon owns all state and all screen access. Clients hold nothing.
- **DBus name `org.nightlightd.Daemon` is the single-instance lock.** Name taken means another daemon is running; exit cleanly.
- **A single-threaded event loop** waiting on two sources: the X11 file descriptor and a timer. Never two things at once.
- **systemd user service** for autostart, enabled in *user* scope, never global.
- **Two crates.** `core` holds pure logic — colour, solar elevation, timezone lookup — and depends on nothing. `cli` holds the binary, X11, and DBus. `core` must be testable without a display.

---

## Out of scope for v0.1

Say no to these. They are the reasons comparable projects died.

- **Wayland.** X11 only. A wlroots backend is `#31`, later, as a separate output layer.
- **Brightness.** Colour only. This is where Gammy drowned.
- **ICC colour profiles.** Legitimate, expensive, `#32`.
- **Per-monitor temperature.** `#34`.
- **NVIDIA driver quirks.** Wait for reports.

If a change requires a new dependency, say so before adding it and explain what it buys.

---

## How I want to work

I am learning Rust. This project is the vehicle for that. Working code I do not understand is worth less than nothing to me.

- **Explain before you write.** What you are about to do, and why.
- **One function at a time.** Do not produce a finished file in one pass.
- **Name the alternative you rejected**, whenever you make a design choice.
- **Do not assert that something works.** Give me the command; I will run it and tell you what I saw.
- **Push back on me.** If I ask for something that contradicts the architecture above, say so.

Milestone M1 (`#4`–`#9`) is pure maths with no I/O. **I write the implementations there.** Write the tests, review what I produce, tell me what is un-idiomatic — but do not hand me the answer.

From M2 onward, the difficulty is X11 and event loops rather than Rust. Lean in.

---

## Verification is not optional

Three issues in `docs/ISSUES.md` are marked **Verified** — `#7`, `#19`, `#13`. They exist because a specific defect was reproduced with a specific command. When you implement one, the acceptance criterion is a command I can run and an output I can see. Not a claim.

`#13` in particular is where a plausible-looking implementation will pass a casual glance and fail in the real world. It has to survive an actual suspend cycle and an actual monitor being unplugged. There is no way to fake that.

---

## Conventions

- `cargo clippy` and `cargo fmt` must be clean. CI enforces both.
- No `unwrap()` outside tests.
- Missing files, absent hardware, malformed config — degrade quietly and carry on. A night light that panics is worse than no night light.
- Commit messages: imperative mood, prefixed by area. `core: add temperature interpolation table`.
- Reference the issue number in the commit body when there is one.
