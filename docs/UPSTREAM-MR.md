# Upstream MR: timezone location provider for gammastep

**Goal.** Add a location provider to gammastep that derives an approximate location from the system timezone, so the program stops hanging silently when geoclue2 is unavailable.

**Why this comes before writing `nightlightd`.** See `PRIOR-ART.md`, defect 1. This is roughly 80 lines of C against an interface that already exists. It fixes the bug for every existing gammastep user, which is far more people than will ever install a new tool. And whatever the maintainer says in review tells us whether defects 2 and 3 are patchable in place — which decides whether `nightlightd` needs to exist at all.

**Three outcomes, all useful.**

| Outcome | What we get |
|---|---|
| Merged | A contribution in the tool that replaced redshift. |
| Rejected | A documented, public reason why a rewrite is warranted. Cite it in the README. |
| Ignored | The same, plus the code is already written and ports to Rust in an afternoon. |

**Scope discipline.** This MR adds one file, edits three lines in three others, and touches nothing else. If it grows beyond that, it stops being reviewable and starts being a fork.

---

## Phase 0 — Ask before building

- [ ] **0.1** Open an issue on `gitlab.com/chinstrap/gammastep` before writing any code.

  Suggested text:

  > Running gammastep with no config file and no `-l` prints its settings, then hangs at location acquisition. Nothing is applied and no error is emitted. Geoclue2 is unavailable on many desktops, which leaves manual coordinates as the only working option.
  >
  > Would a fallback provider that derives an approximate location from the system timezone (`/etc/localtime` → `zone.tab`) be welcome? Accuracy within about 1° is sufficient for the solar elevation transition, and it would remove the need for any configuration in the common case.

- [ ] **0.2** Do not wait for a reply to start Phase 1. Do the reconnaissance regardless.
- [ ] **0.3** If no reply after two weeks, open the MR anyway and reference the issue.

---

## Phase 1 — Fork, clone, build

- [ ] **1.1** Fork `gitlab.com/chinstrap/gammastep` through the web UI. GitLab has no CLI fork.

- [ ] **1.2** Clone and wire up the upstream remote.

  ```bash
  git clone https://gitlab.com/<username>/gammastep.git
  cd gammastep
  git remote add upstream https://gitlab.com/chinstrap/gammastep.git
  git remote -v
  ```

- [ ] **1.3** Branch. Never work on `main`.

  ```bash
  git checkout -b timezone-location-provider
  ```

- [ ] **1.4** Install build dependencies. This works because the package is already in the archive.

  ```bash
  sudo apt build-dep gammastep
  ```

  If this fails with "unable to find a source package", enable source repositories in Software Sources, run `sudo apt update`, retry.

- [ ] **1.5** Identify the build system. Do not assume.

  ```bash
  ls | grep -E 'meson.build|configure.ac|Makefile.am|CMakeLists'
  ```

  - `meson.build` → `meson setup build && ninja -C build`
  - `configure.ac` → `./bootstrap && ./configure && make`

- [ ] **1.6** Run the freshly built binary directly. Do not install it.

  ```bash
  ./build/gammastep -m randr -l 39.9:32.8 -v
  ```

  **Acceptance:** output matches the system-installed 2.0.9 exactly. This is your reference point. If it differs, stop and find out why before changing anything.

---

## Phase 2 — Autopsy

The objective is to understand the location provider interface well enough to implement it from memory. Run each command, read the output, do not skim.

- [ ] **2.1** Inventory the source tree.

  ```bash
  ls src/
  ```

  Expect `location-geoclue2.c` and `location-manual.c`. Both implement the same interface.

- [ ] **2.2** Find the interface definition.

  ```bash
  grep -rn "location_provider_t" src/*.h
  ```

  Read every field of the struct: `init`, `start`, `free`, `print_help`, `set_option`, `get_fd`, `handle`. Your provider fills these in. Write down what each one is called with and what it must return.

