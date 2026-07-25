//! The `StageView` widget state: instances, towers, selection, persistence,
//! and the fixture-level selection / duplication / tower helpers.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::gizmo::Drag;
use super::layout::{
    default_transform, layout_key, Instance, LayoutFile, LightTransform, SavedInstance, Tower,
    TOWER_SLOTS,
};
use super::math::{v3, Camera};
use super::settings::Settings;
use super::{LAYOUT_FILE, SETUPS_DIR};
use crate::showbuddy::Patch;

pub struct StageView {
    pub cam: Camera,
    /// Free-space camera navigation instead of orbiting a focal point.
    pub fly_mode: bool,
    pub fly_speed: f32,
    /// Visual lights; several may reference the same patch fixture.
    pub instances: Vec<Instance>,
    pub towers: Vec<Tower>,
    /// Selected instance indices.
    pub selection: HashSet<usize>,
    pub sel_tower: Option<usize>,
    /// Whether the stage box is selected (shows its resize arrows).
    pub sel_stage: bool,
    /// Most recently picked *fixture* index (drives the channel editor).
    pub last_selected: Option<usize>,
    pub(crate) drag: Drag,
    /// Layout snapshots (instances, towers) for undo — newest last.
    pub(crate) undo_stack: Vec<(Vec<Instance>, Vec<Tower>)>,
    /// While dragging: which tower slot each light is hovering over (instance
    /// index → (tower, slot)). Drives the live snap preview/highlight.
    pub(crate) snap_preview: HashMap<usize, (usize, usize)>,
    /// Pointer angle (radians) captured for the active rotation-ring drag.
    pub(crate) gizmo_last_angle: f32,
    pub(crate) layout_path: PathBuf,
}

/// How many edit steps the visualizer can undo.
pub(crate) const UNDO_DEPTH: usize = 10;

impl StageView {
    pub fn new() -> Self {
        Self {
            cam: Camera::default(),
            fly_mode: false,
            fly_speed: 5.0,
            instances: Vec::new(),
            towers: Vec::new(),
            selection: HashSet::new(),
            sel_tower: None,
            sel_stage: false,
            last_selected: None,
            drag: Drag::None,
            undo_stack: Vec::new(),
            snap_preview: HashMap::new(),
            gizmo_last_angle: 0.0,
            layout_path: PathBuf::from(LAYOUT_FILE),
        }
    }

    /// Snapshot the current arrangement before a mutating gesture so it can
    /// be undone. Keeps at most `UNDO_DEPTH` steps.
    pub(crate) fn push_undo(&mut self) {
        self.undo_stack
            .push((self.instances.clone(), self.towers.clone()));
        if self.undo_stack.len() > UNDO_DEPTH {
            let drop = self.undo_stack.len() - UNDO_DEPTH;
            self.undo_stack.drain(0..drop);
        }
    }

    /// Restore the most recent pre-edit snapshot (⌘Z).
    pub fn undo(&mut self, patch: &Patch) -> bool {
        let Some((instances, towers)) = self.undo_stack.pop() else {
            return false;
        };
        self.instances = instances;
        self.towers = towers;
        self.selection.retain(|&i| i < self.instances.len());
        if self.sel_tower.is_some_and(|t| t >= self.towers.len()) {
            self.sel_tower = None;
        }
        self.drag = Drag::None;
        self.save(patch);
        true
    }

    fn read_layout(&self) -> LayoutFile {
        let Ok(text) = std::fs::read_to_string(&self.layout_path) else {
            return LayoutFile::default();
        };
        if let Ok(lf) = serde_json::from_str::<LayoutFile>(&text) {
            if !lf.instances.is_empty() || !lf.towers.is_empty() {
                return lf;
            }
        }
        // Legacy { key -> transform } map.
        let map: HashMap<String, LightTransform> =
            serde_json::from_str(&text).unwrap_or_default();
        LayoutFile {
            instances: map
                .into_iter()
                .map(|(key, t)| SavedInstance {
                    key,
                    t,
                    opacity: 1.0,
                    mount: None,
                })
                .collect(),
            towers: Vec::new(),
        }
    }

