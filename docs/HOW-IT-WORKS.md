# How this works

This document explains **what every piece in `ISSUES.md` is and why it's there**, assuming you know nothing. There is no code here. Only ideas.

---

## In one sentence

**A program that shifts your screen towards red in the evening and shifts it back in the morning.**

That's it. Everything else is the work of doing that reliably.

---

## Part 1: How a screen's colour gets changed

### The little table on the graphics card

When your computer draws something, that image doesn't go straight to the monitor. There's a stop along the way.

Just before sending each pixel's red, green and blue values out, the graphics card passes them through **a translation table**. It looks something like this:

```
red in 0   →  red out 0
red in 1   →  red out 1
red in 2   →  red out 2
...
red in 255 →  red out 255
```

Normally the table is stupid: whatever goes in, comes out. That's called the **identity transform**.

The table's name is a **gamma ramp**. "Ramp" because if you plot it, it looks like a straight incline.

### The trick

Now imagine you change the blue channel's table like this:

```
blue in 100 →  blue out 55
blue in 200 →  blue out 110
```

Every blue value is halved. You don't touch red at all.

The result: everything on screen has less blue in it, so it looks redder. **That's the red filter.** You're not putting coloured glass in front of the monitor — you're corrupting the graphics card's translation table.

### Three nice consequences

1. **No root needed.** You already have permission to write to your own screen's table.
2. **Screenshots come out clean.** The change happens *after* the image is drawn, on the way to the monitor. A screenshot tool sees the original, before the table.
3. **Nothing gets slower.** The graphics card was already using this table anyway.

### And one ugly one

The table is **not permanent.** The system silently resets it when any of these happen:

- The computer wakes from sleep
- The resolution changes
- A monitor is plugged in or unplugged
- You exit a fullscreen game

The screen suddenly goes blue again. The user says "this program is broken."

