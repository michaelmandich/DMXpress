//! Application state and the top-level eframe update loop. The actual panel
//! rendering lives in the `ui` module; `update` just dispatches to it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;
use std::net::Ipv4Addr;

use eframe::egui;

use crate::fixturedb;
use crate::net::{self, DiscoveredNode, NetCmd, NetEvent, NetHandle};
use crate::audio::{self, AudioEngine, AudioTrigger};
use crate::order::{self, Order};
use crate::oscillator::{self, CustomWaveform, Look};
use crate::palette::{self, Feature, Palette, PaletteRef, PaletteSeq, SeqPattern};
use crate::phaser::{self, Phaser};
use crate::preset::{self, SavedCycle, SavedOsc, UserPreset};
use crate::profiles::{self, UserFixture};
use crate::scene::{self, Scene};
use crate::stack::{self, Stack};
use crate::view::{self, View};
use crate::showbuddy::{self, Patch, PresetBank, Role};
use crate::stage::{self, StageView, V3};
use crate::transition::{TransitionBinding, TransitionConfig, TransitionRun};
use crate::chase::{ChaseConfig, ChaseRun, ChaseSource};
use crate::engine::{Layer, Mixer};
use crate::group::{self, Group, GroupMode};
/// A timed per-channel ramp used to fade palettes and phasers in and out.
#[derive(Debug, Clone, Copy)]
pub struct Ramp {
    pub from: f32,
    pub to: f32,
    pub start: Instant,
    pub dur: f32,
    /// Snap at the midpoint instead of sweeping through every value in
    /// between (colour wheels, gobo slots, macro channels).
    pub stepped: bool,
    /// Remove the faded thing (oscillator / flat add) once the ramp lands.
    pub remove_after: bool,
}

impl Ramp {
    /// Current value and whether the ramp has finished.
    fn value(&self, now: Instant) -> (f32, bool) {
        let k = if self.dur <= 0.0 {
            1.0
        } else {
            (now.duration_since(self.start).as_secs_f32() / self.dur).clamp(0.0, 1.0)
        };
        let v = if self.stepped {
            if k < 0.5 {
                self.from
            } else {
                self.to
            }
        } else {
            self.from + (self.to - self.from) * k
        };
        (v, k >= 1.0)
    }
}

/// A palette-cycle fade in flight: the cycle's overlay weight ramps from 0
/// to 1 (or back) over `dur` seconds from `start`.
pub(crate) struct CycleFade {
    pub start: Instant,
    pub dur: f32,
    pub out: bool,
}