    fn apply_layout(&mut self, patch: &Patch, set: &Settings, lf: LayoutFile) {
        let key_to_fixture: HashMap<String, usize> = patch
            .fixtures
            .iter()
            .enumerate()
            .map(|(i, f)| (layout_key(f), i))
            .collect();
        self.towers = lf.towers;
        let mut instances: Vec<Instance> = Vec::new();
        for si in lf.instances {
            if let Some(&fi) = key_to_fixture.get(&si.key) {
                let mount = si
                    .mount
                    .filter(|(ti, s)| *ti < self.towers.len() && *s < TOWER_SLOTS);
                instances.push(Instance {
                    fixture: fi,
                    t: si.t,
                    opacity: si.opacity,
                    mount,
                });
            }
        }
        // Every patched fixture gets at least one instance.
        for (fi, f) in patch.fixtures.iter().enumerate() {
            if !instances.iter().any(|inst| inst.fixture == fi) {
                instances.push(Instance {
                    fixture: fi,
                    t: default_transform(f, set),
                    opacity: 1.0,
                    mount: None,
                });
            }
        }
        self.instances = instances;
        self.selection.clear();
        self.sel_tower = None;
        self.last_selected = None;
        self.drag = Drag::None;
        self.undo_stack.clear();
    }

    /// (Re)build light instances for a freshly loaded patch, keeping any
    /// previously saved placement, duplicates and towers.
    pub fn sync(&mut self, patch: &Patch, set: &Settings) {
        let lf = self.read_layout();
        self.apply_layout(patch, set, lf);
    }

    fn layout_data(&self, patch: &Patch) -> LayoutFile {
        LayoutFile {
            instances: self
                .instances
                .iter()
                .filter_map(|inst| {
                    patch.fixtures.get(inst.fixture).map(|f| SavedInstance {
                        key: layout_key(f),
                        t: inst.t.clone(),
                        opacity: inst.opacity,
                        mount: inst.mount,
                    })
                })
                .collect(),
            towers: self.towers.clone(),
        }
    }

    pub fn save(&self, patch: &Patch) {
        if let Ok(json) = serde_json::to_string_pretty(&self.layout_data(patch)) {
            let _ = std::fs::write(&self.layout_path, json);
        }
    }

    /// Snapshot the current arrangement for embedding in a configuration.
    pub(crate) fn export_layout(&self, patch: &Patch) -> LayoutFile {
        self.layout_data(patch)
    }

    /// Replace the current arrangement with one from a configuration.
    pub(crate) fn import_layout(&mut self, patch: &Patch, set: &Settings, lf: LayoutFile) {
        self.apply_layout(patch, set, lf);
        self.save(patch);
    }

    /// Discard the saved layout: every light back to its ShowBuddy-derived
    /// default position, duplicates and towers removed.
    pub fn reset_layout(&mut self, patch: &Patch, set: &Settings) {
        self.push_undo();
        self.towers.clear();
        self.instances = patch
            .fixtures
            .iter()
            .enumerate()
            .map(|(fi, f)| Instance {
                fixture: fi,
                t: default_transform(f, set),
                opacity: 1.0,
                mount: None,
            })
            .collect();
        self.selection.clear();
        self.sel_tower = None;
        self.save(patch);
    }

    // ---- named setups ----

    fn setup_path(name: &str) -> PathBuf {
        let safe: String = name
            .trim()
            .chars()
            .map(|c| if c.is_alphanumeric() || " -_().".contains(c) { c } else { '_' })
            .collect();
        PathBuf::from(SETUPS_DIR).join(format!("{safe}.json"))
    }