**None of the competing tools fix this. You will.** (`ISSUES.md` #13)

---

## Part 2: How you know which colour to write

### What colour temperature means

Heat a piece of iron and it glows red, then orange, then white, then bluish. Physicists turned that relationship into a formula: **give it a temperature, it tells you a colour.**

Measured in Kelvin:

| Temperature | What it feels like |
|---|---|
| 1900K | Candlelight. Extremely red. |
| 2500K | Old incandescent bulb. Clearly yellow. |
| 2800K | The sweet spot for most people. |
| 4500K | Slightly warm. |
| 6500K | Your screen's normal state. No change. |

The confusing part: **a lower number means redder.** Counterintuitive, but that's the convention.

### From temperature to RGB

The program's first job: take the number `2800` and produce three multipliers, something like `(red: 1.00, green: 0.75, blue: 0.55)`.

There's a formula for this, but it's fiddly. What Redshift does is smarter: it precomputes the answer for every 100K step between 1000K and 10000K, stores it in a table, and interpolates between neighbours. Faster, and fewer precision bugs.

Do the same. (#4)

### Then you turn those multipliers into a ramp

The red table stays as it was (multiplier 1.00).
Every value in the green table gets multiplied by 0.75.
Every value in the blue table gets multiplied by 0.55.

Done. (#5)

---

## Part 3: How you know *when* to change it

### Why we don't use the clock

You could just say "dim it at 8pm." But:

- In Ankara the sun sets at 20:45 in June and 16:45 in December.
- In Tallinn it's still light at midnight in summer.

A program that uses a fixed time is wrong.

### Instead: where the sun is

The right question is: **how far below the horizon is the sun right now?**

You can compute this. Inputs: latitude, longitude, date and time. Output: the sun's angle in degrees. Positive means it's up; negative means it's set.

The formula is a standard algorithm published by NOAA (the US weather agency). About 40 lines of trigonometry. No library required. (#6)

### Then a smooth transition

Sun angle above `+3°`: full daylight, 6500K.
Sun angle below `-6°`: full night, 2800K.
In between: proportionally somewhere between the two.

So the filter never snaps on and off. It arrives slowly, and the user doesn't even notice. (#8)

### But where do you get the latitude and longitude?

**This is where the product's most important difference lives.**

Redshift gets it from a service called Geoclue, which tries to guess your location over the network. On most machines it no longer works. When Redshift can't find your location, it can't compute sunset, and when it can't compute sunset, it never starts at all.

**That is the cause of the vast majority of "I installed Redshift and it errored out" reports.**

Your solution is almost insultingly simple: **look at the timezone.**

The system already knows: `Europe/Istanbul`. And the timezone database installed on every Linux machine already contains a representative coordinate for every zone. For Istanbul that's roughly 41.0 N, 29.0 E.

If you live in Ankara, that's 350km off. In a sunset calculation that's a few minutes of error. **Nobody will ever notice.**

In exchange:

- No network access
- No permission prompts
- No questions for the user
- No mandatory config file

**Install it, run it, it works.** (#7)

---

## Part 4: The shape of the program

### A watchman and a remote control

The program has to stay on watch. Time passes, the screen's table gets wiped, the user changes something. A program that runs once and exits cannot do that job.

But the watchman doesn't need a face. No window, no icon, silent, in the background.

So: two pieces.

**The watchman (daemon).** Runs in the background. Tracks the time, writes the colour to the screen, puts it back when it gets wiped. Nobody ever sees it.

**The remote (client).** A tray icon or a terminal command. It does nothing on its own. It just sends the watchman a message: "set the temperature to 2800."

### Why this is better

- If the tray icon crashes, the filter keeps living.
- The terminal, the tray icon, and anything you write later all talk to the same watchman. **One brain, many remotes.**
- In an environment with no system tray at all, the program still works.

Redshift never made this split, which is why it's fragile. Its filter and its interface are welded together.

### One file, two modes

You don't write two programs. You write one executable with two personalities:

```
nightlightd --daemon      →  run as the watchman
nightlightd --temp 2800   →  run as the remote, send the watchman a message
```

The user installs one thing. The code lives in one place. (#20)

---

## Part 5: How the watchman and the remote talk

### What DBus is

**The standard postal service that Linux programs use to talk to each other.**

It is running on your machine right now. You don't install it. When you change the volume, when a notification arrives, when a Bluetooth device connects — all of it goes over DBus.

How it works: a program claims a **name**, say `org.nightlightd.Daemon`. Another program sends a message to that name. The postman delivers it.

Your watchman will claim a name. Your remote will send messages to it. (#18)

### And a free gift: the single-instance guarantee

A DBus name can have **exactly one owner.** If one program has claimed `org.nightlightd.Daemon`, a second one cannot.

So your watchman starts like this:

1. Try to claim the name.
2. Name taken → print "already running", exit.
3. Name free → claim it, start working.

**The screen flicker you saw becomes architecturally impossible.** (#19)

### Why the flicker happened

Redshift lets two copies run at once. One says "make the screen 2800K", the other says "no, 6500K", and both write once a second. The screen ping-pongs.

Why two copies appear: there's a file in the system's autostart folder, and the user also added an entry by hand. Neither knows about the other.

XFCE's developers know about this too — it's recorded in their own issue tracker. It has gone unfixed for years.

---

## Part 6: How the program starts at login

### The old way, and why it's bad

There's a folder called `~/.config/autostart/`. You drop a file in it, and the desktop environment runs the command in it at login.

The problem: **nobody knows about anybody.** A file in the system folder, a file in the user folder, and an entry added through the XFCE settings panel. Three copies start.

### systemd user services

`systemd` is the thing that starts and supervises programs on Linux. It's usually associated with system services (networking, sound, printing) — but you can write **user services** too.

You write a small text file:

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

The user runs one command, once:

```bash
systemctl --user enable --now nightlightd
```

In return:

- The program starts at every login
- **It never starts twice.** If it's already running, systemd leaves it alone.
- If it crashes, systemd restarts it
- `systemctl --user status nightlightd` tells you what happened
- `journalctl --user -u nightlightd` gives you the logs

When a user says "it doesn't work," what do you ask them for? Exactly these. (#21, #22)

---

## Part 7: What runs inside the watchman

### The event loop

The watchman is an infinite loop. It sits there, waiting for one of three things:

**1. Time passed.** It wakes once a minute. It asks: "where is the sun right now?" Based on the answer, it nudges the colour.

**2. Something changed on the screen.** X11 tells it: a monitor was plugged in, the resolution changed, the machine woke up. The moment it hears this, it writes the colour again. *(The thing none of the competitors do.)*

**3. A message came from the remote.** The user changed the temperature from the tray, or turned the filter off.

All three are handled **in the same loop, one at a time.** It never does two things simultaneously. That's why there are no race conditions, no deadlocks, no confusion.

### Why this is the hard part

Waiting on two different things at once. The timer is one source, X11 is another. Whichever fires first, you have to react to it — without missing the other.

The standard way on Linux is `poll` — "block until one of these sources is ready." You can turn a timer into a source too (`timerfd`).

**This is the hardest piece of the project.** In a first Rust project, this is where you'll get stuck. That's normal. (#15)

---

## Part 8: X11, Wayland, and why you have to care

### What X11 is

The old system that manages everything on screen. Around since 1984. It draws windows, tracks the mouse, and **grants access to the gamma ramp table.**

Your program talks to X11. Through a library, you tell it "change this screen's table like so." (#10, #11)

### What Wayland is

The newer system meant to replace X11. Safer, cleaner. And it generally **does not let a program touch the screen's colour.**

The result:

| Where | Does it work |
|---|---|
| Any X11 session, any distro | Yes |
| GNOME's Wayland session | No |
| KDE's Wayland session | No |
| Sway, Hyprland (wlroots-based) | Yes, but it needs a separate code path |

So your product is worthless to someone on Ubuntu with GNOME. Put that at the top of the README, without apology.

### Shelf life

The Mint side is fine. Cinnamon is getting Wayland support, but X11 continues alongside it, and no decision has been made about which becomes the default. XFCE and MATE aren't even as far along as Cinnamon.

**Your audience will be on X11 for years.** It's shrinking, but slowly.

### Who your users actually are

- **XFCE and MATE users.** They have nothing built in. They need this most.
- **People running i3, bspwm, awesome.** They're on X11, they have no night light, and they're already comfortable installing things from GitHub. **They'll find you first.**
- **Cinnamon users.** They already have one, but some will prefer yours.

---

## Part 9: Finishing the code doesn't finish the work

You wrote the program. It works. **Maybe a third of the job is done,** because nobody knows it exists.

### Flathub

Something like an app store for Linux. Put your program there and it shows up **inside Mint's own Software Manager.** Nobody has to approve you.

**This is your real storefront.** (#27)

### The .deb file

For people who don't like Flatpak. Drop an installer on GitHub; they double-click it.

Remember Gammy: to use it you have to compile it from source. That's exactly why nobody does. (#26)

### The AUR

Arch Linux's user repository. Free, easy, and **Arch users are the best early testers you will ever get.** When something breaks, they send you the GPU model, the driver version, and the exact error message. (#28)

### Announcing

Reddit (r/linux, r/xfce), the Mint forums, the XFCE forums.

And there's this: XFCE's own issue tracker has a request titled "we need a night light," **open since 2019,** with people still subscribed to it. Leave a comment there: *"I wrote this. Use it until the native version lands."*

A waiting audience. (#30)

### What you will not do

**Do not set "get it into Mint by default" as a goal.** That decision belongs to Mint's founder, and he doesn't warm to new projects. But if a hundred thousand people install it from Flathub, that decision changes on its own.

Don't reverse the order.

---

## Part 10: Where Rust fits

### Four pieces

| Piece | What you use | Difficulty |
|---|---|---|
| Colour and sun maths | Nothing. Pure maths. | Easy |
| Writing colour to the screen | The `x11rb` crate | Medium |
| DBus | The `zbus` crate | Medium |
| Tray icon | The crates aren't mature | Hard |

### The order matters

If Rust is new to you, this project can hand you its difficulties **in the right order**, or throw all of them at your face at once. The right order:

**1. Colour maths.** Number in, number out. A pure function. You learn Rust's basic syntax here. The borrow checker never bothers you.

**2. Sun maths.** More pure maths.

**3. Write to the screen.** Your first real system resource. A connection is opened, closed; there is ownership. **Rust's ownership model suddenly makes sense here.**

**4. Put it in a loop.** Waiting on two sources at once. Your first real difficulty.

**5. Add DBus.** Let the remote talk.

**6. Tray icon.** Last. Rust's weakest area, and by then the program already works.

Each step ends with **something that runs.** By the end of step three you have a program that does `nightlightd --temp 2800`, and you can start using it.

---

## Last word

This is not a job. Nobody will pay you. What we're calling a "product" is **an open source tool whose maintenance you have taken on** — and that maintenance will arrive as messages from strangers saying "it broke on my NVIDIA card" while you're at school.

An abandoned repo looks worse than one that was never written.

But know this too: **something finished, packaged, and actually installed by people** is a different muscle. Contributing to somebody else's project says "I can help." Carrying a thing from start to finish says "I can own." The second one is harder and rarer.

Finish the code in a month. Then **freeze it:** bug fixes only, no new features. That's the only way it survives.