pub(crate) struct App {
    pub net: NetHandle,
    pub nodes: Vec<DiscoveredNode>,
    pub selected: Option<Ipv4Addr>,
    pub universe: u16,
    pub log: Vec<String>,
    pub manual_ip: String,
    pub patch: Patch,
    pub sel_fixture: Option<usize>,
    pub stage: StageView,
    pub banks: Vec<PresetBank>,
    pub open_bank: Option<usize>,
    pub active_preset: Option<(usize, usize)>,
    pub confirm_reset: bool,
    /// The settled look currently playing when no transition is mid-flight
    /// (static or animated — a static look just has no oscillators).
    pub live: Look,
    pub transition: TransitionConfig,
    pub transition_run: Option<TransitionRun>,
    /// Spherical chase: a non-destructive moving pulse of another preset.
    pub chase: ChaseConfig,
    pub chase_run: Option<ChaseRun>,
    /// Dimmer throb: moment the throb button was hit — dimmers surge to full
    /// and decay back over ~half a second.
    pub throb_at: Option<Instant>,
    /// Global transport hold. While set, the last transmitted frame remains
    /// untouched and no animation, fade, chase, cue, or transition advances.
    pub frozen: bool,
    frozen_at: Option<Instant>,
    /// The output compositor: each frame's layer stack (base look + overlays,
    /// and later Decks/Phasers) flattened into the frame sent to Art-Net.
    pub mixer: Mixer,
    /// Sandboxed Rhai engine every plugin call runs through (see `plugin.rs`).
    pub plugin_engine: rhai::Engine,
    /// Installed plugin scripts, in file order.
    pub plugins: Vec<crate::plugin::Plugin>,
    /// Zero point of the `t` handed to plugin `frame`/`button` calls.
    pub plugin_epoch: Instant,
    /// A plugin theme tint changed — re-apply the style next frame.
    pub plugin_theme_dirty: bool,
    /// Plugin row awaiting delete confirmation in the Plugins window.
    pub plugin_confirm_remove: Option<usize>,
    pub show_plugins: bool,
    /// Stored fixture selections (the renamed grandMA3 Group pool).
    pub groups: Vec<Group>,
    /// Name field for storing the current selection as a group.
    pub group_name: String,
    /// Groups recalled since the last fresh click, in click order. Shift-
    /// clicking group after group builds this chain, which the Orders window
    /// turns into a custom effect route.
    pub group_chain: Vec<usize>,
    /// Custom effect routes (see `order.rs`).
    pub orders: Vec<Order>,
    /// Which order effects currently travel along; `None` = patch order.
    pub active_order: Option<usize>,
    /// Order selected for editing in the Orders window.
    pub order_edit: Option<usize>,
    /// Name field for storing a new order.
    pub order_name: String,
    /// Referenced presets (the renamed grandMA3 Palette pool).
    pub palettes: Vec<Palette>,
    /// Next stable palette id (never reused, so references stay valid).
    pub next_palette_id: u32,
    pub palette_name: String,
    /// Feature tab currently shown in the palette pool.
    pub palette_tab: Feature,
    /// Palette cycle: ordered palette ids the beat-driven cycle steps through.
    pub cycle_ids: Vec<u32>,
    /// Relative width of each cycle segment (parallel to `cycle_ids`).
    pub cycle_weights: Vec<f32>,
    pub advanced_palette_mode: bool,
    pub cycle_on: bool,
    /// Beats per palette step.
    pub cycle_beats_per: f32,
    /// Phase fanned across the fixtures (0 = together, 1 = full cycle spread).
    pub cycle_spread: f32,
    /// How the spacing disperses across the rig.
    pub cycle_pattern: SeqPattern,
    /// Shape of each change: 0 = smooth crossfade, 1 = hard snap.
    pub cycle_shape: f32,
    /// Cycle beat clock, advanced by the live tempo each frame.
    pub cycle_beats: f32,
    pub cycle_last: Option<Instant>,
    /// Whether the current palette cycle follows master taps/BPM.
    pub cycle_master_beat: bool,
    /// Tempo retained by a cycle opted out of the master beat.
    pub cycle_tempo: f32,
    /// Pending gradual correction toward the tapped beat grid.
    pub cycle_beat_nudge: f32,
    /// Which saved sequence the running cycle was loaded from, so two
    /// sequences sharing the same palette set light up independently.
    pub cycle_seq: Option<usize>,
    /// Fade in or out of the cycle in progress, if any (see `start_cycle`).
    pub cycle_fade: Option<CycleFade>,
    /// Stacked effect lanes, one per island (Gobo, Prism, Light ring…):
    /// the palette ids picked on the deck's wheel pages. One id holds that
    /// slot on every light that has it; two or more step through them on
    /// the cycle clock. Each lane is its own mixer layer, so they stack on
    /// the colour cycle and on each other (see `wheels.rs`).
    pub effect_lanes: BTreeMap<String, Vec<u32>>,
    /// Sequence pool: drag tiles to folders (true) or click to select (false).
    pub seq_drag_mode: bool,
    /// Saved palette sequences (colours + motion), with their folders.
    pub seqs: Vec<PaletteSeq>,
    pub seq_folders: Vec<String>,
    pub seq_name: String,
    /// Programmer channel → palette it currently traces to (for cue references).
    pub live_refs: HashMap<usize, PaletteRef>,
    /// Channels the programmer is actively asserting (the record mask).
    pub live_active: HashSet<usize>,
    /// Reusable spread effects (the renamed grandMA3 Phaser pool).
    pub phasers: Vec<Phaser>,
    /// Running phasers: name → the 0-based addresses its oscillators own.
    pub active_phasers: HashMap<String, Vec<usize>>,
    /// DMX test window: forced channel values (0-based addr → value), applied
    /// on top of everything right before output.
    pub test_overrides: HashMap<usize, u8>,
    /// Encoder layer (0-based addr → value): channels dialled in with the
    /// Stream Deck's programmer knob, painted over the mix (under the grand
    /// master) until Clear drops them (see `encoder.rs`).
    pub encoder_layer: HashMap<usize, u8>,
    /// Channel-type key the programmer knob is on (see `encoder_channels`).
    pub encoder_key: Option<String>,
    /// The mixer's output this frame, before masters and overrides — what
    /// the encoder starts nudging from.
    pub mixed: net::Frame,
    /// Hold-phaser overrides (0-based addr → value): forced onto the output
    /// every frame until the phaser is stopped (e.g. smoke machine on).
    pub hold_overrides: HashMap<usize, u8>,
    /// Flat-add phaser offsets (0-based addr → signed add): summed onto the
    /// output every frame until the phaser is stopped.
    pub add_overrides: HashMap<usize, i16>,
    /// Timed per-channel fades easing palette recalls / poses into the
    /// programmer base (0-based addr → ramp).
    pub base_fades: HashMap<usize, Ramp>,
    /// Oscillator-depth ramps fading wave phasers in and out.
    pub osc_ramps: HashMap<usize, Ramp>,
    /// Level ramps fading flat-add phasers in and out.
    pub add_ramps: HashMap<usize, Ramp>,
    /// Palette recall fade time in seconds (the Palettes window fader); 0 = snap.
    pub palette_fade_s: f32,
    pub palette_transition: TransitionBinding,
    /// Phaser apply/stop fade time in seconds (the Phasers window fader); 0 = snap.
    pub phaser_fade_s: f32,
    pub phaser_transition: TransitionBinding,
    /// User-crafted oscillator waveforms shared by the oscillator and phaser builders.
    pub custom_waveforms: Vec<CustomWaveform>,
    pub waveform_edit: CustomWaveform,
    pub waveform_edit_sel: Option<usize>,
    pub waveform_drag: Option<usize>,
    /// Native DMXpress presets (saved programmer snapshots incl. oscillators).
    pub user_presets: Vec<UserPreset>,
    /// Preset folders (may be empty); presets reference them by name.
    pub preset_folders: Vec<String>,
    pub preset_name: String,
    pub active_user_preset: Option<usize>,
    /// Next `UserPreset.id` to hand out (see `preset::next_id`).
    pub next_preset_id: u32,
    /// The phaser currently being edited in the pool window.
    pub phaser_edit: Phaser,
    pub phaser_name: String,
    /// Edit mode: clicking a pool tile loads it into the editor for
    /// non-destructive tweaking instead of applying it.
    pub phaser_edit_mode: bool,
    /// Pool index of the phaser being edited in place (edit mode).
    pub phaser_edit_sel: Option<usize>,
    /// Live-apply: every edit in the phaser editor re-applies to the
    /// selection immediately, as if Apply were pressed on each change.
    pub phaser_live: bool,
    /// Master BPM: when on, this one clock drives the tempo of every look
    /// (programmer, transitions, chase) each frame.
    pub master_bpm: f32,
    pub master_bpm_on: bool,
    /// Recent tap-tempo timestamps (cleared after a pause).
    pub beat_taps: Vec<Instant>,
    /// Time machine: how fast animation time flows (magnitude only).
    pub time_rate: f32,
    /// Time machine: run every clock backwards.
    pub time_reverse: bool,
    /// Auto-flip the direction every `pendulum_beats`, so looks sweep out and
    /// retrace instead of looping.
    pub pendulum: bool,
    pub pendulum_beats: f32,
    /// Beat clock reading at the last direction flip.
    pub(crate) pendulum_anchor: f32,
    /// Throw the beat clock back on itself every `stutter_beats`, freezing the
    /// look into a short repeating loop.
    pub stutter: bool,
    pub stutter_beats: f32,
    pub(crate) stutter_anchor: Option<f32>,
    /// Cue lists (the renamed grandMA3 Sequence/cuelist pool).
    pub stacks: Vec<Stack>,
    /// Captured effect states that play as their own mixer layers, so several
    /// can run at once (see `scene.rs`). Pool order is priority.
    pub scenes: Vec<Scene>,
    /// Name field for capturing the programmer as a scene.
    pub scene_name: String,
    /// Play the pool in order, each scene handing to the next on its hold.
    pub scene_chain: bool,
    /// Live audio capture and analysis (spectrum, beat tracker).
    pub audio: AudioEngine,
    /// Elgato Stream Deck control surface (see `streamdeck.rs`).
    pub deck: crate::streamdeck::DeckEngine,
    /// Whether the Stream Deck mirrors the app. On by default: a control
    /// surface that needs switching on before it does anything is a control
    /// surface you forget to switch on.
    pub send_to_deck: bool,
    /// Tracks the previous frame's `send_to_deck`, so the deck is cleared
    /// exactly once when the checkbox is switched off.
    pub deck_active: bool,
    /// Non-destructive output override (Stream Deck Master Dimmer press):
    /// forces the wire to black without touching the programmer/stacks, so
    /// lifting it picks the show back up exactly where it was.
    pub blackout: bool,
    /// Which deck page is showing — flipped by the Mode knob.
    pub deck_page: crate::streamdeck::DeckPage,
    /// Which of the Palettes page's sub-pages (colours, gobos, prisms) the
    /// deck shows — flipped by the Page knob.
    pub palette_sub: crate::streamdeck::PaletteSub,
    /// The Phaser page's slots (phaser name, colour, symbol), 36 per page and
    /// as many pages as you add, edited from the Phasers window and
    /// persisted to `phaser_deck.json`.
    pub phaser_deck: Vec<Option<crate::streamdeck::PhaserSlot>>,
    /// Which page of `phaser_deck` the deck (and the editor) is showing —
    /// scrolled by the Page knob while in Phaser mode.
    pub deck_phaser_page: usize,
    /// Which page of lights the deck's Fixtures page shows.
    pub deck_fixture_page: usize,
    /// Slot index currently open in the Phaser page editor, if any.
    pub deck_slot_edit: Option<usize>,
    pub deck_slot_phaser: String,
    pub deck_slot_color: [u8; 3],
    pub deck_slot_label: String,
    pub deck_slot_icon: crate::streamdeck::KeyIcon,
    /// The Presets board's pads, 36 per page, persisted to `preset_deck.json`;
    /// the deck's Presets page shows these pages, then the ShowBuddy banks.
    pub preset_deck: Vec<Option<crate::preset_deck::PresetSlot>>,
    /// Which page of `preset_deck` the deck and the board window show; one
    /// past the last board page = the ShowBuddy page.
    pub deck_preset_page: usize,
    /// Band-threshold rules painting looks over the show (see `audio.rs`).
    pub audio_triggers: Vec<AudioTrigger>,
    /// Detected beats press the TAP button automatically.
    pub audio_follow_beat: bool,
    /// Last beat counter value folded into the tap machinery.
    pub audio_beat_seen: u64,
    /// When the last detected beat arrived (drives the UI flash).
    pub audio_beat_flash: Option<Instant>,
    /// Wall clock of the last audio-trigger evaluation (for envelopes).
    pub audio_last_eval: Option<Instant>,
    /// Trigger selected on the graph for band/threshold dragging.
    pub audio_sel: Option<usize>,
    /// Capture sources found at the last refresh.
    pub audio_devices: Vec<audio::AudioSource>,
    /// Device name remembered from `audio.json` for preselection.
    pub audio_device_pref: Option<(String, bool)>,
    /// Stack shown/edited in the Stacks window.
    pub cur_stack: Option<usize>,
    /// Default fade applied to newly recorded cues.
    pub cue_fade: f32,
    /// Grand master 0..1 — scales every dimmer channel in the final output.
    pub grand_master: f32,
    /// Record filter: which features Store captures into a cue (all on = full).
    pub record_mask: HashSet<Feature>,
    /// Saved workspace layouts (the renamed grandMA3 Views).
    pub views: Vec<View>,
    pub view_name: String,
    /// Saved stage-camera bookmarks (cameras.json), Inspector → Stage.
    pub cameras: Vec<stage::CameraBookmark>,
    /// Current text in the command line.
    pub command: String,
    pub settings: stage::Settings,
    /// The Network screen (Settings → Network…): protocol setup, node and
    /// sACN source tables, checklist, test patterns and packet monitor.
    pub network: crate::ui::network::NetworkState,
    pub show_settings: bool,
    pub show_artnet: bool,
    pub show_transition: bool,
    pub show_chases: bool,
    pub show_groups: bool,
    pub show_orders: bool,
    pub show_scenes: bool,
    pub show_audio: bool,
    pub show_beat: bool,
    pub show_palettes: bool,
    pub show_phasers: bool,
    pub show_stacks: bool,
    pub show_decks: bool,
    pub show_command: bool,
    pub show_views: bool,
    /// Floating Log window visible.
    pub show_log: bool,
    /// Floating Oscillator window visible.
    pub show_osc: bool,
    /// Keys (see [`crate::ui::floating_panel`]) of panels currently popped
    /// out into their own native OS window instead of docked as a floating
    /// `egui::Window` inside the main window.
    pub popped_out: HashSet<&'static str>,
    /// Docked panels folded away: the side panels to a thin rail
    /// ("fixtures", "inspector"), the channel controls to one row
    /// ("channels"). All three folded leaves just the stage.
    pub collapsed: HashSet<&'static str>,
    /// The Phaser board window (the deck's Phaser page, on screen).
    pub show_phaser_board: bool,
    /// Board in arrange mode: pads are assigned and swapped, not pressed.
    pub board_arrange: bool,
    /// The Presets board window (the deck's Presets page, on screen).
    pub show_preset_board: bool,
    /// The Phasers window's library search and filter.
    pub phaser_search: String,
    pub phaser_lib_filter: crate::phaser::LibFilter,
    /// Ctrl+Shift+Z history (see `undo.rs`).
    pub undo: crate::undo::History,
    /// Rolling backups of the show (see `backup.rs`).
    pub autosave: crate::backup::Autosave,
    /// A show file waiting for the "Import show?" answer.
    pub pending_import: Option<std::path::PathBuf>,
    /// A backup waiting for the "Restore backup?" answer.
    pub confirm_restore: Option<std::path::PathBuf>,
    /// The Configurations window's import path field.
    pub import_path: String,
    /// Name field for saving the current stage arrangement as a setup.
    pub setup_name: String,
    /// Fixtures patched in DMXpress on top of the ShowBuddy patch.
    pub user_fixtures: Vec<UserFixture>,
    /// Merge the ShowBuddy patch in at all (off = DMXpress fixtures only).
    pub include_showbuddy: bool,
    /// Last known ShowBuddy fixtures, from a live import, the local cache, or
    /// a loaded configuration. Used whenever ShowBuddy cannot be reached so a
    /// show keeps its rig on machines without it.
    pub showbuddy_patch: Vec<showbuddy::Fixture>,
    /// Individual ShowBuddy fixtures hidden from the rig (`display@from` keys).
    pub excluded_fixtures: Vec<String>,
    /// Profiles patched most recently, newest first (see `profiles::UserPatch::recent`).
    pub recent_profiles: Vec<profiles::RecentProfile>,
    /// Patch window visible.
    pub show_patch: bool,
    /// Profile index selected in the Patch window.
    pub patch_profile: usize,
    /// Name field in the Patch window.
    pub patch_name: String,
    /// Start address field in the Patch window (1-based).
    pub patch_addr: u16,
    /// How many copies to add at once in the Patch window.
    pub patch_count: u16,
    /// The bundled fixture library, loaded on demand (see `fixturedb.rs`).
    pub library: fixturedb::Library,
    /// The gobo catalogue: masks, names, and which model carries what in
    /// which slot (see `gobo.rs`).
    pub gobos: crate::gobo::Catalogue,
    /// Gobo slots pinned by hand in the Gobos window.
    pub user_gobos: crate::gobo::UserGobos,
    /// Gobos window visible.
    pub show_gobos: bool,
    /// Assign mode in the Gobos window: clicking a slot picks its picture
    /// instead of sending the lights there.
    pub gobo_assign_mode: bool,
    /// Search box and maker filter in the Gobos window's picture picker.
    pub gobo_search: String,
    pub gobo_maker: Option<String>,
    /// The slot the picker is choosing a picture for.
    pub gobo_pick: Option<crate::gobo::GoboPick>,
    /// Thumbnails uploaded so far, by gobo key (`None` = no mask to show).
    pub(crate) gobo_thumbs: HashMap<String, Option<egui::TextureHandle>>,
    /// Search box in the Patch window's fixture browser.
    pub patch_search: String,
    /// Manufacturer the browser is narrowed to, if any.
    pub patch_manufacturer: Option<String>,
    /// Library id picked in the browser, ready to patch.
    pub patch_library_sel: Option<String>,
    /// Configurations window visible.
    pub show_configs: bool,
    pub show_dmx_test: bool,
    /// Name field for saving the current configuration.
    pub config_name: String,
    /// Configuration pending delete confirmation.
    pub confirm_delete_config: Option<String>,
    /// New-show confirmation dialog visible.
    pub confirm_new_show: bool,
    /// New-show option: drop the ShowBuddy lights from the patch.
    pub new_show_drop_showbuddy: bool,
    /// New-show option: reset light positions to defaults.
    pub new_show_reset_layout: bool,
    /// Selected channels (0-based DMX buffer indices) in the channel editor.
    pub sel_channels: HashSet<usize>,
    /// Channel control UI state: filter, sort, folded groups, range anchor.
    /// Not saved, not part of undo.
    pub chan_ui: crate::ui::chanrows::ChanUi,
    /// Inspector shell: active tab, per-tab UI state and the cosmetic
    /// preferences in inspector.json. Not saved with the show, not undone.
    pub insp: crate::ui::inspector::InspectorUi,
    /// Independent UI zoom for each major panel (1.0 = default).
    pub zoom: PanelZoom,
    /// The toolbar logo, uploaded on first paint. `Some(None)` means the
    /// embedded PNG failed to decode, so the text wordmark is used instead.
    pub(crate) logo: Option<Option<egui::TextureHandle>>,
}

