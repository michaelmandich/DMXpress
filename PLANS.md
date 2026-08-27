# DMXpress — Feature Plans

Six proposed features, ordered roughly by effort/dependency. Each has scope, design, files touched, and open questions.

---

## 1. Truss Sections — F34 Straight & Radius (Curved) Truss

**Goal:** Add real truss primitives to the stage layout alongside the existing `Tower`: F34-style straight box truss sections and curved (radius) truss arcs, with fixture mounting slots.

### Design
- New enum in [src/stage/layout.rs](src/stage/layout.rs):
  ```rust
  enum Structure {
      Tower(Tower),                    // existing
      TrussStraight(TrussSection),     // F34-style box truss
      TrussRadius(TrussArc),           // curved segment
  }
  ```
- `TrussSection`: length (standard sizes 0.5/1/2/3 m), position, yaw/pitch/roll, mount slots spaced every 0.5 m (top + bottom chords).
- `TrussArc`: radius, arc angle (e.g. 90°/180°/360°), segment count, mount slots distributed along the arc. Slot transforms computed from arc parametrisation so fixtures aim correctly (tangent-aligned yaw).
- Rendering: wireframe box-truss look (4 chords + diagonal bracing) in [src/stage/draw.rs](src/stage/draw.rs); arcs tessellated into short straight segments.
- Gizmo/inspector support: reuse existing `LightTransform` gizmo ([src/stage/gizmo.rs](src/stage/gizmo.rs)); inspector gets fields for length/radius/arc angle.
- Persistence: extend `stage_layout.json` schema (serde with `#[serde(default)]` for backward compat with existing tower-only layouts).
- Truss-to-truss snapping (corner blocks) is a stretch goal — start with free placement.

### Files
`src/stage/layout.rs`, `draw.rs`, `geometry.rs`, `gizmo.rs`, `inspector.rs`, `input.rs`

### Open questions
- Should fixtures re-parent to truss (move with it) — yes, like towers today?
- Which standard section lengths do you actually own?

**Effort: Medium** — geometry + rendering + UI, but follows existing tower patterns.

---

## 2. Detachable Panels — Move Boxes Out of the Main Window

**Goal:** Pop panels (Palettes, Phasers, Stacks, Beat, etc.) out into separate OS windows, e.g. onto a second monitor.

### Design
- egui 0.29 supports **multi-viewport** natively: `ctx.show_viewport_immediate` / `show_viewport_deferred` with `ViewportBuilder`.
- Add a per-panel state: `enum PanelHome { Docked, Floating, Detached }` stored in `views.json`.
- Each detachable panel gets a "pop out" button in its header; when detached, its `ui(...)` fn renders inside a `show_viewport_deferred` closure instead of the side/central panel.
- Panel render fns already take `&mut App` + `Ui`, so mostly a routing change in [src/ui/mod.rs](src/ui/mod.rs).
- Caveats: deferred viewports need `App` state access to be thread-safe-ish (egui runs them on the same thread with immediate viewports — prefer **immediate** viewports to avoid `Send + Sync` requirements).
- Close button on the detached window re-docks the panel.

### Files
`src/ui/mod.rs`, each panel module (small header change), `src/view.rs` (persistence), `src/main.rs`

**Effort: Medium-Low** — egui does the heavy lifting; main work is decoupling panel fns from panel containers.

---

## 3. Ableton Link Integration (Tempo / Beat Sync)

**Goal:** Sync `master_bpm` and beat phase to other software (Ableton, DJ apps) on the LAN automatically — replacing/augmenting tap-to-beat.

