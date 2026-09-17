# DMXpress — User Guide

A friendly, start-to-finish guide for running a light show with DMXpress, written
for someone who has **never used a lighting console before**. Read it top to
bottom once; after that, the "Worked example" near the end is your cheat sheet.

---

## 1. What DMXpress is

DMXpress is a small lighting controller. It takes your fixtures (moving heads,
LED pars, etc.), lets you build looks and effects, and streams the result to the
real world over **Art-Net** (DMX-over-network).

It is modelled on the professional console **grandMA3**, but with friendlier
names. If you ever read MA tutorials, here is the translation table:

| DMXpress name | grandMA3 name | What it is |
|---|---|---|
| **Programmer** | Programmer | Your live "scratch pad" of values you're editing right now |
| **Groups** | Groups | Saved fixture *selections* |
| **Palettes** | Presets | Saved *values* for one feature (color, position, …) |
| **Phasers** | Phasers / Effects | Moving/rolling effects (sine, chase, rainbow…) |
| **Stacks** | Sequences / Cue lists | An ordered list of looks ("cues") you step through |
| **Decks** | Executors | The faders + Go buttons that play your stacks |
| **Views** | Views | Saved window layouts |
| **Command line** | Command line | Type short commands instead of clicking |

> **The one big idea:** you *build* a look in the **Programmer**, then **Store**
> it into something reusable (a Palette, a Cue…). Playback (Stacks/Decks) runs
> *underneath* the Programmer, so whatever you're editing always wins until you
> **Clear** the Programmer.

---

## 2. The screen at a glance

```
┌─────────────────────────────────────────────────────────────┐
│  Top toolbar:  Universe · ArtPoll · Reload patch · 📡🌊⏱🌀… │  ← open/close everything here
├───────────┬─────────────────────────────────┬───────────────┤
│ Fixtures  │        3D Stage view            │   Inspector   │
│  (left)   │   (live beams, click to select) │  (five tabs — │
│           ├─────────────────────────────────┤   see below)  │
│  click to │   Channel control               │               │
│  select   │   (sliders for selected fixtures)│              │
├───────────┴─────────────────────────────────┴───────────────┤
│  🎚 Decks — Grand Master + one fader per stack (when shown)  │
├──────────────────────────────────────────────────────────────┤
│  ⌨ Command line (when shown)                                 │
└──────────────────────────────────────────────────────────────┘
```

Floating windows (Palettes, Phasers, Stacks, Groups, Views, Log, …) open on top
when you toggle their toolbar button. Every panel has its own little **zoom
control** (the `−  100%  +`) in its corner.

### The Inspector (right panel)

The Inspector is five tabs. Press **Alt + 1…5** to jump between them, **Alt + I**
to fold the whole panel away, or roll the wheel over the tab strip. Under the
strip a **context line** always says what is picked — "6 lights · SlimPAR ×6",
"F34 truss 2 · 4 / 24 slots" — with the live colour beside it; click it to jump
to **Selection**, press the ✕ to drop the selection, or right-click it for
select-all / same-type.

**Presets** — your stored looks, as coloured pads. Each pad takes its colour
from the look itself, so a warm wash is amber and a cold blue is blue, with
little badges for the ones carrying oscillators, phasers or a colour cycle.
Type a name and press **Store look** to keep what the Programmer is holding.
Drop pads into **folders**, pin the ones you reach for, search them, and choose
whether a recall cuts, fades over *n* seconds, or follows the Transition window.
The **board** button opens a page of pads laid out exactly like the Stream
Deck's Presets page; turn **Arrange** on and you can click a pad to point it at
a preset, drag pads to swap them, right-click to clear one, and set each pad's
colour, four-letter label and artwork. **Blackout** lives at the bottom.