/// Per-panel text/element scale factors.
pub(crate) struct PanelZoom {
    pub fixtures: f32,
    pub inspector: f32,
    pub central: f32,
    pub log: f32,
    pub osc: f32,
    pub transition: f32,
    pub groups: f32,
    pub orders: f32,
    pub scenes: f32,
    pub audio: f32,
    pub palettes: f32,
    pub gobos: f32,
    pub phasers: f32,
    pub preset_board: f32,
    pub stacks: f32,
    pub views: f32,
    pub plugins: f32,
}

impl Default for PanelZoom {
    fn default() -> Self {
        Self {
            fixtures: 1.0,
            inspector: 1.0,
            central: 1.0,
            log: 1.0,
            osc: 1.0,
            transition: 1.0,
            groups: 1.0,
            orders: 1.0,
            scenes: 1.0,
            audio: 1.0,
            palettes: 1.0,
            gobos: 1.0,
            phasers: 1.0,
            preset_board: 1.0,
            stacks: 1.0,
            views: 1.0,
            plugins: 1.0,
        }
    }
}

/// Read the ShowBuddy patch, falling back to `cache` when ShowBuddy itself is
/// unreachable — it lives at a fixed absolute macOS path that does not exist on
/// a cloned checkout or a machine without it installed. Returns the patch plus
/// the fixture list to remember as the new cache.
fn load_showbuddy(
    cache: &[showbuddy::Fixture],
    log: &mut Vec<String>,
) -> (Patch, Vec<showbuddy::Fixture>) {
    match showbuddy::load_default() {
        Ok(p) => {
            log.push(format!(
                "Loaded ShowBuddy patch: {} fixtures",
                p.fixtures.len()
            ));
            showbuddy::save_cache(&p.fixtures);
            let fixtures = p.fixtures.clone();
            (p, fixtures)
        }
        Err(e) if !cache.is_empty() => {
            log.push(format!(
                "ShowBuddy unavailable ({e:#}) — restored {} fixture(s) saved with this show",
                cache.len()
            ));
            let patch = Patch {
                fixtures: cache.to_vec(),
                warnings: Vec::new(),
            };
            (patch, cache.to_vec())
        }
        Err(e) => {
            log.push(format!("ShowBuddy patch load failed: {e:#}"));
            (Patch::default(), Vec::new())
        }
    }
}

/// Read ShowBuddy's preset banks, falling back to the local cache when
/// ShowBuddy is unreachable. A successful read is parsed in full and cached so
/// the presets keep working elsewhere.
fn load_banks(log: &mut Vec<String>) -> Vec<PresetBank> {
    match showbuddy::load_preset_banks() {
        Ok(mut banks) => {
            showbuddy::hydrate_presets(&mut banks);
            showbuddy::save_preset_cache(&banks);
            log.push(format!(
                "Loaded {} preset banks ({} presets)",
                banks.len(),
                banks.iter().map(|x| x.presets.len()).sum::<usize>()
            ));
            banks
        }
        Err(e) => {
            let cached = showbuddy::load_preset_cache();
            if cached.is_empty() {
                log.push(format!("preset banks load failed: {e:#}"));
            } else {
                log.push(format!(
                    "ShowBuddy presets unavailable ({e:#}) — using {} cached bank(s) ({} presets)",
                    cached.len(),
                    cached.iter().map(|x| x.presets.len()).sum::<usize>()
                ));
            }
            cached
        }
    }
}

impl App {
    pub fn new() -> Self {
        let net = net::spawn().expect("failed to start net thread");
        let mut log = Vec::new();
        let user_patch = profiles::load_user_patch();
        let mut showbuddy_patch = showbuddy::load_cache();
        let mut patch = if user_patch.include_showbuddy {
            let (p, fx) = load_showbuddy(&showbuddy_patch, &mut log);
            showbuddy_patch = fx;
            p
        } else {
            log.push("ShowBuddy patch disabled — DMXpress fixtures only".into());
            Patch::default()
        };
        // The library unpacks to over ten megabytes, so it only loads when
        // something actually needs it: a patched `lib:` fixture now, or the
        // patch browser later.
        let library = if user_patch.fixtures.iter().any(|f| fixturedb::library_id(&f.profile).is_some())
        {
            fixturedb::Library::load()
        } else {
            fixturedb::Library::default()
        };
        profiles::extend_patch(&mut patch, &user_patch, &library);
        let gobos = crate::gobo::Catalogue::load();
        if let Some(e) = &gobos.error {
            log.push(format!("Gobo catalogue unavailable ({e}) — beams render without gobos"));
        }
        let user_gobos = crate::gobo::UserGobos::load();
        crate::gobo::assign_patch(&mut patch, &library, &gobos, &user_gobos);
        if !user_patch.fixtures.is_empty() {
            log.push(format!("Patched {} DMXpress fixtures", user_patch.fixtures.len()));
        }
        for w in &patch.warnings {
            log.push(format!("patch warning: {w}"));
        }
        let sel_fixture = if patch.fixtures.is_empty() { None } else { Some(0) };
        let settings = stage::Settings::load();
        let insp = crate::ui::inspector::InspectorUi::load();
        let mut stage = StageView::new();
        stage.sync(&patch, &settings);
        stage.show = insp.prefs.display;
        stage.fly_mode = insp.prefs.fly_mode;
        if insp.prefs.stage.restore_camera {
            if let Some(c) = &insp.prefs.stage.last_camera {
                stage.cam.apply_snapshot(c);
            }
        }
        let banks = if user_patch.include_showbuddy {
            load_banks(&mut log)
        } else {
            Vec::new()
        };
        let palettes = palette::load_palettes();
        let next_palette_id = palettes.iter().map(|p| p.id).max().map_or(0, |m| m + 1);
        let seq_store = palette::load_seqs();
        let mut preset_store = preset::load_presets();
        let next_preset_id = preset::assign_ids(&mut preset_store.presets);
        let preset_deck = crate::preset_deck::load_preset_deck();
        // A brand-new, empty board on a machine with ShowBuddy banks starts
        // the deck on the banks page, so nothing familiar disappears.
        let deck_preset_page = if preset_deck.iter().all(Option::is_none) && !banks.is_empty() {
            crate::preset_deck::preset_deck_pages(&preset_deck)
        } else {
            0
        };
        let audio_file = audio::load_audio();
        let plugin_engine = crate::plugin::engine();
        let plugins = crate::plugin::load_all(&plugin_engine);
        if !plugins.is_empty() {
            log.push(format!("Plugins: {} loaded", plugins.len()));
        }
        Self {
            network: crate::ui::network::NetworkState::new(net.config.clone()),
            net,
            nodes: Vec::new(),
            selected: None,
            universe: 0,
            log,
            manual_ip: String::new(),
            patch,
            sel_fixture,
            stage,
            banks,
            open_bank: None,
            active_preset: None,
            confirm_reset: false,
            live: Look::black(),
            transition: TransitionConfig::default(),
            transition_run: None,
            chase: ChaseConfig::default(),
            chase_run: None,
            throb_at: None,
            frozen: false,
            frozen_at: None,
            mixer: Mixer::new(),
            plugin_engine,
            plugins,
            plugin_epoch: Instant::now(),
            plugin_theme_dirty: true,
            plugin_confirm_remove: None,
            show_plugins: false,
            groups: group::load_groups(),
            group_name: String::new(),
            group_chain: Vec::new(),
            orders: order::load_orders(),
            active_order: None,
            order_edit: None,
            order_name: String::new(),
            palettes,
            next_palette_id,
            palette_name: String::new(),
            palette_tab: Feature::Color,
            cycle_ids: Vec::new(),
            cycle_weights: Vec::new(),
            advanced_palette_mode: false,
            cycle_seq: None,
            cycle_fade: None,
            effect_lanes: BTreeMap::new(),
            seq_drag_mode: false,
            cycle_on: false,
            cycle_beats_per: 4.0,
            cycle_spread: 0.0,
            cycle_pattern: SeqPattern::Wave,
            cycle_shape: 0.0,
            cycle_beats: 0.0,
            cycle_last: None,
            cycle_master_beat: true,
            cycle_tempo: 120.0,
            cycle_beat_nudge: 0.0,
            seqs: seq_store.seqs,
            seq_folders: seq_store.folders,
            seq_name: String::new(),
            live_refs: HashMap::new(),
            live_active: HashSet::new(),
            phasers: phaser::load_phasers(),
            active_phasers: HashMap::new(),
            test_overrides: HashMap::new(),
            encoder_layer: HashMap::new(),
            encoder_key: None,
            mixed: net::Frame::black(),
            hold_overrides: HashMap::new(),
            add_overrides: HashMap::new(),
            base_fades: HashMap::new(),
            osc_ramps: HashMap::new(),
            add_ramps: HashMap::new(),
            palette_fade_s: 0.0,
            palette_transition: TransitionBinding::Custom,
            phaser_fade_s: 0.0,
            phaser_transition: TransitionBinding::Custom,
            custom_waveforms: oscillator::load_waveforms(),
            waveform_edit: CustomWaveform::default(),
            waveform_edit_sel: None,
            waveform_drag: None,
            user_presets: preset_store.presets,
            preset_folders: preset_store.folders,
            preset_name: String::new(),
            active_user_preset: None,
            next_preset_id,
            phaser_edit: Phaser::default(),
            phaser_name: String::new(),
            phaser_edit_mode: false,
            phaser_edit_sel: None,
            phaser_live: false,
            master_bpm: 120.0,
            master_bpm_on: false,
            beat_taps: Vec::new(),
            time_rate: 1.0,
            time_reverse: false,
            pendulum: false,
            pendulum_beats: 4.0,
            pendulum_anchor: 0.0,
            stutter: false,
            stutter_beats: 0.5,
            stutter_anchor: None,
            stacks: stack::load_stacks(),
            scenes: scene::load_scenes(),
            scene_name: String::new(),
            scene_chain: false,
            audio: AudioEngine::new(),
            deck: crate::streamdeck::DeckEngine::new(),
            send_to_deck: true,
            deck_active: false,
            blackout: false,
            deck_page: crate::streamdeck::DeckPage::default(),
            palette_sub: crate::streamdeck::PaletteSub::default(),
            phaser_deck: crate::streamdeck::load_phaser_deck(),
            deck_phaser_page: 0,
            deck_fixture_page: 0,
            deck_slot_edit: None,
            deck_slot_phaser: String::new(),
            deck_slot_color: [110, 120, 150],
            deck_slot_label: String::new(),
            deck_slot_icon: crate::streamdeck::KeyIcon::None,
            preset_deck,
            deck_preset_page,
            audio_triggers: audio_file.triggers,
            audio_follow_beat: audio_file.follow_beat,
            audio_beat_seen: 0,
            audio_beat_flash: None,
            audio_last_eval: None,
            audio_sel: None,
            audio_devices: Vec::new(),
            audio_device_pref: audio_file.device.map(|d| (d, audio_file.loopback)),
            cur_stack: None,
            cue_fade: 3.0,
            grand_master: 1.0,
            record_mask: Feature::ALL.iter().copied().collect(),
            views: view::load_views(),
            view_name: String::new(),
            cameras: stage::load_cameras(),
            command: String::new(),
            settings,
            show_settings: false,
            show_artnet: false,
            show_transition: false,
            show_chases: false,
            show_groups: false,
            show_orders: false,
            show_scenes: false,
            show_audio: false,
            show_beat: false,
            show_palettes: false,
            show_phasers: false,
            show_stacks: false,
            show_decks: false,
            show_command: false,
            show_views: false,
            show_log: true,
            show_osc: true,
            popped_out: HashSet::new(),
            collapsed: HashSet::new(),
            show_phaser_board: false,
            board_arrange: false,
            show_preset_board: false,
            phaser_search: String::new(),
            phaser_lib_filter: crate::phaser::LibFilter::All,
            undo: crate::undo::History::new(),
            autosave: crate::backup::Autosave::new(),
            pending_import: None,
            confirm_restore: None,
            import_path: String::new(),
            setup_name: String::new(),
            user_fixtures: user_patch.fixtures,
            include_showbuddy: user_patch.include_showbuddy,
            showbuddy_patch,
            excluded_fixtures: user_patch.excluded,
            recent_profiles: user_patch.recent,
            show_patch: false,
            patch_profile: 0,
            patch_name: String::new(),
            patch_addr: 1,
            patch_count: 1,
            library,
            gobos,
            user_gobos,
            show_gobos: false,
            gobo_assign_mode: false,
            gobo_search: String::new(),
            gobo_maker: None,
            gobo_pick: None,
            gobo_thumbs: HashMap::new(),
            patch_search: String::new(),
            patch_manufacturer: None,
            patch_library_sel: None,
            show_configs: false,
            show_dmx_test: false,
            config_name: String::new(),
            confirm_delete_config: None,
            confirm_new_show: false,
            new_show_drop_showbuddy: true,
            new_show_reset_layout: true,
            sel_channels: HashSet::new(),
            chan_ui: crate::ui::chanrows::ChanUi::default(),
            insp,
            zoom: PanelZoom::default(),
            logo: None,
        }
    }

