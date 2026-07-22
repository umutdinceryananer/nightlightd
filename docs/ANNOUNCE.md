# Announcement drafts (#30)

Working copy. Edit freely, post in your own voice, delete when done.
The rule from ISSUES.md: don't oversell. "I couldn't get redshift working
so I wrote this" lands better than "a revolutionary new tool."

Two disclaimers every post must carry, in that channel's tone:

1. **Prior art.** Our most serious worry is that someone has already
   built exactly this. docs/PRIOR-ART.md is our best effort to check,
   but say it openly and *ask*: if a tool already does this properly,
   tell me and I'll link it (and happily retire this).
2. **One machine.** Dogfooded on exactly one computer: Linux Mint
   Xfce, X11, one monitor. The .deb and AUR builds exist, but other
   setups are untested territory; say so plainly and ask for reports.

---

## LinkedIn (TR)

Birkaç ay önce Rust öğrenmeye karar verdim ve klasik tavsiyeye uydum:
kitap değil, proje.

Problem gerçekti: Linux Mint kurulumumda gece ekranı sarartacak düzgün
bir araç bulamadım. redshift arşivlenmiş; devamı olan gammastep ise
konum bulamayınca sessizce hiçbir şey yapmıyor, iki kopyası aynı anda
çalışıp ekranı titretebiliyor, uyku dönüşünü fark etmiyor. Üçü de
laf değil, hepsini komutlarla ölçüp repoya yazdım.

nightlightd bunları düzeltmek için var:

- Sıfır yapılandırma: konumu /etc/localtime'dan buluyor. Soru yok,
  izin yok, ağ yok.
- Tek kopya garantisi: D-Bus adı kilit görevi görüyor, ikinci daemon
  kendini kapatıyor.
- Uykudan dönüşte, monitör takılıp çıkarıldığında ekran olaylarını
  dinleyip filtreyi yeniden basıyor.

Üstüne bir de terminal arayüzü koydum (ratatui ile): temalar ekranın o
anki renk sıcaklığını takip ediyor. Gece çöktükçe arayüz de ekranla
birlikte ısınıyor. Güneşin konumundan türetilmiş günlük program, harita
üzerinden konum seçimi, hepsi terminalde.

v0.1.0 çıktı: .deb paketi ve AUR'da hazır. X11-only, bilinçli olarak.

İki dürüst not. Birincisi, en ciddi endişem bunun zaten var olması.
Yazmadan önce mevcut araçları elimden geldiğince ölçtüm ve repoya
belgeledim, ama bunu düzgün yapan bir araç biliyorsanız lütfen
söyleyin; seve seve ona yönlendiririm. İkincisi, proje şimdilik tek
bir makinede test edildi (Mint Xfce, X11, tek monitör). Farklı
kurulumlarda pürüz çıkması normal, çıkarsa issue açmanız en büyük
katkı olur.

Repo: https://github.com/umutdinceryananer/nightlightd

Rust'ta ilk projem. Kod hakkında her türlü eleştiriye açığım.

---

## r/linux (EN)

**Title:** nightlightd, a zero-config night light daemon for X11 (first
release)