    pub fn list_setups() -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(SETUPS_DIR) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "json") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        out.push(stem.to_string());
                    }
                }
            }
        }
        out.sort();
        out
    }

    /// Save the current arrangement (positions, duplicates, towers) under a
    /// name in the setups folder.
    pub fn save_setup(&self, patch: &Patch, name: &str) -> bool {
        if name.trim().is_empty() {
            return false;
        }
        let _ = std::fs::create_dir_all(SETUPS_DIR);
        serde_json::to_string_pretty(&self.layout_data(patch))
            .ok()
            .and_then(|json| std::fs::write(Self::setup_path(name), json).ok())
            .is_some()
    }

    pub fn load_setup(&mut self, patch: &Patch, set: &Settings, name: &str) -> bool {
        let Ok(text) = std::fs::read_to_string(Self::setup_path(name)) else {
            return false;
        };
        let Ok(lf) = serde_json::from_str::<LayoutFile>(&text) else {
            return false;
        };
        self.apply_layout(patch, set, lf);
        self.save(patch);
        true
    }

    pub fn delete_setup(name: &str) {
        let _ = std::fs::remove_file(Self::setup_path(name));
    }

    // ---- selection helpers (fixture-level, used by the fixture list) ----

    /// Is any instance of patch fixture `fi` selected?
    pub fn fixture_selected(&self, fi: usize) -> bool {
        self.selection
            .iter()
            .any(|&i| self.instances.get(i).is_some_and(|inst| inst.fixture == fi))
    }

    /// Unique patch-fixture indices covered by the current selection.
    pub fn selected_fixtures(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self
            .selection
            .iter()
            .filter_map(|&i| self.instances.get(i).map(|inst| inst.fixture))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Select every instance of patch fixture `fi`.
    pub fn select_fixture(&mut self, fi: usize, additive: bool) {
        if !additive {
            self.selection.clear();
        }
        for (i, inst) in self.instances.iter().enumerate() {
            if inst.fixture == fi {
                self.selection.insert(i);
            }
        }
        self.sel_tower = None;
        self.last_selected = Some(fi);
    }

    /// ⇧-click in the fixture list: toggle all instances of fixture `fi`.
    pub fn toggle_fixture(&mut self, fi: usize) {
        let idxs: Vec<usize> = self
            .instances
            .iter()
            .enumerate()
            .filter(|(_, inst)| inst.fixture == fi)
            .map(|(i, _)| i)
            .collect();
        let all_in = idxs.iter().all(|i| self.selection.contains(i));
        for i in idxs {
            if all_in {
                self.selection.remove(&i);
            } else {
                self.selection.insert(i);
            }
        }
        self.last_selected = Some(fi);
    }

    // ---- duplicates ----

    /// Copy every selected light. Copies follow the same DMX channels —
    /// for fixtures wired to several physical units. New copies become the
    /// selection.
    pub fn duplicate_selection(&mut self, patch: &Patch) {
        let mut sel: Vec<usize> = self.selection.iter().copied().collect();
        sel.sort_unstable();
        if sel.is_empty() {
            return;
        }
        self.push_undo();
        let mut new_sel = HashSet::new();
        for &i in &sel {
            let Some(src) = self.instances.get(i) else { continue };
            let mut inst = src.clone();
            inst.mount = None;
            inst.t.pos = inst.t.pos + v3(0.6, 0.0, 0.0);
            new_sel.insert(self.instances.len());
            self.instances.push(inst);
        }
        if !new_sel.is_empty() {
            self.selection = new_sel;
            self.save(patch);
        }
    }

    /// Remove selected duplicates. The last remaining copy of each fixture
    /// is kept so every patched light stays visible.
    pub fn delete_selection(&mut self, patch: &Patch) {
        let mut count: HashMap<usize, usize> = HashMap::new();
        for inst in &self.instances {
            *count.entry(inst.fixture).or_default() += 1;
        }
        let mut doomed: Vec<usize> = self
            .selection
            .iter()
            .copied()
            .filter(|&i| i < self.instances.len())
            .collect();
        doomed.sort_unstable_by(|a, b| b.cmp(a)); // remove from the back
        if doomed.is_empty() {
            return;
        }
        self.push_undo();
        let mut removed = false;
        for i in doomed {
            let fi = self.instances[i].fixture;
            if let Some(c) = count.get_mut(&fi) {
                if *c > 1 {
                    *c -= 1;
                    self.instances.remove(i);
                    removed = true;
                }
            }
        }
        if removed {
            self.selection.clear();
            self.save(patch);
        }
    }

    // ---- towers ----

    pub fn add_tower(&mut self, patch: &Patch) {
        self.push_undo();
        let mut t = Tower::default();
        t.pos.x = self.towers.len() as f32 * 1.5 - 2.0;
        self.sel_tower = Some(self.towers.len());
        self.towers.push(t);
        self.selection.clear();
        self.save(patch);
    }

    pub fn delete_tower(&mut self, patch: &Patch, ti: usize) {
        if ti >= self.towers.len() {
            return;
        }
        self.push_undo();
        self.towers.remove(ti);
        for inst in &mut self.instances {
            match &mut inst.mount {
                Some((t, _)) if *t == ti => inst.mount = None,
                Some((t, _)) if *t > ti => *t -= 1,
                _ => {}
            }
        }
        self.sel_tower = None;
        self.save(patch);
    }

    /// ⌘-click: select every light of the same type (same .dmx definition).
    /// Inside an existing multi-selection this narrows to that type within the
    /// group; otherwise it adds the whole type from the full patch.
    /// `fi` is a patch fixture index.
    pub fn select_same_type(&mut self, patch: &Patch, fi: usize) {
        let Some(key) = patch.fixtures.get(fi).map(|f| f.file.clone()) else {
            return;
        };
        let in_group = self.selection.len() > 1 && self.fixture_selected(fi);
        let instances = &self.instances;
        let same_file = |j: usize| {
            instances
                .get(j)
                .and_then(|inst| patch.fixtures.get(inst.fixture))
                .is_some_and(|g| g.file == key)
        };
        if in_group {
            self.selection.retain(|&j| same_file(j));
        } else {
            for j in 0..instances.len() {
                if same_file(j) {
                    self.selection.insert(j);
                }
            }
        }
        self.sel_tower = None;
        self.last_selected = Some(fi);
    }
}