    /// Reload the ShowBuddy patch, append the user-patched fixtures, and
    /// re-sync the stage. Used at startup, after patch edits, and when a
    /// configuration is loaded.
    pub fn rebuild_patch(&mut self) {
        let cache = std::mem::take(&mut self.showbuddy_patch);
        let mut patch = if self.include_showbuddy {
            let (p, fixtures) = load_showbuddy(&cache, &mut self.log);
            self.showbuddy_patch = fixtures;
            p
        } else {
            // Keep the snapshot so re-enabling ShowBuddy restores the rig.
            self.showbuddy_patch = cache;
            Patch::default()
        };
        profiles::extend_patch(&mut patch, &self.current_user_patch(), &self.library);
        crate::gobo::assign_patch(&mut patch, &self.library, &self.gobos, &self.user_gobos);
        for w in &patch.warnings {
            self.log.push(format!("patch warning: {w}"));
        }
        self.log.push(format!(
            "Patch rebuilt: {} fixtures ({} from DMXpress)",
            patch.fixtures.len(),
            self.user_fixtures.len()
        ));
        self.sel_fixture = if patch.fixtures.is_empty() { None } else { Some(0) };
        self.patch = patch;
        self.stage.sync(&self.patch, &self.settings);
        self.sel_channels.clear();
        self.chan_ui.anchor = None;
        self.live_refs.clear();
        self.live_active.clear();
        self.active_phasers.clear();
        self.hold_overrides.clear();
        self.encoder_layer.clear();
        self.effect_lanes.clear();
        // ShowBuddy's preset banks follow its patch: no ShowBuddy, no banks.
        if self.include_showbuddy {
            self.banks = load_banks(&mut self.log);
        } else {
            self.banks.clear();
        }
        self.open_bank = None;
        self.active_preset = None;
        self.active_user_preset = None;
    }

    /// The in-memory DMXpress patch state as one [`profiles::UserPatch`].
    pub fn current_user_patch(&self) -> profiles::UserPatch {
        profiles::UserPatch {
            include_showbuddy: self.include_showbuddy,
            fixtures: self.user_fixtures.clone(),
            excluded: self.excluded_fixtures.clone(),
            recent: self.recent_profiles.clone(),
        }
    }

    /// Persist the DMXpress patch (fixtures + ShowBuddy toggle + exclusions).
    pub fn save_user_patch(&self) {
        profiles::save_user_patch(&self.current_user_patch());
    }

    /// Wipe the show and start fresh: clears groups, palettes, phasers,
    /// stacks, views and the programmer (the patch and stage settings stay).
    /// Optionally drops the ShowBuddy lights and resets light positions.
    pub fn new_show(&mut self, drop_showbuddy: bool, reset_layout: bool) {
        self.groups.clear();
        group::save_groups(&self.groups);
        self.palettes.clear();
        self.next_palette_id = 0;
        palette::save_palettes(&self.palettes);
        self.phasers = phaser::default_phasers();
        phaser::save_phasers(&self.phasers);
        self.stacks.clear();
        stack::save_stacks(&self.stacks);
        self.cur_stack = None;
        self.user_presets.clear();
        self.preset_folders.clear();
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.active_user_preset = None;
        self.forget_preset_caches();
        self.next_preset_id = 1;
        self.preset_deck = vec![None; crate::preset_deck::PRESET_DECK_SLOTS];
        self.deck_preset_page = 0;
        crate::preset_deck::save_preset_deck(&self.preset_deck);
        self.views.clear();
        view::save_views(&self.views);
        self.cameras.clear();
        stage::save_cameras(&self.cameras);
        self.live = Look::black();
        self.live_refs.clear();
        self.live_active.clear();
        self.active_phasers.clear();
        self.hold_overrides.clear();
        self.encoder_layer.clear();
        self.effect_lanes.clear();
        self.transition_run = None;
        self.chase_run = None;
        self.active_preset = None;
        self.active_user_preset = None;
        if drop_showbuddy {
            self.include_showbuddy = false;
            self.save_user_patch();
        }
        self.rebuild_patch();
        if reset_layout {
            self.stage.reset_layout(&self.patch, &self.settings);
        }
        self.log.push(format!(
            "Fresh show: pools cleared{}{}",
            if drop_showbuddy { ", ShowBuddy lights removed" } else { "" },
            if reset_layout { ", light positions reset" } else { "" }
        ));
    }

    /// Bundle every persisted piece of state into a [`Configuration`].
    pub fn snapshot_configuration(&self) -> crate::config::Configuration {
        crate::config::Configuration {
            settings: self.settings.clone(),
            layout: self.stage.export_layout(&self.patch),
            user_fixtures: self.user_fixtures.clone(),
            include_showbuddy: self.include_showbuddy,
            showbuddy_patch: (!self.showbuddy_patch.is_empty())
                .then(|| self.showbuddy_patch.clone()),
            excluded_fixtures: self.excluded_fixtures.clone(),
            groups: self.groups.clone(),
            orders: self.orders.clone(),
            palettes: self.palettes.clone(),
            phasers: self.phasers.clone(),
            user_presets: self.user_presets.clone(),
            preset_folders: self.preset_folders.clone(),
            stacks: self.stacks.clone(),
            scenes: self.scenes.clone(),
            views: self.views.clone(),
            cameras: self.cameras.clone(),
            universe: self.universe,
            grand_master: self.grand_master,
            cue_fade: self.cue_fade,
        }
    }

    /// Replace the whole show state with a loaded configuration and persist
    /// each piece so the next launch starts from it too.
    pub fn apply_configuration(&mut self, cfg: crate::config::Configuration) {
        self.settings = cfg.settings;
        let _ = self.settings.save();
        self.user_fixtures = cfg.user_fixtures;
        self.include_showbuddy = cfg.include_showbuddy;
        // Configurations written before shows carried their own patch have no
        // snapshot; keep whatever is already known rather than wiping it.
        if let Some(fixtures) = cfg.showbuddy_patch {
            showbuddy::save_cache(&fixtures);
            self.showbuddy_patch = fixtures;
        }
        self.excluded_fixtures = cfg.excluded_fixtures;
        self.save_user_patch();
        self.rebuild_patch();
        let patched: std::collections::HashSet<String> = self
            .patch
            .fixtures
            .iter()
            .map(|f| profiles::fixture_key(&f.display, f.from))
            .collect();
        let orphans = cfg
            .layout
            .instances
            .iter()
            .filter(|i| !patched.contains(&i.key))
            .count();
        if orphans > 0 {
            self.log.push(format!(
                "{orphans} saved light position(s) have no matching fixture in the patch and were dropped"
            ));
        }
        self.stage
            .import_layout(&self.patch, &self.settings, cfg.layout);
        self.groups = cfg.groups;
        self.orders = cfg.orders;
        order::save_orders(&self.orders);
        self.active_order = None;
        self.order_edit = None;
        self.group_chain.clear();
        group::save_groups(&self.groups);
        self.palettes = cfg.palettes;
        self.next_palette_id = self.palettes.iter().map(|p| p.id).max().map_or(0, |m| m + 1);
        palette::save_palettes(&self.palettes);
        self.phasers = cfg.phasers;
        phaser::save_phasers(&self.phasers);
        self.user_presets = cfg.user_presets;
        self.preset_folders = cfg.preset_folders;
        self.next_preset_id = preset::assign_ids(&mut self.user_presets);
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.active_user_preset = None;
        self.forget_preset_caches();
        self.stacks = cfg.stacks;
        stack::save_stacks(&self.stacks);
        self.cur_stack = None;
        self.scenes = cfg.scenes;
        scene::save_scenes(&self.scenes);
        self.scene_chain = false;
        self.views = cfg.views;
        view::save_views(&self.views);
        self.cameras = cfg.cameras;
        stage::save_cameras(&self.cameras);
        self.universe = cfg.universe;
        let _ = self.net.cmd_tx.send(NetCmd::SetUniverse(self.universe));
        self.grand_master = cfg.grand_master;
        self.cue_fade = cfg.cue_fade;
        // Everything referencing old fixture indices is invalid now.
        self.live = Look::black();
        self.active_phasers.clear();
        self.hold_overrides.clear();
        self.encoder_layer.clear();
        self.effect_lanes.clear();
        self.transition_run = None;
        self.chase_run = None;
        self.active_preset = None;
    }

