# Gobos in the visualizer

The 3D stage renders a fixture's beam through the picture of the gobo it is
showing: the haze inside the cone breaks into the gobo's rays, and the pool
where the beam lands on the stage or floor is the projected image, rotated
by the fixture's gobo rotation channel.

## Where the pictures come from

`fixtures/gobos.tar.gz` is the gobo catalogue. It is built by
`tools/gobo-pack` from two open sources and bundled into the app the same
way the fixture library is:

- **QLC+** (`resources/gobos/`, Apache-2.0): about 1,000 gobo images filed
  by maker (Chauvet, Clay Paky, GLP, Robe, SGM, and a generic set), plus its
  fixture definitions, which say which image sits in which DMX range of which
  channel for about 390 models.
- **Open Fixture Library** (`resources/gobos/`, MIT): a smaller named set
  with keywords, and the handful of OFL fixtures that reference them.

Inside the pack:

| File | What it is |
|------|------------|
| `index.json` | One entry per gobo: key, display name, maker, keywords. |
| `wheels.json` | Per maker and model: channel name → DMX ranges → gobo key. |
| `<key>.png` | The mask, 256×256 8-bit grey, **white where light passes**. |

Keys look like `qlc/Chauvet/gobo00079` or `ofl/stars`.

## How a light gets its gobos

When the patch is built, every gobo wheel channel of every fixture is joined
against `wheels.json` by maker and model (library fixtures know theirs; a
built-in profile or ShowBuddy light matches on the model name its profile
contains). Each stepped band on the wheel takes the range that covers most
of it. The result lands on `Band::gobo` in memory; nothing is written back
to the library.

Then the **Gobos** window's own assignments are applied on top (see below).

A slot with no picture projects a plain cone, exactly as before.

## The Gobos window

Open it from the toolbar. It lists the selected lights grouped by type, one
row of picture tiles per gobo wheel, with the slot each light is currently
on outlined. With nothing selected it lists every type in the rig.

- **Click a tile** to send those lights to that slot. The wheel channel goes
  into the programmer, the same as recalling a palette.
- **✎ Assign pictures**, then click a tile, to choose what that slot
  projects. The picker searches the whole catalogue by name, keyword and
  maker. *No gobo* makes the slot a plain beam; *Auto* forgets your choice.

Assignments are saved in `gobos_user.json`, keyed by fixture type
(`lib:…`, `builtin:…`, or the ShowBuddy file), so every light of that model
picks them up, now and after a repatch.

## Adding your own gobo pictures

Drop a PNG into `fixtures/gobos/` (subfolders are fine). It appears in the
picker under *Custom* as `user/<file name>`. Any PNG works: a grey mask, or
a photo of the pattern the light throws on a dark wall. Dark opaque pixels
block light; bright or transparent pixels pass it. Other sizes are resampled
to 256×256.

## Rebuilding the catalogue

```sh
curl -L -o qlc.tar.gz https://github.com/mcallegari/qlcplus/archive/refs/heads/master.tar.gz
curl -L -o ofl.tar.gz https://github.com/OpenLightingProject/open-fixture-library/archive/refs/heads/master.tar.gz
mkdir -p build/qlc build/ofl
tar xzf qlc.tar.gz --strip-components=1 -C build/qlc --wildcards "qlcplus-master/resources/fixtures/*" "qlcplus-master/resources/gobos/*"
tar xzf ofl.tar.gz --strip-components=1 -C build/ofl --wildcards "open-fixture-library-master/fixtures/*" "open-fixture-library-master/resources/*"
cargo run --release --manifest-path tools/gobo-pack/Cargo.toml -- \
    --qlc build/qlc --ofl build/ofl --out fixtures/gobos.tar.gz --size 256 --levels 8
cargo test gobo
```

`--levels 8` keeps eight grey levels per mask, which compresses the pack to
about 6.6 MB (full 8-bit is 16 MB; the GPU's bilinear filter smooths the
steps out). `--sheet debug.png` writes a contact sheet of raw renders next to
their masks for checking a source's drawing convention.

`tools/gobo-pack` is its own crate so the app never depends on an SVG
rasterizer.

## Checking the rendering without opening the app

Three tests exercise the GPU path on whatever adapter the machine has, and
skip with a note where there is none:

```sh
cargo test volumetric headless -- --nocapture --test-threads=1
```

- `volumetric::tests::shader_validates` parses and validates the WGSL the
  way wgpu does, so a shader mistake fails here instead of at launch.
- `volumetric::tests::renders_gobo_beam_and_pool_offscreen` renders one
  striped gobo beam and one plain beam and writes `target/gobo_render.png`.
- `headless::tests::stage_renders_the_local_rig_headless` and
  `gobos_window_renders_headless` run the real stage widget and the real
  Gobos window on the rig patched in this directory, through egui's own
  renderer, and write `target/stage_headless.png` and
  `target/gobos_window_headless.png`. With `--nocapture` the first one also
  prints which lights got pictures for which slots.

## Known gaps

- Many QLC+ fixture files leave the gobo image blank (`Res1=""`), so a
  model can be in the library and still show `?` tiles. Assign pictures by
  hand, or extend `wheels.json` upstream.
- Coloured glass gobos are rendered by brightness only; their tint is not
  applied to the beam yet.
- Prisms and frost do not yet affect the projected image.