### Design
- Crate: [`rusty_link`](https://crates.io/crates/rusty_link) — safe Rust bindings for the official Ableton Link C++ SDK. Actively maintained.
- New module `src/link.rs`:
  - Owns `AblLink` + `SessionState`.
  - Each frame (or on a small timer): `capture_app_session_state()`, read `tempo()` and `beat_at_time(clock, quantum)`.
  - Drive existing clocks: set `master_bpm = link_tempo`, and instead of integrating `live.beats` from dt, **derive it from Link's beat** when Link is enabled (`live.beats = link_beat % 4.0` style, or gently slew to it to preserve the no-discontinuity design).
- UI: toggle + peer count in the Beat window ([src/ui/beat.rs](src/ui/beat.rs)): "Link: ON (3 peers) ♩=128.0".
- Tap-tempo can push tempo back *into* the Link session (`set_tempo`), so tapping still works and propagates to peers.
- The existing `snap_beats` machinery maps well: Link's quantum=4 matches the bar-based `beats: 0..4` accumulator.

### Files
New `src/link.rs`; `src/app.rs` (clock drive), `src/ui/beat.rs` (UI), `Cargo.toml`

### Risks
- `rusty_link` builds the C++ SDK via cmake — needs cmake installed; fine on macOS with brew.
- Phase-follow vs phase-jump: recommend slewing beats toward Link phase over ~250 ms rather than hard snapping.

**Effort: Medium-Low** — the beat architecture (accumulator + snap) was built for exactly this.

---

## 4. Claude Integration — Talk to Your Lights

**Goal:** Natural-language console: "make the back towers deep blue and add a slow sine on tilt" → app executes it.

### Design
- New module `src/assistant.rs` + chat panel `src/ui/assistant.rs`.
- Use the **Anthropic Messages API** (plain HTTPS via `reqwest` + `serde_json`; no SDK needed). API key from env var `ANTHROPIC_API_KEY` or settings (stored locally, never committed).
- **Tool-use pattern**: define a small set of tools Claude can call, mapped to existing app operations:
  - `set_color(group|fixtures, rgbw)` → palette application
  - `apply_phaser(selection, wave, subdiv, spread)` → phaser engine
  - `set_bpm(bpm)`, `start_chase(...)`, `fire_stack(name)`, `blackout()`
  - `get_state()` → returns patch, groups, running looks (so Claude has context)
- System prompt includes fixture patch summary + group names, generated from live state.
- Threading: API calls on a background thread (`std::thread` + channel), results applied on the UI thread — the app already has a frame loop, so poll an `mpsc::Receiver` in `update()`.
- Guardrails: every tool call is logged in the chat panel; optional "confirm before execute" toggle.

### Files
New `src/assistant.rs`, `src/ui/assistant.rs`; `src/app.rs` (command dispatch), `Cargo.toml` (`reqwest`, `tokio` or blocking client)

### Open questions
- Chat panel vs. single command bar (like the existing command UI in [src/ui/command.rs](src/ui/command.rs))?
- Should Claude be able to *create* palettes/phasers/stacks, or only trigger existing ones (safer v1)?

**Effort: Medium** — API plumbing is easy; the tool schema ↔ app-command mapping is the real work.

---

## 5. Track Analysis — Drop a Track, Detect Beats, Timed Playback

**Goal:** Load an audio file, analyze BPM + beat grid (and ideally sections/energy), then play it back with the light engine locked to the detected grid.

### Design — three stages
1. **Decode**: `symphonia` crate (pure Rust, mp3/aac/flac/wav/ogg) → mono f32 PCM.
2. **Analyze** (offline, background thread with progress bar):
   - Onset detection: spectral flux over STFT (`rustfft`), pick peaks.
   - Tempo: autocorrelation / comb-filter over onset envelope → BPM + downbeat phase.
   - Optional v2: energy envelope per band → auto section markers (drops, breakdowns) that could trigger stacks.
   - Cache results as `<track>.beats.json` next to the audio file.
3. **Playback**: `cpal` (or `rodio`) output stream. Playback clock is authoritative: each frame set `live.beats` from `(playhead_seconds - downbeat_offset) * bpm / 60`, i.e. same drive point as Ableton Link (feature 3) — **build a common `ClockSource` abstraction: Internal | Tap | Link | Track**.
- UI: new "Track" panel — drop zone / file picker, waveform strip with beat ticks, play/pause/scrub, detected BPM readout, nudge ±.

### Files
New `src/audio/` module (`decode.rs`, `analyze.rs`, `player.rs`); `src/ui/track.rs`; `src/app.rs` (ClockSource)

### Risks
- Beat tracking accuracy on variable-tempo material — v1 targets constant-BPM electronic music, offer manual grid nudge.
- Keep analysis off the UI thread; files can be minutes long.

**Effort: High** — the biggest feature here. Recommend doing feature 3 first since both share the ClockSource refactor.

---

## 6. Fixture Database — Add All Lights to the DB

**Goal:** Move beyond the 11 hardcoded profiles in [src/profiles.rs](src/profiles.rs) to a proper fixture library.

### Design
- **Format**: adopt the **Open Fixture Library (OFL)** JSON schema — thousands of community-maintained fixture definitions, free to bundle/download (github.com/OpenLightingProject/open-fixture-library). Alternative: GDTF (industry standard but much heavier to parse).
- New module `src/fixturedb.rs`:
  - Loads fixture definitions from a `fixtures/` directory (JSON files) at startup, merged with built-ins + `patch_user.json` + ShowBuddy imports.
  - Maps OFL capabilities → existing `Channel` roles (Pan/Tilt/Dimmer/RGBW/Strobe/Zoom/etc.) and physical data (pan/tilt range, beam angle) → existing `Profile` fields. The current role-by-name matching gets replaced by explicit capability mapping — more reliable.
- Patch UI ([src/ui/patchcfg.rs](src/ui/patchcfg.rs)): searchable fixture browser (manufacturer → model → mode) instead of a fixed list.
- Keep the 11 built-ins as a fallback so existing show files load unchanged.
- v2: in-app "download fixture" from the OFL API.

### Files
New `src/fixturedb.rs`, `fixtures/` data dir; `src/profiles.rs` (Profile becomes data-driven), `src/ui/patchcfg.rs`

**Effort: Medium-High** — the OFL→Channel mapping needs care (mode selection, fine channels, capability ranges), but pays off across every other feature.

---

## Suggested Order

| # | Feature | Effort | Why this order |
|---|---------|--------|----------------|
| 1 | Detachable panels (2) | M-Low | Self-contained, instant QoL win |
| 2 | Truss sections (1) | Medium | Self-contained, no external deps |
| 3 | Ableton Link (3) | M-Low | Introduces the `ClockSource` abstraction |
| 4 | Track analysis (5) | High | Reuses ClockSource from Link work |
| 5 | Fixture DB (6) | M-High | Independent; do anytime |
| 6 | Claude assistant (4) | Medium | Best last — benefits from richer app commands built above |

Shared refactor to do early: **`ClockSource` enum** (Internal / Tap / Link / Track) so features 3 & 5 don't fight over `master_bpm` and `live.beats`.