    /// Fixtures that have a resolved stage position, paired with that position.
    pub(crate) fn fixture_positions(&self) -> Vec<(usize, V3)> {
        let mut pos = self.stage.fixture_positions(&self.patch);
        // A super-fixture occupies a single place in space, so chases and
        // spatial transitions sweep past it as one light instead of rippling
        // through its members.
        for bundle in self.super_fixtures() {
            let mut sum = V3::default();
            let mut n = 0.0;
            for &fi in bundle {
                if let Some(Some(p)) = pos.get(fi) {
                    sum = sum + *p;
                    n += 1.0;
                }
            }
            if n > 0.0 {
                let center = sum * (1.0 / n);
                for &fi in bundle {
                    if let Some(slot) = pos.get_mut(fi) {
                        if slot.is_some() {
                            *slot = Some(center);
                        }
                    }
                }
            }
        }
        pos.into_iter()
            .enumerate()
            .filter_map(|(fi, p)| p.map(|p| (fi, p)))
            .collect()
    }

    /// The rig, as the array of maps plugin `frame` calls receive: one entry
    /// per fixture with its 1-based base address, the first channel of each
    /// core role (0 when absent), and its stage position in metres.
    pub(crate) fn plugin_rig(&self) -> rhai::Array {
        use rhai::Dynamic;
        let positions = self.stage.fixture_positions(&self.patch);
        let mut rig = rhai::Array::with_capacity(self.patch.fixtures.len());
        for (i, fx) in self.patch.fixtures.iter().enumerate() {
            let base = fx.from.saturating_sub(1) as usize;
            let (mut dimmer, mut red, mut green, mut blue, mut white) = (0i64, 0i64, 0i64, 0i64, 0i64);
            for (ci, ch) in fx.channels.iter().enumerate() {
                let a = (base + ci + 1) as i64;
                match ch.role() {
                    Role::Dimmer if dimmer == 0 => dimmer = a,
                    Role::Red if red == 0 => red = a,
                    Role::Green if green == 0 => green = a,
                    Role::Blue if blue == 0 => blue = a,
                    Role::White if white == 0 => white = a,
                    _ => {}
                }
            }
            let p = positions.get(i).copied().flatten().unwrap_or_default();
            let mut m = rhai::Map::new();
            m.insert("i".into(), Dynamic::from_int(i as i64));
            m.insert("name".into(), Dynamic::from(fx.display.clone()));
            m.insert("addr".into(), Dynamic::from_int(fx.from as i64));
            m.insert("channels".into(), Dynamic::from_int(fx.channels.len() as i64));
            m.insert("dimmer".into(), Dynamic::from_int(dimmer));
            m.insert("red".into(), Dynamic::from_int(red));
            m.insert("green".into(), Dynamic::from_int(green));
            m.insert("blue".into(), Dynamic::from_int(blue));
            m.insert("white".into(), Dynamic::from_int(white));
            m.insert("x".into(), Dynamic::from_float(p.x as f64));
            m.insert("y".into(), Dynamic::from_float(p.y as f64));
            m.insert("z".into(), Dynamic::from_float(p.z as f64));
            rig.push(Dynamic::from_map(m));
        }
        rig
    }

    /// Lights the active order welds into multi-light steps. Touching any one
    /// of them drags in the rest, so they cannot be addressed individually
    /// while that order is active — whatever mode their group advertises.
    pub(crate) fn order_bound_fixtures(&self) -> HashSet<usize> {
        let mut out = HashSet::new();
        if let Some(o) = self.active_order.and_then(|i| self.orders.get(i)) {
            for step in o.steps.iter().filter(|s| s.is_unit()) {
                out.extend(step.fixtures.iter().copied());
            }
        }
        out
    }

    /// Fixture bundles that behave as one light no matter what is selected:
    /// the multi-fixture steps of the active order, then every group in "One
    /// fixture" mode. Order steps come first because an explicit order is the
    /// user's final word on what travels together — which is also how a
    /// fixture belonging to several groups stops being ambiguous.
    fn super_fixtures(&self) -> Vec<&[usize]> {
        let mut out: Vec<&[usize]> = Vec::new();
        let mut claimed: HashSet<usize> = HashSet::new();
        if let Some(o) = self.active_order.and_then(|i| self.orders.get(i)) {
            for step in &o.steps {
                if step.is_unit() && step.fixtures.iter().all(|fi| !claimed.contains(fi)) {
                    claimed.extend(step.fixtures.iter().copied());
                    out.push(&step.fixtures);
                }
            }
        }
        for g in &self.groups {
            if g.mode == GroupMode::AsFixture
                && g.fixtures.len() > 1
                && g.fixtures.iter().all(|fi| !claimed.contains(fi))
            {
                claimed.extend(g.fixtures.iter().copied());
                out.push(&g.fixtures);
            }
        }
        out
    }

