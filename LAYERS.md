# Layers: how a DMX channel gets its value

Every frame, DMXpress rebuilds all 1024 output channels from scratch. Nothing
is stored on the wire; each value is the result of a fixed stack of layers
applied bottom to top. This document is the reference for that stack: what
the layers are, in what order they apply, how each one folds into the ones
beneath it, and where a plugin can insert itself.

The code lives in two places:

- `src/engine.rs` is the **mixer**: a list of weighted, per-channel layers
  flattened into one frame.
- `App::update` in `src/app.rs` builds the mixer stack every frame, then runs
  a second set of **post-mix passes** over the flattened frame before
  handing it to the network thread.

## The stack at a glance

Read bottom to top. A higher slot beats a lower one on every channel it
asserts; everything it does not assert falls through.

```
            +----------------------------------------------------------------+
post-mix    | 16  Blackout (Master Dimmer press)          whole frame -> 0  |
passes      | 15  DMX test window                         replace, raw      |
(raw ops    | 14  Hold phasers                            replace, raw      |
 on the     | 13  Flat-add phasers                        +/- add, capped   |
 flattened  | 12  Dim-range compression                   remap dimmers     |
 frame)     | 11  Dimmer throb                            max on dimmers    |
            | 10  Grand master                            x on dimmers      |
            |  9  Encoder layer (Stream Deck knob)        replace           |
            +----------------------------------------------------------------+
            |                   `mixed` = Mixer::render()                    |
            +----------------------------------------------------------------+
            |  8  Chase                                   mix, band weight  |
mixer       |  7  Audio triggers                          per-trigger blend |
layers      |  6  Effect lanes (gobo, prism...)           mix               |
(weighted,  |  5  Palette cycle                           mix, fade weight  |
 masked,    |  4  PROGRAMMER (live look)                  mix, active mask  |
 blended)   |  3  Plugins   <- the only plugin slot today                   |
            |  2  Scenes                                  per-scene blend   |
            |  1  Stacks (cue lists)                      mix, fader weight |
            |  0  Black                                   floor             |
            +----------------------------------------------------------------+
                                            |
                                            v
                               `net.dmx` (the wire buffer)                    
                  Art-Net / sACN sender, 3D stage, fixture list, channel      
                 sliders and the Stream Deck fixture page all read this.      
```

Two gates sit outside the stack entirely:

- **Freeze** (`frozen`) skips the whole rebuild. The last frame keeps
  transmitting and no clock advances.
- **Time machine and master BPM** are not layers. They are clocks that every
  animated layer (programmer oscillators, scenes, chase, cycle) reads.

## Part 1: mixer layers (slots 0 to 8)

Each mixer layer is a full 1024-channel frame plus a list of
`(address, weight)` pairs and a blend mode. Only the listed addresses are
touched. The mixer starts from black and merges each layer in push order.

Blend modes (`engine::Blend`):

| Blend | Script header | Behaviour on one channel |
|---|---|---|
| `Mix` | `mix` | Crossfade from what is beneath toward this value by `weight`. At weight 1 it replaces outright (LTP). |
| `Max` | `highest` | Keep the brighter of beneath and `value × weight` (HTP). |
| `Add` | `add` | Add `value × weight` to beneath, saturating at 255. |

The slots, bottom to top:

| Slot | Layer | Source file | Order inside the slot | Channels asserted | Weight | Blend |
|---|---|---|---|---|---|---|
| 0 | Black | `engine.rs` | | all | | floor |
| 1 | Stacks | `stack.rs` | pool order | every channel any cue up to the current one touches (tracking) | stack fader level | Mix |
| 2 | Scenes | `scene.rs` | pool order, later wins | the scene's captured channels | level × fade-in | per scene: Override / Highest / Add |
| 3 | Plugins | `plugin.rs` | filename order (sorted) | whatever `frame()` returns | 1.0 | per script header |
| 4 | Programmer | `oscillator.rs`, `transition.rs` | one layer | `live_active` only | 1.0 | Mix |
| 5 | Palette cycle | `app.rs` | one layer | union of every palette in the cycle | cycle fade in/out | Mix |
| 6 | Effect lanes | `app.rs`, `wheels.rs` | island name order | the lane's palette channels | 1.0 | Mix |
| 7 | Audio triggers | `audio.rs` | list order | resolved palette or preset values on the trigger's group | envelope (gate, pulse or level) | per trigger, default Highest |
| 8 | Chase | `chase.rs` | one layer | every fixture, weighted by band coverage | 0..1 per fixture, soft edge optional | Mix |

