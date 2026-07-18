# TUI design roadmap (#35)

Written after actually studying SunReactor's five screenshots pixel by pixel,
plus the ratatui showcase ecosystem. The goal is not to copy SunReactor — it is
to understand *why* it reads as polished, steal the mechanics, and beat it
where it is weak.

---

## What SunReactor gets right (verified by eye, not vibes)

1. **One-hue theming.** Every screen is a single accent colour plus shades of
   it — dim borders, bright emphasis, solid-block selection. The README then
   shows the same UI in orange, green, blue, and synthwave, which makes one
   tool look like a rich product. Their theme picker ships ~18 named themes
   (Dracula, Gruvbox, Nord, Tokyo Night…). **This is 80% of the magic.**
2. **A big branded header** with a status strip under it:
   `daemon:active · 04:56 · wthr:0% · [LIVE]` — the LIVE chip is a solid
   green badge. You know the system is alive at a glance.
3. **Solid-block selection.** Selected rows/fields get accent background +
   dark text. Unambiguous focus.
4. **Title-on-border cards** with generous inner padding and aligned columns.
5. **A milestone table** (Rise Start 05:09 → 5%, Peak 13:09 → 60%, Night
   Floor 22:07…) — the day's plan as data. Dense, glanceable, useful.
6. **Consistent centered footer** of key hints separated by pipes, on every
   screen.

## What SunReactor gets wrong (our openings)

1. **The logo is literally broken.** The outline-font banner clips and
   overlaps its own rows on every screenshot — the letters are mangled. They
   shipped a garbled wordmark as the first thing you see.
2. **Oceans of dead space.** 40–60% of every screen is empty; content is
   top-anchored with no max-width or vertical balance.
3. **Web-form thinking.** Five tabs for a simple domain; the Limits screen is
   five full-width, three-row text boxes holding the numbers "5" and "60".
   A "TUI Refresh Rate (FPS)" setting exists.
4. **The core mental model is never drawn.** Brightness-over-the-day — the
   entire point of the tool — appears only as a table. The single chart in
   the app is the *weather*, buried in a tab.
5. **Monochrome swallows semantics.** On/off/error states have no red/green
   language (the lone green LIVE chip breaks their own one-hue rule).
6. **Focus glare.** The focused field becomes a giant solid bright block with
   low-contrast text inside.

**Positioning conclusion:** we keep our one-screen philosophy (it directly
attacks weaknesses 2 and 3), we draw the curve front and centre (weakness 4),
we keep red/green state dots (weakness 5), and we render our wordmark
*correctly* (weakness 1). We adopt their theming, header strip, selection
language, and milestone table.

---

## Design reference pool

- **SunReactor** — theming mechanics, header strip, milestone table.
- **gitui / bottom / yazi / television / atuin / posting** — the ratatui apps
  people screenshot; study their spacing, tab-less hierarchies, and palettes.
- **awesome-ratatui** libraries worth knowing: `tachyonfx` (shader-like
  effects/transitions), `tui-big-text` (already in), `ratatui-splash-screen`.
- **vhs** (charmbracelet) — scripted terminal GIFs; deterministic README
  demos, the same tape re-rendered per theme.

---

## Roadmap

### Phase A — identity and cohesion (the 80%)
- **A1. Theme system.** A `Theme` struct (accent, accent_dim, surface, text,
  muted, ok, err) threaded through every widget — no bare `Color::*` at call
  sites. Curated named themes: `ember` (default, our warm orange), `gruvbox`,
  `nord`, `tokyo-night`, `phosphor`, `synthwave`. `--theme` flag + `T` cycles
  live. Every README screenshot in a different theme, like theirs.
- **A2. Header strip.** Centered, *unbroken* slant wordmark in accent; under
  it `daemon:active · 21:36 · sun −12.4° · [LIVE]` with a real green/red
  chip; version right-aligned.
- **A3. Emphasis language.** Solid accent chips for keys (have), accent-bg
  selection for anything selectable, bold accent card titles (have).

### Phase B — layout discipline
- **B1. Max-width container** (~100 cols) centered horizontally; extra
  vertical space distributed around the content, never all below it.
- **B2. Consistent card interiors:** 1-cell padding everywhere, equal-height
  top cards, aligned label columns.
- **B3. Responsive tiers:** compact (<66 cols), normal, wide (>110 cols —
  cards and curve side by side).

### Phase C — content richness
- **C1. "today" milestone table,** computed from our own solar maths per
  location: Night end (−6°), Full day (+3°), Solar noon, Fall start, Full
  night — each with time and the kelvin it lands on. Beats SunReactor's
  table because ours derives from the real curve, theirs from percentages.
- **C2. Sunrise/sunset times** in the sun card.
- **C3. `?` help overlay** — dim modal, key list, any key closes.

### Phase D — ship the look
- **D1. vhs tape** scripted demo (open → toggle → nudge night → auto), one
  GIF per theme, deterministic.
- **D2. README hero GIF + theme gallery row** (#29 feeds on this).

Acceptance: a stranger seeing one README GIF should think "polished tool",
and the same UI must hold up in `ember`, `nord`, and `synthwave` shots.