    /// What an audio trigger paints: the source's stored values, cut down to
    /// the group's addresses when a group is set.
    pub(crate) fn trigger_values(&self, t: &AudioTrigger) -> Vec<(usize, u8)> {
        let Some(source) = t.source else {
            return Vec::new();
        };
        let values: Vec<(usize, u8)> = match source {
            audio::TriggerSource::Palette(id) => self
                .palettes
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.values.clone())
                .unwrap_or_default(),
            audio::TriggerSource::Preset(i) => self
                .user_presets
                .get(i)
                .map(|p| p.values.clone())
                .unwrap_or_default(),
        };
        let Some(g) = t.group.and_then(|gi| self.groups.get(gi)) else {
            return values;
        };
        let mut mask = vec![false; net::DMX_SLOTS];
        for &fi in &g.fixtures {
            if let Some(fx) = self.patch.fixtures.get(fi) {
                let base = fx.from.saturating_sub(1) as usize;
                for ci in 0..fx.channels.len() {
                    if base + ci < mask.len() {
                        mask[base + ci] = true;
                    }
                }
            }
        }
        values
            .into_iter()
            .filter(|&(a, _)| a < mask.len() && mask[a])
            .collect()
    }

    /// Split `fixtures` into effect units — the slots a spread fans across.
    ///
    /// The active order goes first: its steps decide both the sequence and
    /// which fixtures share a phase, so overlapping groups resolve cleanly.
    /// Then any "One fixture" group still wholly unclaimed collapses into a
    /// unit. Whatever is left keeps patch order, one slot each.
    pub(crate) fn effect_units(&self, fixtures: &[usize]) -> Vec<Vec<usize>> {
        let mut remaining: HashSet<usize> = fixtures.iter().copied().collect();
        let mut units: Vec<Vec<usize>> = Vec::new();
        if let Some(o) = self.active_order.and_then(|i| self.orders.get(i)) {
            for step in &o.steps {
                let unit: Vec<usize> = step
                    .fixtures
                    .iter()
                    .copied()
                    .filter(|fi| remaining.remove(fi))
                    .collect();
                if !unit.is_empty() {
                    units.push(unit);
                }
            }
        }
        for group in &self.groups {
            if group.mode == GroupMode::AsFixture
                && !group.fixtures.is_empty()
                && group.fixtures.iter().all(|fi| remaining.contains(fi))
            {
                for fi in &group.fixtures {
                    remaining.remove(fi);
                }
                units.push(group.fixtures.clone());
            }
        }
        for &fi in fixtures {
            if remaining.remove(&fi) {
                units.push(vec![fi]);
            }
        }
        units
    }

    /// A super-fixture behaves as a single light, so the selection can never
    /// hold only part of one: touching any member pulls in the rest. Also
    /// forgets the group chain once nothing is selected, so the next click
    /// starts a fresh route.
    pub(crate) fn sync_selection_units(&mut self) {
        if self.stage.selection.is_empty() {
            self.group_chain.clear();
        }
        let selected: HashSet<usize> = self.stage.selected_fixtures().into_iter().collect();
        let missing: Vec<usize> = self
            .super_fixtures()
            .into_iter()
            .filter(|b| b.iter().any(|fi| selected.contains(fi)))
            .flat_map(|b| b.iter().copied())
            .filter(|fi| !selected.contains(fi))
            .collect();
        for fi in missing {
            self.stage.add_fixture_to_selection(fi);
        }
    }

    /// Put every group the selection fully covers into `mode`. Returns how
    /// many groups changed.
    pub(crate) fn set_covered_groups_mode(&mut self, mode: GroupMode) -> usize {
        let selected: HashSet<usize> = self.stage.selected_fixtures().into_iter().collect();
        let mut changed = 0;
        for g in &mut self.groups {
            if g.fixtures.is_empty() || g.mode == mode {
                continue;
            }
            if g.fixtures.iter().all(|fi| selected.contains(fi)) {
                g.mode = mode;
                changed += 1;
            }
        }
        if changed > 0 {
            group::save_groups(&self.groups);
        }
        changed
    }

    /// Freeze or resume the complete show transport without rewriting effect
    /// state. Resuming moves wall-clock origins past the pause, preserving all
    /// integrated phases and in-flight paths exactly.
    pub fn set_frozen(&mut self, frozen: bool) {
        if frozen == self.frozen {
            return;
        }
        if frozen {
            self.frozen = true;
            self.frozen_at = Some(Instant::now());
            self.log.push("Transport frozen".into());
            return;
        }

        let now = Instant::now();
        let paused = self
            .frozen_at
            .take()
            .map_or(std::time::Duration::ZERO, |at| now.duration_since(at));
        self.frozen = false;
        self.live.resume_clock();
        if let Some(run) = &mut self.transition_run {
            run.resume_after(paused);
        }
        if let Some(run) = &mut self.chase_run {
            run.resume_after(paused);
        }
        for stack in &mut self.stacks {
            stack.resume_after(paused);
        }
        for sc in &mut self.scenes {
            sc.resume_after(paused);
        }
        for ramp in self
            .base_fades
            .values_mut()
            .chain(self.osc_ramps.values_mut())
            .chain(self.add_ramps.values_mut())
        {
            ramp.start += paused;
        }
        if let Some(at) = &mut self.throb_at {
            *at += paused;
        }
        if self.cycle_on {
            self.cycle_last = Some(now);
        }
        self.log.push(format!(
            "Transport resumed after {:.1}s",
            paused.as_secs_f32()
        ));
    }

    pub(crate) fn draw_ui(&mut self, ctx: &egui::Context) {
        for line in crate::plugin::drain_prints() {
            self.log.push(format!("Plugin: {line}"));
        }
        if self.plugin_theme_dirty {
            self.plugin_theme_dirty = false;
            crate::ui::install_with(ctx, &crate::plugin::merged_theme(&self.plugins));
        }
        self.top_bar(ctx);
        self.artnet_window(ctx);
        self.transition_window(ctx);
        self.chases_window(ctx);
        self.beat_window(ctx);
        self.groups_window(ctx);
        self.orders_window(ctx);
        self.scenes_window(ctx);
        self.audio_window(ctx);
        self.palettes_window(ctx);
        self.gobos_window(ctx);
        self.phasers_window(ctx);
        self.phaser_board_window(ctx);
        self.preset_board_window(ctx);
        self.plugins_window(ctx);
        self.plugin_panels(ctx);
        self.stacks_window(ctx);
        self.views_window(ctx);
        self.command_bar(ctx);
        self.executor_bar(ctx);
        self.log_window(ctx);
        self.fixtures_panel(ctx);
        self.inspector_panel(ctx);
        self.confirm_reset_window(ctx);
        self.settings_window(ctx);
        self.network_window(ctx);
        self.patch_window(ctx);
        self.configs_window(ctx);
        self.dmx_test_window(ctx);
        self.central_panel(ctx);
        self.stage_camera_tick(ctx);
        self.show_oscillator(ctx);
        self.safety_windows(ctx);
        // With every edit of the frame in, see whether one happened.
        self.undo_tick(ctx);
        // Last: lift every button drawn above (see `ui::theme::relief_pass`).
        crate::ui::relief_pass(ctx);
    }

    /// Save the programmer's current content — every non-zero channel value
    /// plus running oscillators — as a named native preset.
    pub fn store_user_preset(&mut self, name: String) {
        let source = self
            .transition_run
            .as_ref()
            .map_or_else(|| self.live.clone(), |run| run.pending().clone());
        let mut values: Vec<(usize, u8)> = source
            .base
            .iter()
            .enumerate()
            .filter(|&(_, &v)| v != 0)
            .map(|(a, &v)| (a, v))
            .collect();
        let mut oscs: Vec<(usize, SavedOsc)> = source
            .oscs
            .iter()
            .filter(|(_, o)| o.enabled)
            .map(|(&a, o)| {
                (
                    a,
                    SavedOsc {
                        invert: o.invert,
                        amount: o.amount,
                        phase: o.phase,
                        subdiv: o.subdiv,
                        shape: o.shape,
                        custom_wave: o.custom_wave.clone(),
                        master_beat: o.master_beat,
                        local_beats: o.local_beats,
                        local_tempo: o.local_tempo,
                    },
                )
            })
            .collect();
        // Channels dialled in on the encoder are what you actually see, so
        // they go in as base values — replacing whatever the programmer held
        // there and dropping any wave the knob was overriding.
        let from_encoders = self.encoder_layer.len();
        if from_encoders > 0 {
            values.retain(|(a, _)| !self.encoder_layer.contains_key(a));
            oscs.retain(|(a, _)| !self.encoder_layer.contains_key(a));
            values.extend(
                self.encoder_layer
                    .iter()
                    .filter(|(_, &v)| v != 0)
                    .map(|(&a, &v)| (a, v)),
            );
            values.sort_unstable_by_key(|(a, _)| *a);
        }
        if values.is_empty() && oscs.is_empty() {
            self.log
                .push("Preset: programmer is empty — nothing to store".into());
            return;
        }
        if from_encoders > 0 {
            self.log
                .push(format!("Preset: baked in {from_encoders} encoder channel(s)"));
        }
        self.log.push(format!(
            "Stored preset \"{}\" ({} values, {} oscillators)",
            name,
            values.len(),
            oscs.len()
        ));
        self.user_presets.push(UserPreset {
            id: self.next_preset_id,
            name,
            folder: String::new(),
            values,
            oscs,
            speed: source.speed,
            tempo: source.tempo,
            master_speed: source.master_speed,
            active_phasers: self
                .active_phasers
                .iter()
                .map(|(name, addrs)| (name.clone(), addrs.clone()))
                .collect(),
            add_overrides: self.add_overrides.iter().map(|(&a, &v)| (a, v)).collect(),
            hold_overrides: self.hold_overrides.iter().map(|(&a, &v)| (a, v)).collect(),
            lanes: self.effect_lanes.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            cycle: self.cycle_on.then(|| SavedCycle {
                ids: self.cycle_ids.clone(),
                weights: self.cycle_weights.clone(),
                beats_per: self.cycle_beats_per,
                spread: self.cycle_spread,
                pattern: self.cycle_pattern,
                shape: self.cycle_shape,
                master_beat: self.cycle_master_beat,
                tempo: self.cycle_tempo,
            }),
            color: None,
            symbol: String::new(),
            icon: crate::streamdeck::KeyIcon::None,
            fade: None,
            pinned: false,
        });
        self.next_preset_id += 1;
        preset::save_presets(&self.preset_folders, &self.user_presets);
    }

    /// Carry every running phaser across a preset swap so it keeps playing
    /// on top of the new look: wave oscillators are re-inserted into the
    /// target, pose-owned channels keep their stored positions, and hold
    /// channels are forced post-mixer every frame anyway.
    fn carry_phasers_into(&self, target: &mut Look) {
        // While a transition runs, `live` is blacked out (the run owns the
        // real look) — carry from the newest pending look instead, otherwise
        // phaser channels would get stamped to zero.
        let src: &Look = match &self.transition_run {
            Some(run) => run.pending(),
            None => &self.live,
        };
        for addrs in self.active_phasers.values() {
            for &a in addrs {
                if self.hold_overrides.contains_key(&a) {
                    continue;
                }
                if let Some(osc) = src.oscs.get(&a).or_else(|| self.live.oscs.get(&a)) {
                    target.oscs.insert(a, osc.clone());
                } else {
                    // Pose-owned channel: keep the pose, stop preset motion.
                    target.base[a] = src.base[a];
                    target.oscs.remove(&a);
                }
            }
        }
    }

    /// Recall a native preset — a whole-rig recall like a ShowBuddy preset,
    /// honouring the transition settings.
    pub fn apply_user_preset(&mut self, idx: usize) {
        let Some(p) = self.user_presets.get(idx).cloned() else {
            return;
        };
        let mut target = Look::from_frame(p.base_frame());
        target.oscs = p.osc_map();
        target.speed = p.speed;
        target.tempo = p.tempo;
        target.master_speed = p.master_speed;
        // A preset is a whole-rig recall: the programmer takes over every
        // channel, so anything the preset doesn't set goes to zero and all
        // previous oscillators are replaced.
        self.live_active = (0..net::DMX_SLOTS).collect();
        self.live_refs.clear();
        self.active_preset = None;
        self.active_user_preset = Some(idx);
        // Running phasers ride across preset changes: waves, poses and holds
        // are all carried onto the new look.
        self.carry_phasers_into(&mut target);
        // Saved runtime sources become the underlying snapshot; anything
        // already live remains authoritative above them.
        for (name, addrs) in &p.active_phasers {
            self.active_phasers
                .entry(name.clone())
                .or_insert_with(|| addrs.clone());
        }
        for &(a, v) in &p.add_overrides {
            self.add_overrides.entry(a).or_insert(v);
        }
        for &(a, v) in &p.hold_overrides {
            self.hold_overrides.entry(a).or_insert(v);
        }
        if !self.cycle_on {
            if let Some(cycle) = &p.cycle {
                self.cycle_ids = cycle.ids.clone();
                self.cycle_weights = cycle.weights.clone();
                while self.cycle_weights.len() < self.cycle_ids.len() {
                    self.cycle_weights.push(1.0);
                }
                self.cycle_beats_per = cycle.beats_per;
                self.cycle_spread = cycle.spread;
                self.cycle_pattern = cycle.pattern;
                self.cycle_shape = cycle.shape;
                self.cycle_master_beat = cycle.master_beat;
                self.cycle_tempo = cycle.tempo;
                self.cycle_beats = 0.0;
                self.cycle_last = None;
                self.cycle_on = cycle.ids.len() >= 2;
            }
        }
        // The stacked effect lanes come with the look, replacing whatever
        // was stacked before.
        self.effect_lanes = p.lanes.iter().cloned().collect();

        if self.transition.duration <= 0.0 {
            self.transition_run = None;
            self.live = target;
            *self.net.dmx.lock() = self.live.render();
            self.log.push(format!("Applied preset: {}", p.name));
            return;
        }
        let fixture_positions = self.fixture_positions();
        if let Some(run) = &mut self.transition_run {
            run.push(target, &self.transition, &self.patch, &fixture_positions);
            self.log.push(format!("Queued → {} (chasing)", p.name));
        } else {
            let from = std::mem::replace(&mut self.live, Look::black());
            self.transition_run = Some(TransitionRun::new(
                from,
                target,
                &self.transition,
                &self.patch,
                &fixture_positions,
            ));
            self.log.push(format!(
                "Transitioning to preset: {} ({:.1}s)",
                p.name, self.transition.duration
            ));
        }
    }

    /// Apply a .prt preset to the live DMX buffer.
    pub fn apply_preset(&mut self, bank: usize, idx: usize) {
        let Some(p) = self
            .banks
            .get(bank)
            .and_then(|b| b.presets.get(idx))
            .cloned()
        else {
            return;
        };
        let data = match p.data.clone() {
            Some(data) => data,
            None => match showbuddy::load_preset(&p.path) {
                Ok(data) => data,
                Err(e) => {
                    self.log.push(format!("preset load failed: {e:#}"));
                    return;
                }
            },
        };
        self.active_preset = Some((bank, idx));
        self.active_user_preset = None;
        let animated = data.mods.len();
        let mut target = Look::from_preset(&data);
        // A preset is a whole-rig recall: the programmer takes over every
        // channel, so anything the preset doesn't set goes to zero, palettes
        // stop tracking, and all previous oscillators are replaced.
        self.live_active = (0..net::DMX_SLOTS).collect();
        self.live_refs.clear();
        // Running phasers ride across preset changes: waves, poses and holds
        // are all carried onto the new look.
        self.carry_phasers_into(&mut target);

        if self.transition.duration <= 0.0 {
            // Immediate cut: this look becomes the settled base.
            self.transition_run = None;
            self.live = target;
            *self.net.dmx.lock() = self.live.render();
            self.log.push(format!("Applied preset: {}", p.name));
            return;
        }

        let fixture_positions = self.fixture_positions();
        if let Some(run) = &mut self.transition_run {
            // A blend is already running — queue this one so it chases the
            // looks ahead of it instead of restarting from the output.
            run.push(target, &self.transition, &self.patch, &fixture_positions);
            self.log.push(format!(
                "Queued → {} (chasing, {:.1}s {})",
                p.name,
                self.transition.duration,
                self.transition.mode.label(),
            ));
        } else {
            let from = std::mem::replace(&mut self.live, Look::black());
            self.transition_run = Some(TransitionRun::new(
                from,
                target,
                &self.transition,
                &self.patch,
                &fixture_positions,
            ));
            self.log.push(format!(
                "Transitioning to preset: {} ({:.1}s {}, {} animated channels)",
                p.name,
                self.transition.duration,
                self.transition.mode.label(),
                animated,
            ));
        }
    }

    /// Load a ShowBuddy preset into a [`Look`] (static or animated). Shared by
    /// direct applies and the chase injector.
    pub fn load_look(&mut self, bank: usize, idx: usize) -> Option<(Look, String)> {
        let p = self
            .banks
            .get(bank)
            .and_then(|b| b.presets.get(idx))
            .cloned()?;
        if let Some(data) = &p.data {
            return Some((Look::from_preset(data), p.name));
        }
        match showbuddy::load_preset(&p.path) {
            Ok(data) => Some((Look::from_preset(&data), p.name)),
            Err(e) => {
                self.log.push(format!("preset load failed: {e:#}"));
                None
            }
        }
    }

    /// Begin (or restart) the spherical chase from the selected source preset.
    pub fn start_chase(&mut self) {
        let Some(src) = self.chase.source else {
            self.log
                .push("Pick a preset to inject before starting the chase".into());
            return;
        };
        let loaded = match src {
            ChaseSource::Bank(bank, idx) => self.load_look(bank, idx),
            ChaseSource::User(idx) => self.user_presets.get(idx).map(|p| {
                let mut look = Look::from_frame(p.base_frame());
                look.oscs = p.osc_map();
                look.speed = p.speed;
                look.tempo = p.tempo;
                (look, p.name.clone())
            }),
        };
        match loaded {
            Some((look, name)) => {
                self.chase_run = Some(ChaseRun::new(look));
                self.chase.enabled = true;
                let what = match self.chase.kind {
                    crate::chase::ChaseKind::Glitter => "Glitter started".to_string(),
                    crate::chase::ChaseKind::Pulse => "Pulse sent".to_string(),
                    k => format!("{} chase started", k.label()),
                };
                self.log.push(format!("{what}: injecting {name}"));
            }
            None => self.chase.enabled = false,
        }
    }

    pub fn stop_chase(&mut self) {
        self.chase.enabled = false;
        self.chase_run = None;
    }

    /// Advance the palette/phaser fade ramps: programmer base values,
    /// oscillator depths and flat-add levels ease toward their targets each
    /// frame. Returns whether any ramp is still running.
    fn advance_fades(&mut self) -> bool {
        let any = !self.base_fades.is_empty()
            || !self.osc_ramps.is_empty()
            || !self.add_ramps.is_empty();
        if !any {
            return false;
        }
        let now = Instant::now();
        let mut done: Vec<usize> = Vec::new();
        for (&a, r) in &self.base_fades {
            let (v, fin) = r.value(now);
            self.live.base[a] = v.round().clamp(0.0, 255.0) as u8;
            if fin {
                done.push(a);
            }
        }
        for a in done.drain(..) {
            self.base_fades.remove(&a);
        }
        let mut done_osc: Vec<(usize, bool)> = Vec::new();
        for (&a, r) in &self.osc_ramps {
            let (v, fin) = r.value(now);
            if fin {
                done_osc.push((a, r.remove_after));
            }
            if let Some(o) = self.live.oscs.get_mut(&a) {
                o.amount = v.max(0.0);
            }
        }
        for (a, rm) in done_osc {
            self.osc_ramps.remove(&a);
            if rm {
                self.live.oscs.remove(&a);
            }
        }
        let mut done_add: Vec<(usize, bool)> = Vec::new();
        for (&a, r) in &self.add_ramps {
            let (v, fin) = r.value(now);
            self.add_overrides.insert(a, v.round() as i16);
            if fin {
                done_add.push((a, r.remove_after));
            }
        }
        for (a, rm) in done_add {
            self.add_ramps.remove(&a);
            if rm {
                self.add_overrides.remove(&a);
            }
        }
        true
    }

    /// Build the palette-cycle overlay: steps through the palettes in
    /// `cycle_ids` on the beat. Spacing fans each fixture's phase across the
    /// rig (wave/wings/random), and shape sets each change from a smooth
    /// crossfade (0) to a hard snap (1). Stepped channels (colour wheels)
    /// always snap rather than sweeping through every slot.
    fn cycle_layer(&self) -> Option<Layer> {
        self.cycle_layer_of(&self.cycle_ids, &self.cycle_weights, self.cycle_fade_k())
    }

    /// One island's lane: a single pick holds that palette on every light
    /// it covers; two or more step through them on the cycle clock, with
    /// the cycle's own rate, pattern and spacing.
    fn lane_layer(&self, ids: &[u32]) -> Option<Layer> {
        match ids {
            [] => None,
            [id] => {
                let p = self.palettes.iter().find(|p| p.id == *id)?;
                let mut frame = net::Frame::black();
                let mut weights = Vec::with_capacity(p.values.len());
                for &(a, v) in &p.values {
                    if a < net::DMX_SLOTS {
                        frame[a] = v;
                        weights.push((a, 1.0));
                    }
                }
                (!weights.is_empty()).then(|| Layer::overlay(frame, weights))
            }
            _ => self.cycle_layer_of(ids, &[], 1.0),
        }
    }

    /// The cycle machinery over any list of palettes: `ids` in order, each
    /// holding for its `weights_in` share of a step (1.0 where missing),
    /// the whole overlay scaled by `fade_k`.
    fn cycle_layer_of(&self, ids: &[u32], weights_in: &[f32], fade_k: f32) -> Option<Layer> {
        let entries: Vec<(&Palette, f32)> = ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| {
                self.palettes
                    .iter()
                    .find(|p| p.id == *id)
                    .map(|p| (p, weights_in.get(i).copied().unwrap_or(1.0).max(0.05)))
            })
            .collect();
        if entries.len() < 2 {
            return None;
        }
        let pals: Vec<&Palette> = entries.iter().map(|(p, _)| *p).collect();
        let widths: Vec<f32> = entries.iter().map(|(_, width)| *width).collect();
        let n = pals.len();
        let total_width: f32 = widths.iter().sum();
        let get = |p: &Palette, a: usize| {
            if a >= net::DMX_SLOTS {
                return 0;
            }
            p.values
                .iter()
                .find(|(pa, _)| *pa == a)
                .map_or(0, |&(_, v)| v)
        };
        // Union of every address any palette in the cycle touches.
        let mut addrs: HashSet<usize> = HashSet::new();
        for p in &pals {
            for &(a, _) in &p.values {
                if a < net::DMX_SLOTS {
                    addrs.insert(a);
                }
            }
        }
        // Group the addresses by effect unit — the active order's steps, then
        // whatever is left in patch order — so spacing can offset each unit's
        // position in the cycle. A super-fixture takes one slot, so the cycle
        // treats it as a single light.
        let all: Vec<usize> = (0..self.patch.fixtures.len()).collect();
        let mut fix_addrs: Vec<Vec<usize>> = Vec::new();
        let mut claimed: HashSet<usize> = HashSet::new();
        for unit in self.effect_units(&all) {
            let mut mine: Vec<usize> = Vec::new();
            for fi in unit {
                let Some(f) = self.patch.fixtures.get(fi) else {
                    continue;
                };
                let from0 = f.from as usize - 1;
                mine.extend((from0..from0 + f.channel_count()).filter(|a| addrs.contains(a)));
            }
            if !mine.is_empty() {
                claimed.extend(mine.iter().copied());
                fix_addrs.push(mine);
            }
        }
        let orphans: Vec<usize> = addrs.iter().copied().filter(|a| !claimed.contains(a)).collect();
        if !orphans.is_empty() {
            fix_addrs.push(orphans);
        }
        let m = fix_addrs.len().max(1);
        let pos = self.cycle_beats / self.cycle_beats_per.max(0.01);
        // Fade window: shape 0 fades the whole step, shape 1 snaps.
        let fade_w = (1.0 - self.cycle_shape).clamp(0.0, 1.0);
        let mut frame = net::Frame::black();
        let mut weights: Vec<(usize, f32)> = Vec::with_capacity(addrs.len());
        for (k, mine) in fix_addrs.iter().enumerate() {
            let disp = match self.cycle_pattern {
                SeqPattern::Wave => k as f32 / m as f32,
                SeqPattern::Wings => {
                    let x = k as f32 / (m - 1).max(1) as f32;
                    1.0 - (x * 2.0 - 1.0).abs()
                }
                SeqPattern::Random => {
                    crate::chase::rand01((k as u32).wrapping_mul(2654435761).wrapping_add(7))
                }
            };
            // Past one full cycle the spacing breaks the pattern up rather
            // than winding more of it on (see `phaser::scatter_offset`).
            let offset = crate::phaser::scatter_offset(k, disp, self.cycle_spread);
            let pk = (pos - offset * total_width).rem_euclid(total_width);
            let mut start = 0.0;
            let mut idx = n - 1;
            for (i, width) in widths.iter().enumerate() {
                if pk < start + *width {
                    idx = i;
                    break;
                }
                start += *width;
            }
            let frac = ((pk - start) / widths[idx]).clamp(0.0, 1.0);
            let (cur, next) = (pals[idx], pals[(idx + 1) % n]);
            // Blend amount: hold, then fade into the next colour across the
            // last `fade_w` of the step (b stays 0 the whole step at snap).
            let b = if fade_w < 0.001 {
                0.0
            } else {
                ((frac - (1.0 - fade_w)) / fade_w).clamp(0.0, 1.0)
            };
            for &a in mine {
                let va = get(cur, a) as f32;
                let vb = get(next, a) as f32;
                let v = if self.channel_is_stepped(a) {
                    if b < 0.5 {
                        va
                    } else {
                        vb
                    }
                } else {
                    va + (vb - va) * b
                };
                frame[a] = v.round().clamp(0.0, 255.0) as u8;
                weights.push((a, fade_k));
            }
        }
        Some(Layer::overlay(frame, weights))
    }

    pub fn drain_events(&mut self) {
        while let Ok(evt) = self.net.evt_rx.try_recv() {
            match evt {
                NetEvent::Discovered(node) => {
                    if !self.nodes.iter().any(|n| n.ip == node.ip) {
                        self.log
                            .push(format!("Discovered {} ({})", node.ip, node.short_name));
                        // Auto-select first discovered node.
                        if self.selected.is_none() {
                            self.selected = Some(node.ip);
                            let _ = self.net.cmd_tx.send(NetCmd::SetTarget(Some(node.ip)));
                        }
                        self.nodes.push(node);
                    }
                }
                NetEvent::Status(s) => self.log.push(s),
            }
        }
        if self.log.len() > 200 {
            let excess = self.log.len() - 200;
            self.log.drain(0..excess);
        }
    }
}