**Selection** — everything about the lights you have picked. The top card
summarises them and the transform grid edits X / Y / Z, yaw, pitch and size,
marking a row **mixed** when the selected lights disagree. Below that are the
tools: a **nudge** pad (and the arrow keys over the stage), **align** and
**distribute** on any axis, **mirror**, **arrange** into a line, arc, circle or
grid, **aim** (straight down, level, at stage centre, at a point, or fanned),
set a common height, and **hang** the selection onto a tower or truss, choosing
the face and how the lights fill the free slots. A tower or truss picked on its
own gets its own editor here instead.

**Stage** — how you look at the rig. Switch between **Orbit** and **Fly**, jump
to a stored angle with the quick-view pads (Iso, Front, Back, Left, Right, Top
and a front-of-house seat), and **frame** either the selection or the whole rig.
Save the angle you like as a named camera, give it a number key, and press
**1…9** over the stage to fly back to it. The **Show** block turns individual
things on and off — floor grid, stage box, towers, truss, beams, floor pools,
name labels, the gizmo — with All / Clean / Plan presets, and the stage box's
own width, depth and height sit underneath.

**Build** — the rig itself. Eight pads build a **tower**, a straight **F34**
run, a **radius** run, or a whole **goalpost**, **box**, **ring**, **tower pair**
or **arch** in one press; the number keys **1…8** do the same. Standard lengths,
radii, arcs and heights are one tap each. Under the pads, an **outliner** lists
every element, folding a composite's parts under one heading with the lights
hung on them; click to select, rename, hide, mirror, duplicate or point the
camera at one. **Setups** at the bottom save and reload a whole named rig.

**Patch** — the fast way in. Pick a fixture from your **recents**, the built-in
profiles, or the library (search it, filter by moving head / par / bar / strobe),
and a preview card shows the mode, channel count, pan and tilt ranges and what
each channel does. Set a count and a start address — **next free** and **first
gap** find one for you, and it will skip over addresses already taken and keep a
fixture from straddling the two universes. Name them once and they are numbered
for you, continuing the series already in the rig. Then choose where they land:
a line, grid, arc, circle, on the floor, or hung straight onto a truss or tower,
which it will build to fit if you ask. Underneath, the **rig** is grouped by
type with how full each universe is, so you can add one more of something, select
them all, or unpatch them in two clicks.

---

## 3. First-time setup: patch & output

1. **Patch your fixtures.** DMXpress reads a *ShowBuddy* patch on startup. The
   top toolbar shows how many fixtures loaded (also in the **📜 Log**). If you
   edit the patch, click **Reload ShowBuddy patch**.
2. **Pick your universe.** Set **Universe** in the toolbar. Slots 1–512 go to
   that universe, 513–1024 to the next one.
3. **Find your nodes.** Click **ArtPoll** to discover Art-Net devices on the
   network. Open **📡 Art-Net** to see/configure output.
4. Leave **📜 Log** open while you learn — it narrates everything you do.

---

## 4. Step 1 — Select fixtures

You can only edit fixtures that are **selected**. Two ways:

- **Left "Fixtures" panel** — click a fixture name.
- **3D stage view** — click a light.

Selection modifiers (in the Fixtures list):

| Action | Result |
|---|---|
| **Click** | Select just this fixture |
| **Shift+click** | Add/remove this fixture from the selection |
| **⌘ Cmd+click** | Select all fixtures of the **same type** |

The number of selected fixtures drives what the **Channel control** grid shows.

The Fixtures grid shows each light as a tile in its live colour. The picker at
the top of the panel changes the tile style — **Lit** (plain colour chips),
**Amp jewel** (pilot-light jewels in chrome bezels), **Future** (chamfered
slabs with a glowing bar), **Glass**, **Chrome** or **Pillow** — and the
choice is remembered in `settings.json`.

---

## 5. Step 2 — Set values (the Programmer)

Below the stage is **Channel control**. This *is* your Programmer.

- **One fixture selected:** one slider per channel (Dimmer, Red, Pan…), each with
  quick **0** and **255** buttons.