- [ ] **2.3** Find the registry.

  ```bash
  grep -rn "location_providers" src/
  ```

  This is the array your provider gets added to. Note the ordering — it matters. Yours goes **after `geoclue2`, before `manual`**, so it only runs when geoclue has already failed.

- [ ] **2.4** Read the simplest existing implementation end to end.

  ```bash
  cat src/location-manual.c
  ```

  Around a hundred lines. This is the skeleton you will copy. Understand why `get_fd` returns what it returns for a provider that has no file descriptor to poll.

- [ ] **2.5** Read the harder one, for the failure paths.

  ```bash
  cat src/location-geoclue2.c
  ```

  Specifically: what does it do when it cannot resolve a location? Trace that path up into `redshift.c`. **This is where the silent hang lives.** Note the line number.

- [ ] **2.6** Find every place a provider must be registered besides the array.

  ```bash
  grep -rn "location-manual" meson.build src/Makefile.am 2>/dev/null
  grep -rn "location-manual" po/POTFILES.in
  ```

  Miss either of these and the build breaks or CI fails on translations.

- [ ] **2.7** Write down, in your own words, what happens between `main()` and the first gamma ramp write. Three paragraphs, no code. If you cannot do this, go back to 2.2.

  **Acceptance:** you can explain the provider lifecycle to someone else without opening the files.

---

## Phase 3 — Implement

- [ ] **3.1** Prototype the coordinate lookup in isolation, outside gammastep.

  Write a standalone `tz_probe.c` with a `main()`. Given a `TZ` value, print the coordinate. Compile it on its own. Do not touch gammastep's build until this is correct.

  **Acceptance:** `Europe/Istanbul` → roughly `41.0 N, 29.0 E`. `America/New_York` → roughly `40.7 N, -74.0 E`. `Pacific/Auckland` → roughly `-36.9 N, 174.8 E`. Southern and western hemispheres both correct.

- [ ] **3.2** Handle the three ways a system stores its timezone.

  1. `/etc/localtime` is a symlink into `/usr/share/zoneinfo/`. Use `readlink()`, strip the prefix. This is the common case.
  2. `/etc/localtime` is a plain copy of the zone file, not a symlink. Fall back to reading `/etc/timezone` as text.
  3. Neither exists. Return failure.

- [ ] **3.3** Parse `zone.tab` coordinates correctly. This is the one real trap.

  The format is ISO 6709, packed and signed: `+4101+02858` means 41°01′ N, 28°58′ E. Degrees and minutes are concatenated with no separator. Latitude is `±DDMM`, longitude is `±DDDMM`. Some entries carry seconds: `±DDMMSS` and `±DDDMMSS`.

  Also: some systems ship only `zone1970.tab`, not `zone.tab`. Try `zone.tab` first, fall back to `zone1970.tab`.

  **Acceptance:** your prototype parses both forms and both hemispheres.

- [ ] **3.4** Fail silently. If anything is missing or malformed, the provider returns failure and gammastep moves on to the next one. No error output, no abort. A fallback provider that shouts is worse than one that does not exist.

- [ ] **3.5** Write `src/location-timezone.c`, copying the shape of `location-manual.c` exactly. Same brace style, same naming, same error conventions.

- [ ] **3.6** Register it: the provider array, the build file, `po/POTFILES.in`.

- [ ] **3.7** Add no new dependencies. libc only. If you find yourself reaching for a library, you are solving the wrong problem.

---

## Phase 4 — Verify

- [ ] **4.1** The bug is fixed.

  ```bash
  ./build/gammastep -m randr -v
  ```

  **Acceptance:** `Location:`, `Period:` and `Color temperature:` all appear. No `-l` flag. No config file.

- [ ] **4.2** Other timezones work.

  ```bash
  TZ=America/New_York ./build/gammastep -m randr -v -o
  TZ=Pacific/Auckland ./build/gammastep -m randr -v -o
  ```

  `-o` is one-shot; it prints and exits.

- [ ] **4.3** Explicit configuration still wins. A user with `-l` or a manual config block must be unaffected.

- [ ] **4.4** Geoclue still wins when it works. Your provider must not preempt it.