impl eframe::App for App {
    fn on_exit(&mut self) {
        self.insp.prefs.stage.last_camera = Some(self.stage.cam.snapshot());
        self.insp.prefs.save();
        // One last backup if anything changed since the previous one.
        self.backup_now_if_changed("exit");
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        // Ctrl+Shift+Z first, before any text field can see the chord.
        self.undo_keys(ctx);
        // Keep super-fixtures whole before anything reads the selection.
        self.sync_selection_units();

        // Elgato Stream Deck: drain presses/encoder turns and republish key
        // images before the possible early return below, so a knob (e.g.
        // the Master Beat press that un-freezes) stays responsive even
        // while frozen.
        self.sync_deck();

        // A true sample-and-hold: no renderer is called because renderers own
        // their clocks. Art-Net keeps transmitting the unchanged DMX buffer.
        if self.frozen {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            self.draw_ui(ctx);
            return;
        }

        // Master BPM: one clock drives every look's tempo, so beat-synced
        // oscillators stay locked across preset swaps, transitions and chases.
        if self.master_bpm_on {
            self.live.tempo = self.master_bpm;
            if let Some(run) = &mut self.transition_run {
                run.set_tempo(self.master_bpm);
            }
            if let Some(run) = &mut self.chase_run {
                run.set_tempo(self.master_bpm);
            }
        }

        // Direction, rate, pendulum and stutter, published for every clock.
        self.advance_time_machine();

        // Fixture world positions are only needed to place the chase band.
        let chase_active = self.chase.enabled && self.chase_run.is_some();
        let positions = if chase_active {
            self.fixture_positions()
        } else {
            Vec::new()
        };

        // Advance the palette-cycle beat clock from the live tempo — the
        // colour cycle and any stepping effect lane share it.
        if self.cycle_on || self.lanes_cycling() {
            let now = Instant::now();
            let dt = self
                .cycle_last
                .map_or(0.0, |t| now.duration_since(t).as_secs_f32());
            self.cycle_last = Some(now);
            let bpm = if self.cycle_master_beat && self.master_bpm_on {
                self.master_bpm
            } else {
                self.cycle_tempo
            };
            // Palette cycles ride the time machine along with everything else.
            self.cycle_beats += dt * oscillator::time_warp() * bpm.max(1.0) / 60.0;
            if self.cycle_master_beat && self.cycle_beat_nudge.abs() > 0.0001 {
                let step = self.cycle_beat_nudge * (1.0 - (-4.0 * dt).exp());
                self.cycle_beats += step;
                self.cycle_beat_nudge -= step;
            } else if self.cycle_beat_nudge.abs() <= 0.0001 {
                self.cycle_beat_nudge = 0.0;
            }
        } else {
            self.cycle_last = None;
        }

        // Ease palette/phaser fade ramps toward their targets.
        let fades_active = self.advance_fades();

        // Assemble this frame's layer stack and flatten it through the mixer.
        // Bottom→top: every playing stack (cue list), then the programmer on
        // top of the channels it is actively holding, then the chase overlay.
        // The programmer only asserts its active channels, so cues show through
        // everywhere it is not working.
        let mut repaint_ms = 50;
        if fades_active {
            repaint_ms = 25;
        }
        let mut finished_transition = false;
        // Programmer source: a running fade, else the settled `live` look.
        let prog_frame = if let Some(run) = &mut self.transition_run {
            let (rendered, done) = run.render();
            finished_transition = done;
            repaint_ms = 25;
            rendered
        } else {
            let f = self.live.render();
            if self.live.is_animated() {
                repaint_ms = 25;
            }
            f
        };

        let mut chase_layer = None;
        let mut pulse_done = false;
        if chase_active {
            if let Some(run) = &mut self.chase_run {
                chase_layer = Some(run.layer(&self.chase, &self.patch, &positions));
                repaint_ms = 25;
                pulse_done = run.pulse_done(&self.chase);
            }
        }
        if pulse_done {
            self.chase.enabled = false;
            self.chase_run = None;
        }

        self.mixer.begin();
        // Stacks (cue lists) play beneath the programmer, in pool order.
        for st in &mut self.stacks {
            if let Some(layer) = st.render_layer() {
                self.mixer.push(layer);
                if st.is_fading() {
                    repaint_ms = repaint_ms.min(25);
                }
            }
        }
        // Scenes layer among themselves in pool order (later = higher
        // priority) and all sit under the programmer, so you can keep
        // building over whatever is running.
        self.advance_scene_chain();
        for sc in &mut self.scenes {
            let animated = sc.is_animated() || sc.is_fading();
            if let Some(layer) = sc.layer() {
                self.mixer.push(layer);
            }
            if animated && sc.is_running() {
                repaint_ms = repaint_ms.min(25);
            }
        }
        // Plugin layers ride with the scenes, under the programmer, so
        // whatever a script paints never fights the desk for control.
        if self.plugins.iter().any(|p| p.enabled && p.has_frame) {
            let t = self.plugin_epoch.elapsed().as_secs_f64();
            let rig = self.plugin_rig();
            for p in &mut self.plugins {
                if !(p.enabled && p.has_frame) {
                    continue;
                }
                if let Some(layer) = p.layer(&self.plugin_engine, t, rig.clone()) {
                    self.mixer.push(layer);
                }
            }
            repaint_ms = repaint_ms.min(25);
        }
        // The programmer asserts only the channels it is actively holding.
        if !self.live_active.is_empty() {
            let weights: Vec<(usize, f32)> =
                self.live_active.iter().map(|&a| (a, 1.0)).collect();
            self.mixer.push(Layer::overlay(prog_frame, weights));
        }
        // The palette cycle overlays its colour steps above the programmer.
        self.settle_cycle_fade();
        if self.cycle_on {
            if let Some(layer) = self.cycle_layer() {
                self.mixer.push(layer);
                repaint_ms = repaint_ms.min(25);
            }
        }
        // Effect lanes stack above the cycle: an island with one pick holds
        // it, with two or more steps through them on the same clock.
        let lanes: Vec<Layer> = self
            .effect_lanes
            .values()
            .filter_map(|ids| self.lane_layer(ids))
            .collect();
        if self.lanes_cycling() {
            repaint_ms = repaint_ms.min(25);
        }
        for layer in lanes {
            self.mixer.push(layer);
        }
        // Audio triggers: band-threshold flashes ride above the cycle so
        // whatever the music does stays visible over the settled look.
        if self.audio.is_running() {
            let analysis = self.audio.analysis();
            let n = self.audio.beats();
            if n != self.audio_beat_seen {
                self.audio_beat_seen = n;
                self.audio_beat_flash = Some(Instant::now());
                // The tracker presses TAP like a very patient drummer.
                if self.audio_follow_beat {
                    self.beat_tap();
                }
            }
            let now = Instant::now();
            let dt = self
                .audio_last_eval
                .map_or(0.05, |t| now.duration_since(t).as_secs_f32())
                .clamp(0.001, 0.25);
            self.audio_last_eval = Some(now);
            let resolved: Vec<Vec<(usize, u8)>> = self
                .audio_triggers
                .iter()
                .map(|t| {
                    if t.enabled {
                        self.trigger_values(t)
                    } else {
                        Vec::new()
                    }
                })
                .collect();
            for (t, values) in self.audio_triggers.iter_mut().zip(&resolved) {
                if !t.enabled {
                    t.env = 0.0;
                    continue;
                }
                let energy = t.energy(&analysis.spectrum);
                let w = t.advance(energy, dt);
                if let Some(layer) = AudioTrigger::layer(values, w, t.merge) {
                    self.mixer.push(layer);
                }
            }
            repaint_ms = repaint_ms.min(25);
        } else {
            self.audio_last_eval = None;
        }
        if self.send_to_deck {
            repaint_ms = repaint_ms.min(100);
        }
        // The chase overlays on top of everything.
        if let Some(layer) = chase_layer {
            self.mixer.push(layer);
        }
        // Compose the whole output frame locally and publish it with one
        // write at the end. The Art-Net sender snapshots the shared buffer on
        // its own clock, so staging the mix and then patching the grand
        // master, holds and overrides onto it under separate locks let it
        // catch a half-built frame — one packet of the bare palette colour
        // showing through a lock/hold phaser every second or so.
        let mixed = self.mixer.render();
        self.mixed = mixed;
        let mut out = mixed;

        // Encoder layer: channels dialled in on the programmer knob override
        // the mix outright, but sit under the grand master so a dimmer set
        // by hand still fades with the rig.
        if !self.encoder_layer.is_empty() {
            for (&a, &v) in &self.encoder_layer {
                if a < out.len() {
                    out[a] = v;
                }
            }
        }

        // Grand master: scale every dimmer channel in the rig.
        if self.grand_master < 0.999 {
            let gm = self.grand_master.clamp(0.0, 1.0);
            for fx in &self.patch.fixtures {
                let base = fx.from.saturating_sub(1) as usize;
                for (ci, ch) in fx.channels.iter().enumerate() {
                    if ch.role() == Role::Dimmer {
                        let a = base + ci;
                        if a < out.len() {
                            out[a] = (out[a] as f32 * gm).round() as u8;
                        }
                    }
                }
            }
        }

        // Dimmer throb: surge every dimmer toward full, decaying back over
        // ~half a second (sits above the grand master, below holds).
        if let Some(t0) = self.throb_at {
            let t = t0.elapsed().as_secs_f32() / 0.5;
            if t >= 1.0 {
                self.throb_at = None;
            } else {
                let boost = (255.0 * (1.0 - t).powf(1.6)) as u8;
                for fx in &self.patch.fixtures {
                    let base = fx.from.saturating_sub(1) as usize;
                    for (ci, ch) in fx.channels.iter().enumerate() {
                        if ch.role() == Role::Dimmer {
                            let a = base + ci;
                            if a < out.len() {
                                out[a] = out[a].max(boost);
                            }
                        }
                    }
                }
                repaint_ms = repaint_ms.min(16);
            }
        }

        // Compress dimmer channels with embedded strobe/macro bands into
        // their usable dimming range (e.g. moving par: logical 0–255 →
        // 8–134), so faders and phasers never strafe the strobe section.
        // Hold/lock/FX tiles and the DMX test window are applied after this
        // and stay raw — that's the deliberate way in to the strobe bands.
        let mut dim_caps: HashMap<usize, u8> = HashMap::new();
        {
            for fx in &self.patch.fixtures {
                let base = fx.from.saturating_sub(1) as usize;
                for (ci, ch) in fx.channels.iter().enumerate() {
                    let Some((lo, hi)) = ch.dim_range() else {
                        continue;
                    };
                    let a = base + ci;
                    if a < out.len() {
                        if out[a] > 0 {
                            let span = (hi - lo) as f32;
                            out[a] =
                                lo + (out[a] as f32 / 255.0 * span).round() as u8;
                        }
                        dim_caps.insert(a, hi);
                    }
                }
            }
        }

        // Flat-add phasers ride on top of the mix (and the grand master),
        // adding or subtracting a constant level until stopped.
        if !self.add_overrides.is_empty() {
            for (&a, &v) in &self.add_overrides {
                if a < out.len() {
                    let hi = dim_caps.get(&a).copied().unwrap_or(255);
                    out[a] = (out[a] as i16 + v).clamp(0, hi as i16) as u8;
                }
            }
        }

        // Hold phasers (e.g. smoke on) sit on top of everything — presets,
        // blackout fades and the grand master — except the DMX test window.
        if !self.hold_overrides.is_empty() {
            for (&a, &v) in &self.hold_overrides {
                if a < out.len() {
                    out[a] = v;
                }
            }
        }

        // DMX test overrides beat everything — they are the wire truth.
        if !self.test_overrides.is_empty() {
            for (&a, &v) in &self.test_overrides {
                if a < out.len() {
                    out[a] = v;
                }
            }
        }

        // Blackout (Stream Deck Master Dimmer press) wins over everything,
        // including DMX test overrides — a non-destructive override, not a
        // clear, so the show picks back up untouched once it's lifted.
        if self.blackout {
            out = net::Frame::black();
        }
        *self.net.dmx.lock() = out;

        if finished_transition {
            if let Some(run) = self.transition_run.take() {
                // Oscillators armed while the run was in flight (e.g. a phaser
                // applied mid-transition) live in the blacked-out programmer
                // look — fold them into the settled look so they keep playing.
                let armed_mid_run = std::mem::take(&mut self.live.oscs);
                self.live = run.finish();
                self.live.oscs.extend(armed_mid_run);
                self.log.push("Transition complete".into());
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(repaint_ms));
        self.draw_ui(ctx);
    }
}