- **Several fixtures selected:** one row per channel **type** (role), so dragging
  "Dimmer" moves the dimmer on *all* selected fixtures at once.
- **Blackout / Full** buttons set the selected fixtures to 0 / 255.

The moment you move a channel, it becomes **active in the Programmer** (DMXpress
remembers every channel you touch). Active channels are what get **Stored** and
what sit on top of playback. You'll see a running count like
`12 programmer ch active` in the Stacks window.

> Tip: clicking a channel **row label** "arms" it for the **🌊 Oscillator**
> window, where you can make it move on its own. Phasers (below) are the easy
> way to do this across a whole selection.

**Presets (Inspector → Presets):** click a pad to drop a whole-rig look straight
into the Programmer, and **Store look** to keep the one you are holding now.
Your ShowBuddy presets, if the show has any, are in the **ShowBuddy banks**
section of the same tab.

---

### The channel list, in detail

Under the stage, the selected light's channels (or, with several lights selected, each channel *type* across all of them) sit under headings in console order: Dimmer, Position, Color, Beam, Focus, Control, Other. Click a heading to fold it; its Arm / 0 / FL buttons act on the whole group. **By address** flattens the list into DMX order, and the filter box narrows it by name, tag, group or address - press Enter there to arm everything that matches.

Each row is a **level bar**: click anywhere on it and the level jumps there, keep the button down and it follows the pointer. Hold Shift for a fine drag (six pixels per step), Alt + wheel for ±1, Alt + Shift + wheel for ±10. Stepped channels (gobo and colour wheels) show their slot boundaries as ticks and their current slot name beside the bar.

Clicking a name **arms** the row. Armed channels feed the Oscillator window and the arrow keys: with the pointer over the list, ↑ ↓ (or → ←) nudge every armed channel by one, Shift by ten, PgUp / PgDn hop a whole slot on stepped channels, Home / End go to 0 / 255. Arming tactics: Shift-click for a range, Ctrl-click to toggle, Alt-click for every row of the same role, double-click for the whole group, **All / None / Invert** in the toolbar, **Similar** to add every row like the armed ones (same role, group, cell, coarse + fine partner, or name), and a right-click menu on any name with the same choices. The pill at the right of the toolbar shows how many channels are armed; **clear** disarms them all.

## 6. Step 3 — Groups (save selections)

Open **👥 Groups**. A Group is just a saved *selection* of fixtures — no values.

1. Select some fixtures.
2. Type a name → **Store**.
3. Later, click the group tile to re-select exactly those fixtures.

Build groups for the things you'll keep coming back to: `All Movers`,
`Front Wash`, `Odd`, `Even`, `Back Truss`, etc. Groups make everything below
faster.

---

## 7. Step 4 — Palettes (the heart of the workflow) ⭐

Open **🎨 Palettes**. **This is the most important pool to set up first**, so
take your time here.

### What a Palette is
A Palette is a **saved value for one feature**. "Feature" means a *category* of
channels:

| Tab | Covers these channels |
|---|---|
| 🔆 **Dimmer** | Intensity |
| ✛ **Position** | Pan / Tilt (incl. fine) |
| 🎨 **Color** | Red, Green, Blue, White, color wheels |
| 🔦 **Beam** | Strobe / shutter |
| 🔍 **Focus** | Zoom |
| ⚙ **Control** | Speed / everything else |

Because a Color palette only stores color, you can recall "Blue" without
disturbing where the lights are pointing — and a Position palette moves the
lights without changing their color. That separation is what makes a console
fast.

### Why Palettes are *referenced* (the magic)
When you record a cue using a palette, the cue remembers **"Color = Blue
palette"**, not the raw numbers. Re-point or re-tint the palette later and your
cues follow. (See §13 for the exact tracking rule.)

### Setting up your palettes — do this once
For each feature, build the handful you'll reuse:

**Color palettes**
1. Select some fixtures (or a Group).
2. In Channel control, dial Red/Green/Blue to a color (e.g. full Blue).
3. In Palettes, click the **🎨 Color** tab, type `Blue`, press **Store**.
4. Repeat for `Red`, `Green`, `Amber`, `White`, `Cyan`, `Magenta`…

**Position palettes** (moving heads)
1. Select your movers.
2. Drag Pan/Tilt until they point where you want (watch the 3D stage).
3. Click the **✛ Position** tab, name it `Center` (or `Audience`, `Drums`,
   `Upstage`…), **Store**.
4. Build 4–8 positions you'll actually use.

**Dimmer / Beam / Focus palettes** work the same way (e.g. Dimmer `Full`,
`Half`; Focus `Tight`, `Wide`).

### Using palettes
- **Click a palette tile** → recalls it into the Programmer for the selected
  fixtures.
- **Right-click a tile** → **Recall / Update / Delete** (Update overwrites it
  with the current Programmer values).
- The **active-channel count** and **Clear** button (top of the window) control
  the Programmer — **Clear** empties it (releases everything you were editing).

> A normal look = pick a Group, recall a Color palette, recall a Position
> palette, set a Dimmer. Three clicks.

---

## 8. Step 5 — Phasers (movement & effects)

Open **🌈 Phasers**. A Phaser makes a feature **move on its own** across your
selection — chases, sine sweeps, rainbows.

The editor at the top builds one phaser:

| Control | What it does |
|---|---|
| **Feature** | Which channels it drives (Dimmer, Position, Color…) |
| **Amount** | Depth of the effect (how far it swings) |
| **Shape** | Waveform character |
| **Rate** | Speed, as a musical subdivision (or **Free** for a raw speed) |
| **Spread** | Spacing, from order to chaos: 0 keeps every light together, 1 fans one wave across the fixtures, 2 scatters them so every light is on its own phase. The Palettes cycle's Spacing (and the deck's SP keys) work the same way |
| **Wings** | Mirror the spread into 1–8 symmetric groups |
| **Invert** | Flip direction |

