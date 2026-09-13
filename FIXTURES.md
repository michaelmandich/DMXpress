# Adding & Editing Fixture Definitions

**Most fixtures don't need any of this.** The **🔌 Patch** window ships with a
searchable library of ~2,100 fixture modes from ~630 models across 133
manufacturers, so if your light is from a known brand, search for it there and
patch it — no code, no rebuild. The rest of this document covers hand-writing a
built-in profile, which is now only needed for something the library doesn't
have (a no-name fixture, or a custom channel layout).

## The fixture library

The library is `fixtures/library.json`, generated from the
[Open Fixture Library](https://github.com/OpenLightingProject/open-fixture-library)
(MIT) by `tools/ofl_to_library.py`. Unlike the hand-written profiles below,
library fixtures carry an explicit `role` per channel taken from OFL's own
capability data, so Amber/UV/CMY/gobo/prism channels are classified outright
instead of guessed from the channel name.

To refresh it against upstream OFL:

```sh
curl -L -o ofl.tar.gz \
    https://github.com/OpenLightingProject/open-fixture-library/archive/refs/heads/master.tar.gz
tar xzf ofl.tar.gz --strip-components=1 open-fixture-library-master/fixtures
python3 tools/ofl_to_library.py fixtures/ fixtures/library.json
cargo test fixturedb    # checks every entry is still patchable
```

Known gap: modes built from a pixel matrix (LED battens addressed per-pixel)
are skipped, because expanding OFL's template-channel blocks needs the matrix
geometry. Those fixtures still appear if they have a non-matrix mode.

## Hand-written profiles

Built-in fixture profiles live in one file: **`src/profiles.rs`**.
After any change, rebuild and relaunch:

```sh
cargo test    # sanity checks on the profiles
cargo run     # rebuilds and starts DMXpress
```

New profiles show up automatically in the **🔌 Patch** window, under
"Built-in profiles" — no other file needs to change.

---

## Anatomy of a profile

Every fixture has two parts in `src/profiles.rs`:

**1. An entry in the `PROFILES` list** (near the top of the file):

```rust
Profile {
    name: "Generic RGBW Par (4ch)",   // shown in the Patch dropdown
    pan_range: 0.0,                   // total pan travel in degrees (0 = no movement)
    tilt_range: 0.0,                  // total tilt travel in degrees
    beam_width: 30.0,                 // beam cone width in degrees (3D view)
    build: generic_rgbw_par,          // the channel-builder function below
},
```

**2. A channel-builder function** (lower in the file) that lists the DMX
channels **in order, one per DMX address**, straight off the fixture's
DMX chart:

```rust
/// Plain 4-channel RGBW par can.
fn generic_rgbw_par() -> Vec<Channel> {
    vec![v("Red"), v("Green"), v("Blue"), v("White")]
}
```

---

## The three channel helpers

| Helper | Use for | Example |
|--------|---------|---------|
| `v("Name")` | Continuous 0–255 channel (most channels) | `v("Zoom")` |
| `d("Name")` | The master **dimmer** channel | `d("Dimmer")` |
| `s("Name", &[...])` | Stepped/switched channel with labelled ranges | see below |

`s()` takes `(min, max, "Label")` ranges that cover 0–255. The labels show
up in the channel editor so you know what a value does:

```rust
s("Strobe", &[
    (0, 3, "Off"),
    (4, 7, "On"),
    (8, 76, "Strobe"),
    (77, 145, "Pulse"),
    (146, 215, "Random"),
    (216, 255, "Off"),
]),
```

---

## Channel names matter (auto-classification)

DMXpress figures out what each channel *does* from its **name** (case
doesn't matter). Use these words so colors, movement, and dimming work in
the 3D view, palettes, and phasers:

| To get | Put this in the name | Examples from existing profiles |
|--------|----------------------|--------------------------------|
| Pan / Tilt | `pan`, `tilt` (add `fine` for fine channels) | `v("Pan")`, `v("Tilt fine")` |
| Dimmer | use `d(...)`, or name contains `dim`/`master` | `d("Dimmer")` |
| Red/Green/Blue/White | `red`, `green`, `blue`, `white` | `v("Red 1")`, `v("BackColor W")` → use `White` if you want it classified |
| Amber / UV | `amber` / `amb`, `uv` | `v("Amber")`, `v("UV")` |
| Cyan / Magenta / Yellow | `cyan`, `magenta`, `yellow` | `v("Cyan")` |
| Color wheel | `color` / `colour` / `clr` | `s("Color wheel", ...)` |
| Strobe | `strobe` / `strb` | `s("Strobe", ...)` |
| Shutter | `shutter` / `shut` | `s("Shutter", ...)` |
| Zoom / Focus / Iris | `zoom`, `focus`, `iris` | `v("Zoom")` |
| Gobo / Prism / Frost | `gobo`, `prism`, `frost` | `s("Gobo wheel", ...)` |
| Speed | `speed` / `spd` | `v("Pan/Tilt speed")` |
| Nothing special | anything else | `v("Control")`, `v("Reset")` |

**Watch out:** `strobe`/`speed` win over `pan`/`tilt` (so
`"Pan/Tilt speed"` is a Speed channel, which is correct), and the beam words
(`gobo`, `prism`, `iris`, `frost`, `focus`) are checked before the colour
words, so `"Gobo colour"` is a Gobo channel. A channel named
`"Color macros"` will be treated as a Color channel — that's usually what
you want.

Name guessing only applies to hand-written profiles. Library fixtures set
`Channel::role` explicitly, and an explicit role always wins.

---

## Recipe: add a new fixture

Say you bought a "FooBar Wash 250" that runs in a 7-channel mode:
`Pan, Tilt, Dimmer, Red, Green, Blue, Strobe`.

**Step 1** — add a builder function near the other ones (e.g. right after
`generic_rgbw_par`):

```rust
/// FooBar Wash 250, 7-channel mode.
fn foobar_wash_250() -> Vec<Channel> {
    vec![
        v("Pan"),
        v("Tilt"),
        d("Dimmer"),
        v("Red"),
        v("Green"),
        v("Blue"),
        s("Strobe", &[
            (0, 9, "Open"),
            (10, 249, "Strobe"),
            (250, 255, "Open"),
        ]),
    ]
}
```

**Step 2** — add its entry to the `PROFILES` list (order in the list =
order in the dropdown):

```rust
Profile {
    name: "FooBar Wash 250 (7ch)",  // convention: include channel count
    pan_range: 540.0,               // from the manual's specs page
    tilt_range: 270.0,
    beam_width: 22.0,               // eyeball it; 18–30 is typical
    build: foobar_wash_250,
},
```

**Step 3** — update the test at the bottom of `src/profiles.rs`. The
`channel_counts_match_modes` test lists every profile with its expected
channel count — add yours or `cargo test` will fail:

```rust
                ("SHEHDS Bee Eye 19x40 Ring (31ch)", 31),
                ("Generic RGBW Par (4ch)", 4),
                ("FooBar Wash 250 (7ch)", 7),   // <-- add this line
```

**Step 4** — `cargo test` then `cargo run`. Patch it via **🔌 Patch**.

The number of channels in the builder is the number of DMX addresses the
fixture occupies — the `(7ch)` in the name is just a human hint, but keep
it accurate.

---

## Recipe: edit an existing fixture

Example: your Bee Eye actually runs a different strobe mapping. Find
`fn shehds_bee_eye_19x40()` and edit the `s("Strobe", ...)` ranges — the
order and count of channels must still match the fixture's actual mode.

If you switch a fixture to a different channel-count mode (say the
Intimidator Spot's 20-channel mode instead of 16), you must:
1. Rewrite the builder to match the new chart exactly, channel by channel.
2. Update the `(16ch)` in the profile name.
3. Re-patch: fixtures already patched keep their old channel list until
   you remove and re-add them in the **🔌 Patch** window (their saved
   entry references the profile by name).

**Renaming a profile:** already-patched fixtures in `patch_user.json`
reference profiles by exact name. If you rename one, either keep the old
name, or open `patch_user.json` and update the `"profile"` fields to
match, or re-patch the lights.

---

## Movement numbers cheat-sheet

- `pan_range` / `tilt_range`: *total* travel in degrees from the manual
  (e.g. "540° pan / 270° tilt"). Static fixtures (pars): `0.0` for both.
- `beam_width`: the visual cone angle in the 3D stage. Narrow beam ≈ 15–20,
  spot ≈ 20–25, wash/par ≈ 25–35.

---

## Checklist before you're done

- [ ] Channel list matches the manual's DMX chart for that mode, in order.
- [ ] Exactly one `d("Dimmer")` (or dimmer-named) channel if it has one.
- [ ] Names use the keywords above so RGB/pan/tilt/strobe classify right.
- [ ] Profile `name` ends with the channel count, e.g. `(7ch)`.
- [ ] Added the profile to the `channel_counts_match_modes` test list.
- [ ] `cargo test` passes (it checks channel counts and roles).
- [ ] Patched a test unit in **🔌 Patch** and wiggled it in the 3D view.