Notes on the important ones:

**Stacks (1)** are the floor of playback. A stack asserts every channel any of
its cues up to the current one has ever set, so a released channel stays at
the last cue's value until the stack is released. The Decks fader is the
weight, so pulling a fader down reveals whatever is beneath, which is black
unless another stack covers the channel.

**Scenes (2)** stack among themselves in pool order. An Override scene at
full level hides every stack and scene beneath it on its channels.

**Plugins (3)** ride just above scenes and just below the programmer. This
is the only place a script can paint today. Anything the desk is actively
holding wins over a plugin.

**Programmer (4)** is the `live` look: a base frame plus per-channel wave
oscillators (Wave-mode phasers) that swing around the base. Palette recalls
and pose fades ramp the base through `base_fades`. During a transition the
programmer source is the transition run instead, which blends the outgoing
look toward the incoming one per channel window. Two things about its mask:

- It asserts only `live_active` channels. Touch a slider, recall a palette,
  apply a phaser: that channel joins the mask. Clear empties it.
- A **preset recall (ShowBuddy or native) sets the mask to all 1024
  channels.** Until Clear, the programmer then covers every stack, scene and
  plugin, including channels the preset leaves at zero.

**Palette cycle (5)** and **effect lanes (6)** step through palettes on the
beat clock. A lane with a single pick simply holds that palette at full
weight over the programmer.

**Chase (8)** is the top of the mixer: the injected look, weighted by how
much of each fixture the moving band covers this frame.

The flattened result is kept as `App::mixed`. The Stream Deck encoder reads
it to know what value it starts nudging from.

## Part 2: post-mix passes (slots 9 to 16)

After the mixer, `update` walks the flattened frame with raw operations. These
are not weighted layers; each is a specific arithmetic pass, applied in this
order:

| Slot | Pass | State | Channels | Operation | Cleared by |
|---|---|---|---|---|---|
| 9 | Encoder layer | `encoder_layer` | channels dialled with the Stream Deck programmer knob | replace | Clear (stage 1), storing a preset bakes it in |
| 10 | Grand master | `grand_master` | every channel whose role is Dimmer | multiply by 0..1 | `full` / GM fader |
| 11 | Dimmer throb | `throb_at` | Dimmer-role channels | `max(out, boost)` decaying over 0.5 s | time |
| 12 | Dim-range compression | fixture profile `dim_range` | Dimmer channels whose usable band is not 0..255 | remap logical 0..255 into `lo..hi` (0 stays 0) | never; part of the profile |
| 13 | Flat-add phasers | `add_overrides` | channels the Add-mode phaser targets | signed add, clamped to 0..dim cap | phaser tile off, Clear (stage 2) |
| 14 | Hold phasers | `hold_overrides` | channels the hold stores | replace, **raw** (bypasses slot 12) | tile off, Clear (stage 2), Blackout all |
| 15 | DMX test window | `test_overrides` | channels forced in the window | replace, raw | closing the window |
| 16 | Blackout | `blackout` | all | whole frame to 0 | releasing the Master Dimmer key |

Why the order is what it is:

- The **encoder sits under the grand master** so a dimmer dialled by hand
  still fades with the rig.
- **Grand master only touches Dimmer-role channels.** RGB-only fixtures do
  not dim with it.
- **Dim-range compression comes after the master and throb** so every
  logical value above (faders, phasers, plugins, cues) lands inside the
  fixture's dimming band and never strays into an embedded strobe or macro
  range. Holds and the test window come *after* it and are deliberately raw:
  they are the way into those bands on purpose.
- **Holds beat blackout fades and the grand master** because they exist for
  things like a smoke machine that must stay on.
- **Blackout is a non-destructive override**, not a clear. Lifting it
  resumes the show exactly where it was.

The result is written once to `net.dmx`. The network thread snapshots that
buffer on its own clock, which is why the whole frame is composed locally
first: a half-built frame must never be visible to the sender.

## Side paths that touch the wire directly

A few UI actions write `net.dmx` straight away instead of waiting for the
next frame. They are all overwritten by the next `update`, so they only
matter for the one frame they are painted, or while frozen:

- Channel sliders write the buffer immediately and also copy the change into
  the programmer base and mask, so the next frame reproduces it.
- **Blackout all** (Inspector panel, under Presets) writes black, empties
  the programmer, stops the chase and transition and clears holds.

Also note that when a palette recall, a gobo click or the Transition
window's "stop at current" interrupts a running transition, the programmer
base is re-seeded from **the wire buffer**, so it captures the grand master
and any post-mix pass that was in effect at that moment.

## The Clear ladder

Clear peels the stack from the top down, one stage per press:

1. **Encoders**: empty the encoder layer (slot 9).
2. **Effects**: stop oscillators, phasers, holds and adds (slots 4 wave
   part, 13, 14).
3. **Blackout**: release every stack and scene, stop the cycle and lanes,
   clear the programmer (slots 1 to 6). The command line `black` / `bo` does
   this directly.

## Plugins: where they sit and where they could

### Today

A plugin's `frame(t, rig, state)` returns `[address, value]` pairs. Those
become one mixer layer at **slot 3**, weight 1.0 on every returned address,
blended with the `//! blend:` header (`mix` default, `highest`, `add`).
Several enabled plugins push their layers in filename order, so `a_*.rhai`
sits beneath `b_*.rhai`.

What that means in practice for a script author:

- You paint *logical* values. A dimmer at 255 lands at the fixture's usable
  maximum after slot 12, not in its strobe band.
- The grand master, throb, holds and the test window all still apply over
  you.
- The programmer wins on every channel it is holding, and after a preset
  recall that is every channel. Scenes and stacks show through wherever you
  do not paint.
- `t` is wall-clock seconds since the app started. It does not follow the
  time machine or master BPM, and it pauses only because `update` stops
  calling you while frozen.

### Proposed insertion points

To let a plugin choose its place, add one header:

```
//! slot: <name>
```

The plugin's layer is pushed immediately **above** the named slot. Names
follow the table in this document. The default stays `scenes`, which is
exactly today's behaviour.

Mixer slots a script could target with the existing `frame()` hook:

| `slot:` value | Sits above | Typical use |
|---|---|---|
| `black` | nothing; beneath stacks | an ambient base that every cue overrides |
| `stacks` | cue lists, beneath scenes | a bed that scenes can layer on |
| `scenes` (default) | scenes, beneath the programmer | effects the desk can always override |
| `programmer` | the programmer | an effect that rides over what the operator is holding |
| `cycle` | the palette cycle | a colour effect that follows but overrides the cycle |
| `lanes` | effect lanes | gobo / prism / beam effects above the wheel lanes |
| `audio` | audio triggers | an effect that must survive sound-reactive flashes |
| `chase` | everything in the mixer | the top of the mix, still under masters and holds |

Mixer slots can only paint values. A plugin that wants to *scale* or
*remap* the frame (a per-group master, an inverter, a curve, a limiter)
needs the flattened frame as input. That is a second, optional hook:

```
//! slot: master        // any post-mix slot name
fn post(t, out, rig, state) { ... }   // `out` is the 1024-value frame
```

`post` returns the same `[address, value]` shape as `frame`, but the values
replace the frame outright at that point in the pass order, with no blend.
It runs after the named pass:

| `slot:` value | Runs after | Sees |
|---|---|---|
| `mixed` | the mixer, before the encoder | logical values, no masters yet |
| `encoder` | the encoder layer | |
| `master` | the grand master | dimmers already scaled |
| `throb` | the throb | |
| `dimrange` | dim-range compression | **raw** fixture values from here on |
| `add` | flat-add phasers | |
| `hold` | hold phasers | |
| `test` | the DMX test window | last word before blackout |

`blackout` is not a slot. Nothing sits above it.

Two more headers would close the gaps script authors hit first:

- `//! weight: <control id>`: use one of the plugin's own sliders as the
  layer weight instead of a fixed 1.0, so a plugin gets a fader for free.
- Pass `beat` and `bpm` into `frame()` alongside `t`, so a script can lock
  to the master clock and the time machine like every built-in layer does.

None of this is implemented yet; the parsing lives in `plugin::parse_meta`
and the push happens in the one plugin loop inside `App::update`. Moving
that loop into a small "push the plugins for slot X" helper called at each
slot boundary is the whole change for the mixer half.