Workflow:
1. Select fixtures.
2. Dial the editor, then **▶ Apply to selection** (you'll see them start moving).
3. Name it → **Store** to keep it in the tile grid.
4. Click a stored tile to **load + apply** it again later; right-click for
   Apply / Update / Delete.
5. **Clear FX** stops the live oscillation.

DMXpress ships with starter phasers: **Dimmer Chase**, **Dimmer Wings**,
**Color Rainbow**, **Pan Sweep** — load one to see how the controls feel.

> A Phaser is part of the Programmer too, so it gets recorded when you Store a
> cue (below).

---

### Design here, play from the board

The Phasers window is the workbench: pick the channels to oscillate, shape the motion, Apply it to the selection, and **Store** it (or **Store to board**). Its **Library** lists everything stored, one line each, with a search box and filters (Light, Movement, Snapshots, Running). Click a name to edit it in place, the play button starts it, and **+ Board** puts it on a pad.

The **Board** window (toolbar → Board) is where you play: a page of pads laid out exactly like the Stream Deck's Phaser page, with ALL, CLR and TAP in the same corners. Press a pad to start or stop its phaser; a running pad glows with a white ring. **Arrange** lets you click a pad to assign a phaser, symbol and colour, drag pads onto each other to swap them, and right-click to clear; **Fill...** lays in the movement presets and path shapes. Pop the board out into its own window and keep designing in the Phasers window beside it - anything you store to the board appears on the next free pad, and on the deck.

## 9. Step 6 — Stacks (record cues & play them back)

Open **🎬 Stacks**. A Stack is a **cue list**: an ordered set of looks you step
through with **Go**.

### Build a stack
1. Click **＋ New** and give the stack a name (e.g. `Main`).
2. Build a look in the Programmer (Group + palettes + dimmer + maybe a phaser).
3. Set a **Fade** time (seconds).
4. Press **⏺ Store cue**. That snapshots the **active Programmer channels** into
   Cue 1.
5. Change the look, **Store cue** again → Cue 2. Repeat.

### Tracking (why only active channels are stored)
A cue only records the channels you **touched** (the active Programmer). Channels
you didn't touch **track** through from earlier cues — exactly like a real
console. This keeps cues small and edits painless.

### Record mask (record filter)
The **Record:** row of feature toggles (🔆 ✛ 🎨 🔦 🔍 ⚙) decides which features
**Store** captures. Turn off ✛ Position to store a *color-only* cue, for
instance. All on = record everything.

### Play a stack
- **⏭ Go** advances to the next cue (wraps at the end), fading over its time.
- The cue grid lists every cue (`# / Name / Fade / ▶ go / 🗑`); click **▶** to
  jump straight to a cue. The current cue is highlighted green.
- **Clear prog** empties the Programmer so the stack's output shows through (do
  this after recording, or you'll only ever see your scratch pad!).

---

## 10. Step 7 — Decks (the executor bar)

Open **🎚 Decks**. A bottom strip appears with:

- **GM (Grand Master)** on the left — one fader that scales **every dimmer** in
  the rig. Pull it down for a quick fade-to-black of intensities.
- **One strip per stack**, each with:
  - the stack name + current cue label,
  - a **level fader** (fade the whole cue list in and out),
  - **Go** (advance cues), **Off** (release the stack so it stops outputting).

This is your live playback surface: faders up, press Go, mix between stacks.

---

## 11. Step 8 — Command line

Open **⌨ Cmd**. A single text box at the very bottom. Type a verb, press
**Enter**. Faster than the mouse once you know it:

| Command | Does |
|---|---|
| `clear` | Release the Programmer |
| `black` / `bo` | Blackout — release every stack **and** clear the Programmer |
| `go` | Go on the current stack |
| `go 2` | Go on stack #2 |
| `off` / `off 2` | Release all stacks / release stack #2 |
| `store` | Store a cue into the current stack |
| `cue 3` | Jump the current stack to cue position 3 |
| `group 1` | Recall (select) Group #1 |
| `gm 80` | Grand Master to 80% |
| `full` | Grand Master to 100% |

(Numbers are 1-based, in pool order. Unknown input is reported in the Log.)

---

## 12. Step 9 — Views (window layouts)

Open **🗂 Views**. Arrange your windows the way you like (e.g. Palettes +
Stacks + Decks for playback), type a name, **＋ Save**. Click a saved view to
snap every window back to that arrangement. Make a `Program` view and a
`Playback` view and flip between them.

---

## 13. The Programmer & "Clear" — read this twice

This trips up every newcomer, so here it is plainly:

- The **Programmer** holds every channel you've touched since the last **Clear**.
- The Programmer is an **overlay**: it sits **on top of** all playback (Stacks).
  While a channel is in the Programmer, *you* control it, not the cues.
- **Store** (palette or cue) copies the Programmer; it does **not** clear it.
- **Clear prog** (Stacks/Palettes window) or `clear` (command line) releases the
  Programmer so your Stacks/Decks take over the output.

**Golden rule:** after you record cues, press **Clear prog**. If your faders and
Go button "do nothing", it's almost always because the Programmer is still
holding those channels — Clear it.

### How palette references track
Cues store palette **references**, resolved **when you fire the cue** (press Go
or ▶). So if you edit a palette, **re-fire** the cue to pick up the change. A cue
sitting live on stage won't change underneath you until you Go to it again.

---

## 14. Worked example — build a tiny show

A complete run-through. Assumes a few moving heads are patched.

**A. Groups**
1. Select all movers → **👥 Groups** → name `Movers` → **Store**.

**B. Palettes**
2. With `Movers` selected, dial full **Blue** → **🎨 Palettes** → Color tab →
   `Blue` → **Store**. Dial **Red** → store `Red`.
3. Point them at the crowd (Pan/Tilt) → Position tab → `Audience` → **Store**.
   Point them centre-stage → `Center` → **Store**.

**C. Cue 1 (a static blue look)**
4. **🎬 Stacks** → **＋ New** → name `Main`.
5. `group 1` (or click the Movers group) → click **Blue** → click **Center** →
   set Dimmer **Full**.
6. Fade `3` s → **⏺ Store cue**. That's Cue 1.

**D. Cue 2 (red, pointed at the crowd)**
7. Click **Red** → click **Audience** → **⏺ Store cue**. That's Cue 2.

**E. Cue 3 (add movement)**
8. **🌈 Phasers** → load **Pan Sweep** → **▶ Apply to selection** → back to
   Stacks → **⏺ Store cue**. Cue 3 now contains the sweep.

**F. Play it**
9. **Clear prog** (important!).
10. **🎚 Decks** → push the `Main` fader up → press **Go** to step Cue 1 → 2 → 3,
    each fading over its time. Pull **GM** down to dim the whole rig.

You just programmed and played a 3-cue show. Everything is saved to disk
automatically (see below) — reopen DMXpress and it's all still there.

---

## 15. Reference

### Toolbar buttons
`Universe` · `ArtPoll` · `Reload ShowBuddy patch` ·
📡 Art-Net · 🌊 Oscillator · ⏱ Transition · 🌀 Chases · 👥 Groups ·
🎨 Palettes · 🌈 Phasers · 🎬 Stacks · 🎚 Decks · ⌨ Cmd · 🗂 Views ·
📜 Log · ⚙ Settings

### Other tools
- **⏱ Transition** — crossfade the Programmer smoothly from its current look to a
  new one (great for busking).
- **🌀 Chases** — quick step-based chases across fixtures.
- **🌊 Oscillator** — hand-build per-channel motion on channels you've "armed"
  from the Channel control rows.

### Where your show is saved (workspace folder)
| File | Contents |
|---|---|
| `groups.json` | Your Groups |
| `palettes.json` | Your Palettes |
| `phasers.json` | Your Phasers |
| `stacks.json` | Your Stacks & cues |
| `views.json` | Your saved Views |

Plus the ShowBuddy patch/presets DMXpress loads on startup.

### Backups, export and undo

- **Undo anything.** `Ctrl+Shift+Z` (`Cmd+Shift+Z` on a Mac) takes back the last thing you did anywhere in the console - a slider, a stored palette, a recorded cue, a light you moved, a show you loaded - up to ten steps. `Ctrl+Shift+Y` redoes. The two arrows at the left of the toolbar do the same and show how many steps are left. A drag or a typed name counts as one step.
- **Automatic backups.** Every minute in which something changed, and when you quit, the whole show is written to `backups/` next to your show files; the newest 20 are kept. Open **Configs** to see them, back up on demand, or **Restore** one. A restore backs up the current show first, and can itself be undone.
- **Move a show.** **Configs → Export show...** writes the whole show as one file into `exports/` and opens the folder. On another machine, drop that file onto the DMXpress window (or paste its path in **Configs → Import...**) to load it. The show being replaced is backed up first.

### Suggested order of operations
1. Patch & Art-Net → 2. Groups → 3. **Palettes** (color + position first) →
4. Phasers → 5. Stacks (record cues) → 6. Decks (play) → 7. Views (save layout).

---

## 16. Tips & gotchas

- **"My playback does nothing."** Clear the Programmer (§13).
- **"My palette edit didn't change the cue."** Re-fire the cue (§13).
- **Store records only active channels.** If a cue is missing something, make
  sure you actually *touched* that channel (and the **Record** mask allows it).
- **Grand Master only dims dimmers** — fixtures without a dimmer channel (raw RGB)
  won't dim from GM; use a Dimmer palette or the channel sliders instead.
- **Build Groups and Palettes first.** Ten minutes of setup makes the rest of the
  night a few clicks.
