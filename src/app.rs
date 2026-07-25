//! Application state and the top-level eframe update loop. The actual panel
//! rendering lives in the `ui` module; `update` just dispatches to it.

use std::collections::{HashMap, HashSet};
use std::time::Instant;
use std::net::Ipv4Addr;

use eframe::egui;

use crate::net::{self, DiscoveredNode, NetCmd, NetEvent, NetHandle};
use crate::oscillator::{self, CustomWaveform, Look};
use crate::palette::{self, Feature, Palette, PaletteRef, PaletteSeq, SeqPattern};
use crate::phaser::{self, Phaser};
use crate::preset::{self, SavedCycle, SavedOsc, UserPreset};
use crate::profiles::{self, UserFixture};
use crate::stack::{self, Stack};
use crate::view::{self, View};
use crate::showbuddy::{self, Patch, PresetBank, Role};
use crate::stage::{self, StageView, V3};
use crate::transition::{TransitionBinding, TransitionConfig, TransitionRun};
use crate::chase::{ChaseConfig, ChaseRun, ChaseSource};
use crate::engine::{Layer, Mixer};
use crate::group::{self, Group};

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
    /// Dimmer throb: moment the 💥 button was hit — dimmers surge to full
    /// and decay back over ~half a second.
    pub throb_at: Option<Instant>,
    /// Global transport hold. While set, the last transmitted frame remains
    /// untouched and no animation, fade, chase, cue, or transition advances.
    pub frozen: bool,
    frozen_at: Option<Instant>,
    /// The output compositor: each frame's layer stack (base look + overlays,
    /// and later Decks/Phasers) flattened into the frame sent to Art-Net.
    pub mixer: Mixer,
    /// Stored fixture selections (the renamed grandMA3 Group pool).
    pub groups: Vec<Group>,
    /// Name field for storing the current selection as a group.
    pub group_name: String,
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
    /// When true, inspector presets drag (for filing/reordering) instead of
    /// selecting on click.
    pub preset_drag: bool,
    /// Native DMXpress presets (saved programmer snapshots incl. oscillators).
    pub user_presets: Vec<UserPreset>,
    /// Preset folders (may be empty); presets reference them by name.
    pub preset_folders: Vec<String>,
    /// Which preset folders are expanded in the inspector.
    pub open_user_folders: HashSet<String>,
    pub preset_name: String,
    pub active_user_preset: Option<usize>,
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
    /// Cue lists (the renamed grandMA3 Sequence/cuelist pool).
    pub stacks: Vec<Stack>,
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
    /// Current text in the command line.
    pub command: String,
    pub settings: stage::Settings,
    pub show_settings: bool,
    pub show_artnet: bool,
    pub show_transition: bool,
    pub show_chases: bool,
    pub show_groups: bool,
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
    /// Independent UI zoom for each major panel (1.0 = default).
    pub zoom: PanelZoom,
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
    pub palettes: f32,
    pub phasers: f32,
    pub stacks: f32,
    pub views: f32,
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
            palettes: 1.0,
            phasers: 1.0,
            stacks: 1.0,
            views: 1.0,
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
        profiles::extend_patch(&mut patch, &user_patch);
        if !user_patch.fixtures.is_empty() {
            log.push(format!("Patched {} DMXpress fixtures", user_patch.fixtures.len()));
        }
        for w in &patch.warnings {
            log.push(format!("patch warning: {w}"));
        }
        let sel_fixture = if patch.fixtures.is_empty() { None } else { Some(0) };
        let settings = stage::Settings::load();
        let mut stage = StageView::new();
        stage.sync(&patch, &settings);
        let banks = if user_patch.include_showbuddy {
            load_banks(&mut log)
        } else {
            Vec::new()
        };
        let palettes = palette::load_palettes();
        let next_palette_id = palettes.iter().map(|p| p.id).max().map_or(0, |m| m + 1);
        let seq_store = palette::load_seqs();
        let preset_store = preset::load_presets();
        Self {
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
            groups: group::load_groups(),
            group_name: String::new(),
            palettes,
            next_palette_id,
            palette_name: String::new(),
            palette_tab: Feature::Color,
            cycle_ids: Vec::new(),
            cycle_weights: Vec::new(),
            advanced_palette_mode: false,
            cycle_seq: None,
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
            preset_drag: false,
            user_presets: preset_store.presets,
            preset_folders: preset_store.folders,
            open_user_folders: HashSet::new(),
            preset_name: String::new(),
            active_user_preset: None,
            phaser_edit: Phaser::default(),
            phaser_name: String::new(),
            phaser_edit_mode: false,
            phaser_edit_sel: None,
            phaser_live: false,
            master_bpm: 120.0,
            master_bpm_on: false,
            beat_taps: Vec::new(),
            stacks: stack::load_stacks(),
            cur_stack: None,
            cue_fade: 3.0,
            grand_master: 1.0,
            record_mask: Feature::ALL.iter().copied().collect(),
            views: view::load_views(),
            view_name: String::new(),
            command: String::new(),
            settings,
            show_settings: false,
            show_artnet: false,
            show_transition: false,
            show_chases: false,
            show_groups: false,
            show_beat: false,
            show_palettes: false,
            show_phasers: false,
            show_stacks: false,
            show_decks: false,
            show_command: false,
            show_views: false,
            show_log: true,
            show_osc: true,
            setup_name: String::new(),
            user_fixtures: user_patch.fixtures,
            include_showbuddy: user_patch.include_showbuddy,
            showbuddy_patch,
            excluded_fixtures: user_patch.excluded,
            show_patch: false,
            patch_profile: 0,
            patch_name: String::new(),
            patch_addr: 1,
            patch_count: 1,
            show_configs: false,
            show_dmx_test: false,
            config_name: String::new(),
            confirm_delete_config: None,
            confirm_new_show: false,
            new_show_drop_showbuddy: true,
            new_show_reset_layout: true,
            sel_channels: HashSet::new(),
            zoom: PanelZoom::default(),
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
        profiles::extend_patch(&mut patch, &self.current_user_patch());
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
        self.live_refs.clear();
        self.live_active.clear();
        self.active_phasers.clear();
        self.hold_overrides.clear();
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
        self.open_user_folders.clear();
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.active_user_preset = None;
        self.views.clear();
        view::save_views(&self.views);
        self.live = Look::black();
        self.live_refs.clear();
        self.live_active.clear();
        self.active_phasers.clear();
        self.hold_overrides.clear();
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
            palettes: self.palettes.clone(),
            phasers: self.phasers.clone(),
            user_presets: self.user_presets.clone(),
            preset_folders: self.preset_folders.clone(),
            stacks: self.stacks.clone(),
            views: self.views.clone(),
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
        group::save_groups(&self.groups);
        self.palettes = cfg.palettes;
        self.next_palette_id = self.palettes.iter().map(|p| p.id).max().map_or(0, |m| m + 1);
        palette::save_palettes(&self.palettes);
        self.phasers = cfg.phasers;
        phaser::save_phasers(&self.phasers);
        self.user_presets = cfg.user_presets;
        self.preset_folders = cfg.preset_folders;
        self.open_user_folders.clear();
        preset::save_presets(&self.preset_folders, &self.user_presets);
        self.active_user_preset = None;
        self.stacks = cfg.stacks;
        stack::save_stacks(&self.stacks);
        self.cur_stack = None;
        self.views = cfg.views;
        view::save_views(&self.views);
        self.universe = cfg.universe;
        let _ = self.net.cmd_tx.send(NetCmd::SetUniverse(self.universe));
        self.grand_master = cfg.grand_master;
        self.cue_fade = cfg.cue_fade;
        // Everything referencing old fixture indices is invalid now.
        self.live = Look::black();
        self.active_phasers.clear();
        self.hold_overrides.clear();
        self.transition_run = None;
        self.chase_run = None;
        self.active_preset = None;
    }

    /// Fixtures that have a resolved stage position, paired with that position.
    fn fixture_positions(&self) -> Vec<(usize, V3)> {
        self.stage
            .fixture_positions(&self.patch)
            .into_iter()
            .enumerate()
            .filter_map(|(fi, pos)| pos.map(|pos| (fi, pos)))
            .collect()
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

    fn draw_ui(&mut self, ctx: &egui::Context) {
        self.top_bar(ctx);
        self.artnet_window(ctx);
        self.transition_window(ctx);
        self.chases_window(ctx);
        self.beat_window(ctx);
        self.groups_window(ctx);
        self.palettes_window(ctx);
        self.phasers_window(ctx);
        self.stacks_window(ctx);
        self.views_window(ctx);
        self.command_bar(ctx);
        self.executor_bar(ctx);
        self.log_window(ctx);
        self.fixtures_panel(ctx);
        self.inspector_panel(ctx);
        self.confirm_reset_window(ctx);
        self.settings_window(ctx);
        self.patch_window(ctx);
        self.configs_window(ctx);
        self.dmx_test_window(ctx);
        self.central_panel(ctx);
        self.show_oscillator(ctx);
    }

    /// Save the programmer's current content — every non-zero channel value
    /// plus running oscillators — as a named native preset.
    pub fn store_user_preset(&mut self, name: String) {
        let source = self
            .transition_run
            .as_ref()
            .map_or_else(|| self.live.clone(), |run| run.pending().clone());
        let values: Vec<(usize, u8)> = source
            .base
            .iter()
            .enumerate()
            .filter(|&(_, &v)| v != 0)
            .map(|(a, &v)| (a, v))
            .collect();
        let oscs: Vec<(usize, SavedOsc)> = source
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
        if values.is_empty() && oscs.is_empty() {
            self.log
                .push("Preset: programmer is empty — nothing to store".into());
            return;
        }
        self.log.push(format!(
            "Stored preset \"{}\" ({} values, {} oscillators)",
            name,
            values.len(),
            oscs.len()
        ));
        self.user_presets.push(UserPreset {
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
        });
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
                    crate::chase::ChaseKind::Sphere => "Spherical chase started",
                    crate::chase::ChaseKind::Linear => "Linear chase started",
                    crate::chase::ChaseKind::Boomerang => "Boomerang chase started",
                    crate::chase::ChaseKind::Stripes => "Stripe chase started",
                    crate::chase::ChaseKind::Glitter => "Glitter started",
                    crate::chase::ChaseKind::Pulse => "Pulse sent",
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
        let entries: Vec<(&Palette, f32)> = self
            .cycle_ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| {
                self.palettes
                    .iter()
                    .find(|p| p.id == *id)
                    .map(|p| (p, self.cycle_weights.get(i).copied().unwrap_or(1.0).max(0.05)))
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
        // Group the addresses by fixture (patch order) so spacing can offset
        // each fixture's position in the cycle.
        let mut fix_addrs: Vec<Vec<usize>> = Vec::new();
        let mut claimed: HashSet<usize> = HashSet::new();
        for f in &self.patch.fixtures {
            let from0 = f.from as usize - 1;
            let mine: Vec<usize> = (from0..from0 + f.channel_count())
                .filter(|a| addrs.contains(a))
                .collect();
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
            let pk = (pos - self.cycle_spread * disp * total_width)
                .rem_euclid(total_width);
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
                weights.push((a, 1.0));
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();

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

        // Fixture world positions are only needed to place the chase band.
        let chase_active = self.chase.enabled && self.chase_run.is_some();
        let positions = if chase_active {
            self.fixture_positions()
        } else {
            Vec::new()
        };

        // Advance the palette-cycle beat clock from the live tempo.
        if self.cycle_on {
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
            self.cycle_beats += dt * bpm.max(1.0) / 60.0;
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
        // The programmer asserts only the channels it is actively holding.
        if !self.live_active.is_empty() {
            let weights: Vec<(usize, f32)> =
                self.live_active.iter().map(|&a| (a, 1.0)).collect();
            self.mixer.push(Layer::overlay(prog_frame, weights));
        }
        // The palette cycle overlays its colour steps above the programmer.
        if self.cycle_on {
            if let Some(layer) = self.cycle_layer() {
                self.mixer.push(layer);
                repaint_ms = repaint_ms.min(25);
            }
        }
        // The chase overlays on top of everything.
        if let Some(layer) = chase_layer {
            self.mixer.push(layer);
        }
        *self.net.dmx.lock() = self.mixer.render();

        // Grand master: scale every dimmer channel in the rig.
        if self.grand_master < 0.999 {
            let gm = self.grand_master.clamp(0.0, 1.0);
            let mut out = self.net.dmx.lock();
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
                let mut out = self.net.dmx.lock();
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
            let mut out = self.net.dmx.lock();
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
            let mut out = self.net.dmx.lock();
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
            let mut out = self.net.dmx.lock();
            for (&a, &v) in &self.hold_overrides {
                if a < out.len() {
                    out[a] = v;
                }
            }
        }

        // DMX test overrides beat everything — they are the wire truth.
        if !self.test_overrides.is_empty() {
            let mut out = self.net.dmx.lock();
            for (&a, &v) in &self.test_overrides {
                if a < out.len() {
                    out[a] = v;
                }
            }
        }

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