- [ ] **4.5** Nothing breaks when the files are absent.

  ```bash
  sudo mv /etc/localtime /etc/localtime.bak
  ./build/gammastep -m randr -v -o    # should fail gracefully, not crash
  sudo mv /etc/localtime.bak /etc/localtime
  ```

- [ ] **4.6** `git diff --stat` shows one new file and three touched lines. If it shows more, cut it back.

---

## Phase 5 — Submit

- [ ] **5.1** Read the project's own rules before writing anything.

  ```bash
  cat CONTRIBUTING* 2>/dev/null
  git log --oneline -20
  ```

  The last twenty commits tell you the message format. Imitate it.

- [ ] **5.2** Squash to one commit.

  ```bash
  git fetch upstream
  git rebase -i upstream/main
  ```

- [ ] **5.3** Open the MR. Title: `Add timezone-based location provider as fallback`. Boring on purpose.

- [ ] **5.4** Description, three sections. Describe behaviour, not opinions. Do not write "this is an obvious bug."

  ```
  ## Problem

  With no config file and no -l, gammastep prints its settings and then
  hangs at location acquisition. Nothing is applied and no error is
  emitted. Geoclue2 is unavailable on many desktops, which leaves manual
  coordinates as the only working option.

  ## Change

  Adds a `timezone` location provider deriving an approximate location
  from /etc/localtime and zone.tab. Registered after geoclue2 and before
  manual, so it runs only when geoclue has already failed. Explicit
  configuration is unaffected. No new dependencies.

  ## Testing

  Linux Mint 22 (Xfce, X11). TZ=Europe/Istanbul, TZ=America/New_York,
  TZ=Pacific/Auckland. `gammastep -m randr -v` with no config now
  resolves a location and applies the correct period. Accuracy is
  within about 1°, i.e. a few minutes of sunset time.
  ```

- [ ] **5.5** Link the issue from Phase 0.

- [ ] **5.6** Wait. Small projects with one maintainer take months. Do not ping weekly. Start `nightlightd` M0 while you wait.

---

## Gotchas, collected

- `readlink()` does not null-terminate. Do it yourself.
- `zone.tab` is tab-separated and has comment lines starting with `#`. Skip them.
- ISO 6709 minutes are base 60, not decimal. `+4101` is 41 + 1/60 degrees, not 41.01.
- Longitude has three degree digits, latitude has two. Sign is always present.
- A `/etc/localtime` that is a regular file, not a symlink, is common on some systems.
- Do not use `getenv("TZ")` as the primary source. It is usually unset. Use it only for testing.
- Squash. A five-commit MR with "fix typo" in the history reads as careless.

---

## Working with Claude Code

Put this in `CLAUDE.md` at the root of the fork. It gets read on every session.

```markdown
# Working in this repo

A fork of gammastep (itself a fork of redshift), written in C.
I am adding exactly one feature: a timezone-based location provider.
See UPSTREAM-MR.md for the full plan.

## Scope

One new file: src/location-timezone.c
Three edited lines: the provider array, the build file, po/POTFILES.in
Nothing else. No refactoring. No cleanup. No new dependencies.

## Style

Read src/location-manual.c before writing anything. Match its brace
style, naming, and error conventions exactly. An MR that reformats
adjacent code will not be merged.

## How I want to work

I am learning this codebase and I am learning C conventions in it.

- Explain what you are about to do and why, before writing code.
- Do not write the whole file in one pass. One function at a time.
- When you make a design choice, tell me the alternative you rejected.
- If I ask you to explain something, assume I know nothing about it.
- Ask me to run the verification steps myself. Do not assert that
  something works without me having seen it work.

The maintainer will ask me questions in review. "Claude wrote it" is
not an answer I can give. I need to understand every line.
```

That last paragraph is the important one.

---

## What this earns you regardless of the outcome

You will have read a real C codebase, found the exact line where a bug lives, and written a patch against an interface you did not design.

When you get to `nightlightd` issue **#7**, you will already know precisely what a location provider has to do. The Rust version will take an afternoon.