I couldn't get a night light working reliably on my Linux Mint Xfce
install, so I wrote one. redshift is archived; gammastep works but
inherited three defects from redshift's architecture, each of which I
measured before writing a line of code (evidence with commands in the
repo's docs/PRIOR-ART.md):

1. With no config and no geoclue, it hangs at location acquisition and
   applies nothing, silently. nightlightd reads /etc/localtime and
   looks the coordinates up in the tzdata every distro already ships.
   No network, no prompts.
2. Nothing stops two copies running at once (I found four accumulated
   instances from three autostart mechanisms on a stock install).
   nightlightd claims a D-Bus name as a single-instance lock; a second
   copy exits.
3. It never subscribes to RandR events, so a resume from suspend or a
   hotplugged monitor leaves the screen neutral until the next polling
   tick. nightlightd listens and re-applies.

One daemon, three thin clients over D-Bus: tray icon, an f.lux-style
settings panel, and a ratatui terminal dashboard whose default theme
follows the actual colour your screen is filtered to.

X11 only, on purpose; the mechanism doesn't exist on GNOME/KDE
Wayland. v0.1.0 is out with a .deb, and it's on the AUR.

Two honest caveats. First: my biggest worry is that something already
does exactly this and I missed it. PRIOR-ART.md is my best effort to
check, but if you know a tool that gets all three of these right,
please say so and I'll point people at it instead. Second: this has
been dogfooded on exactly one machine (Mint Xfce, X11, one monitor).
The packages build and install, but every other setup is untested.
If it breaks on yours, an issue with your GPU/driver details is the
most useful thing you can send me.

https://github.com/umutdinceryananer/nightlightd

First Rust project; happy to hear what's wrong with it.

---

## r/xfce (EN)

**Title:** I wrote a zero-config night light daemon for Xfce/X11:
nightlightd



---

## r/unixporn (EN, screenshot post)

**Title:** [Xfce] nightlightd, my night light daemon's ratatui
dashboard; the theme follows your screen's actual colour temperature

(Attach docs/screenshots/01-now.png, or a set. unixporn wants OC +
details comment:)

Details comment:
- The tool: nightlightd, a zero-config colour temperature daemon for
  X11 I wrote to learn Rust:
  https://github.com/umutdinceryananer/nightlightd
- The TUI: ratatui + tui-big-text. The default "live" theme's accent is
  the actual blackbody tint the daemon is applying to the screen, so
  the whole interface warms through the evening in step with your
  display. Other themes: tokyo, mocha, nord, gruvbox, synth, ember,
  phosphor.
- The curve is computed from real solar elevation at your location
  (derived from your timezone; no config, no geoclue).
- Terminal: xfce4-terminal.
- Caveat: dogfooded on one machine only (Mint Xfce, X11). Young
  software; issues welcome. If something already does this, tell me.

---

## GitLab comment: xfce4-settings #111 only

Reality check (verified 2026-07): the night-light request is CLOSED as
Won't Fix. xfce4-settings #111 (2017, 11 participants) was closed in
2021 by the maintainer with the position that redshift covers it and
reproducing it inside Xfce would be a maintenance burden. In 2022 a
commenter predicted redshift and f.lux would be abandoned; redshift
was archived in 2026. xfce4-power-manager #161 is a duplicate, skip
it. So the comment does not argue with the Won't Fix; it agrees with
it and fills the third-party role the maintainers pointed at. One
comment, no follow-ups; it is a maintainer's tracker, not a billboard.

This issue is closed and I'm not asking to reopen it; the maintainers'
call was that this belongs in a third-party tool, and I think that
call was right. But since it was made, redshift has been archived
(exactly as predicted above), so I'm leaving this for anyone who still
lands here from a search: I wrote a standalone daemon that does night
light on Xfce/X11 with zero configuration. Location comes from
/etc/localtime, gamma goes via XRandR, the filter is re-applied on
resume and monitor hotplug, and a D-Bus name lock makes a second
instance impossible. Tray icon and settings panel included. Packaged
as a .deb and on the AUR.

https://github.com/umutdinceryananer/nightlightd

Honesty first: it has only been dogfooded on my own machine (Mint
Xfce, one monitor), so treat it as young software. And if another tool
already fills this role properly, link it here and I'll gladly defer
to it.

---

## Linux Mint forum (EN)

**Title:** nightlightd, a night light that works out of the box on
Mint Xfce

Wrote this after fighting redshift/gammastep on a stock Mint Xfce
install (silent failure without a location, duplicate instances from
autostart, no recovery after suspend). nightlightd needs no
configuration at all: install, enable, done. Location comes from your
timezone. Tray icon, settings panel, terminal dashboard. .deb on the
releases page.

Two things to know before installing: I could only test on my own
machine (Mint 22 Xfce, one monitor), so consider it a beta and report
anything odd; and if there's an existing tool that already does this
reliably on Mint, please point me to it. I looked and couldn't find
one, but I'd rather recommend it than maintain a duplicate.

https://github.com/umutdinceryananer/nightlightd

---

## awesome-ratatui PR

Repo: https://github.com/ratatui/awesome-ratatui. Fork on GitHub, add
under "Apps" (alphabetical), one line:

    - [nightlight-tui](https://github.com/umutdinceryananer/nightlightd) - Dashboard for the nightlightd screen-temperature daemon. The theme follows the actual colour your screen is filtered to.

PR title: `Add nightlight-tui`
PR body: one sentence + a screenshot link.

---

## ratatui Discord, #showcase channel

(This is the most direct line to the ratatui maintainers; Orhun has
written that he finds projects via GitHub, social media and the
Discord servers. Short and visual, one message, attach 01-now.png.)

Built my first Rust project with ratatui: nightlight-tui, the
dashboard for a zero-config screen colour temperature daemon (X11).
The default theme's accent is the actual colour the daemon is
filtering the screen to, so the whole UI warms in step with your
display as the evening comes on. Also: a solar schedule computed from
real sun elevation, and a braille world map for pinning a location.

https://github.com/umutdinceryananer/nightlightd

---

## r/rust (EN)

**Title:** My first Rust project: a zero-config screen colour
temperature daemon for X11, with a ratatui dashboard

I set out to learn Rust with a real problem: I couldn't get a night
light working reliably on Linux Mint Xfce. redshift is archived and
gammastep inherited three measurable defects from it (silent hang
without a location source, no protection against duplicate instances,
no reaction to RandR events after suspend). All three are documented
with commands and outputs in the repo's PRIOR-ART.md.

The result is a workspace of five crates: a pure core (colour maths,
solar elevation, timezone-to-location) with no dependencies, a daemon
owning X11 and D-Bus, and three thin clients that hold no state: a
tray icon (ksni), a settings panel (egui), and a ratatui dashboard.
The TUI's default theme follows the actual colour the screen is
filtered to; the schedule tab derives the day's milestones from real
solar elevation rather than hand-set times.

Things Rust made pleasant: the core crate is pure functions and tests;
the wire format of the D-Bus status struct is pinned by a test in
every client, so a signature drift fails CI instead of failing at
runtime.

Honest caveats: dogfooded on one machine (Mint Xfce, X11, one
monitor), and my biggest worry is that something already does exactly
this; if so, tell me and I'll point people there.

https://github.com/umutdinceryananer/nightlightd

It's my first Rust project, so unidiomatic corners are guaranteed.
Reviews of the code are worth more to me than stars.

---

## Order and pacing

0. GitHub repo topics (rust, tui, ratatui, x11, night-light,
   color-temperature, daemon): two minutes, permanent discoverability;
   Orhun trawls GitHub by exactly these.
1. The GitLab comment on #111 first (quiet, useful, low key). DONE.
2. r/xfce and the Mint forum (the audience with the actual problem).
3. awesome-ratatui PR + ratatui Discord #showcase (the direct line to
   the ratatui crowd).
4. r/rust and then r/linux once the first feedback is in (so the
   threads have answers).
5. r/unixporn + LinkedIn last, with the best screenshot.

Don't post everything the same hour. Spread over a few days; each
channel's feedback improves the next post.
