//! Element builders (single and composite), groups, outliner data, rig
//! summary and the setup-folder listing for the Inspector's Build tab.
//!
//! [`plan`] is pure: a [`Recipe`] plus an origin becomes towers and trusses,
//! each already named and tagged with its composite group.
//! [`StageView::build`] is the single entry point that pushes them onto the
//! stage — one undo step, one save, the last part selected. Everything else
//! here reads the rig (the outliner rows, the group names, the rig summary)
//! or edits it under the same undo contract: `push_undo` first, `save` last,
//! and neither for a cosmetic rename or hide.
//!
//! No egui: the outliner's glyph is reported as a [`RecipeKind`] and the
//! Build tab maps it to an `Icon`.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use super::layout::{
    CompositeKind, ElementGroup, ElementRef, LayoutFile, Tower, Truss, TrussKind, TOWER_SLOTS,
};
use super::math::{dir_from_angles, rotate_about, v3, V3};
use super::settings::Settings;
use super::view::StageView;
use super::SETUPS_DIR;
use crate::showbuddy::Patch;

/// Standard F34 straight-run lengths, in metres.
pub(crate) const F34_LENGTHS: [f32; 5] = [0.5, 1.0, 1.5, 2.0, 3.0];
/// Standard curved-run radii, in metres.
pub(crate) const RADIUS_PRESETS: [f32; 4] = [1.0, 1.5, 2.0, 3.0];
/// Standard curved-run sweeps, in degrees.
pub(crate) const ARC_PRESETS: [f32; 5] = [45.0, 90.0, 180.0, 270.0, 360.0];
/// Standard trim heights, in metres.
pub(crate) const HEIGHT_PRESETS: [f32; 5] = [2.5, 3.2, 4.0, 5.0, 6.0];
/// Standard spans between two legs, in metres.
pub(crate) const SPAN_PRESETS: [f32; 4] = [3.0, 4.0, 6.0, 8.0];

/// The eight things the palette can build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum RecipeKind {
    Tower,
    #[default]
    Straight,
    Radius,
    Goalpost,
    Box,
    Ring,
    TowerPair,
    Arch,
}

impl RecipeKind {
    pub(crate) const ALL: [RecipeKind; 8] = [
        Self::Tower,
        Self::Straight,
        Self::Radius,
        Self::Goalpost,
        Self::Box,
        Self::Ring,
        Self::TowerPair,
        Self::Arch,
    ];
    pub(crate) const COUNT: usize = 8;

    /// What the pad and the parameter card are titled.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Tower => "Tower",
            Self::Straight => "F34",
            Self::Radius => "Radius",
            Self::Goalpost => "Goalpost",
            Self::Box => "Box",
            Self::Ring => "Ring",
            Self::TowerPair => "Tower pair",
            Self::Arch => "Arch",
        }
    }

    /// The stem new elements of this kind are numbered from.
    pub(crate) fn name_prefix(self) -> &'static str {
        match self {
            Self::Tower => "Tower",
            Self::Straight => "F34 truss",
            Self::Radius => "Radius truss",
            Self::Goalpost => "Goalpost",
            Self::Box => "Box",
            Self::Ring => "Ring",
            Self::TowerPair => "Tower pair",
            Self::Arch => "Arch",
        }
    }

    /// Stable key for `inspector.json` and id salts.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Tower => "tower",
            Self::Straight => "straight",
            Self::Radius => "radius",
            Self::Goalpost => "goalpost",
            Self::Box => "box",
            Self::Ring => "ring",
            Self::TowerPair => "towerpair",
            Self::Arch => "arch",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<RecipeKind> {
        Self::ALL.into_iter().find(|k| k.key() == key)
    }

    /// One line under the card title: what this kind is made of.
    pub(crate) fn caption(self) -> &'static str {
        match self {
            Self::Tower => "4 slots up, 4 under",
            Self::Straight => "4 faces · a slot every 0.5 m",
            Self::Radius => "top and under faces · a slot every 0.5 m of arc",
            Self::Goalpost => "two towers and a beam",
            Self::Box => "four grounded runs",
            Self::Ring => "one closed radius run",
            Self::TowerPair => "two towers, same heading",
            Self::Arch => "two towers and a half-ring stood on edge",
        }
    }

    /// The pad's tooltip.
    pub(crate) fn hover(self) -> &'static str {
        match self {
            Self::Tower => {
                "Floor stand with one crossbar: four slots on top, four underneath. \
                 Shift-click adds one now; right-click for standard heights."
            }
            Self::Straight => {
                "Straight F34 box-truss run. Lights clip onto any of its four faces. \
                 Shift-click adds one now; right-click for standard lengths."
            }
            Self::Radius => {
                "Curved truss sweeping an arc, with a top and an under face. \
                 Shift-click adds one now; right-click for standard sweeps."
            }
            Self::Goalpost => {
                "Two towers with a straight beam across their tops. \
                 Shift-click adds one now; right-click for standard spans."
            }
            Self::Box => {
                "Four straight runs in a rectangle overhead, legs down at the corners. \
                 Shift-click adds one now; right-click for standard footprints."
            }
            Self::Ring => {
                "One closed circular run hung flat overhead. It is hard to click on the \
                 stage — pick it from the list below instead."
            }
            Self::TowerPair => {
                "Two matching floor stands facing the same way, one each side. \
                 Shift-click adds one now; right-click for standard spacings."
            }
            Self::Arch => {
                "Two towers with a half-ring stood on edge between them. The arch is hard \
                 to click on the stage — pick it from the list below instead."
            }
        }
    }

    /// The composite this kind tags its parts with; `None` for the three
    /// single elements.
    pub(crate) fn composite(self) -> Option<CompositeKind> {
        match self {
            Self::Goalpost => Some(CompositeKind::Goalpost),
            Self::Box => Some(CompositeKind::Box),
            Self::Ring => Some(CompositeKind::Ring),
            Self::TowerPair => Some(CompositeKind::TowerPair),
            Self::Arch => Some(CompositeKind::Arch),
            _ => None,
        }
    }
}

impl CompositeKind {
    /// What a group with no named members falls back to.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Goalpost => "Goalpost",
            Self::Box => "Box",
            Self::Ring => "Ring",
            Self::TowerPair => "Tower pair",
            Self::Arch => "Arch",
        }
    }

    /// The glyph a one-part group (a ring) wears in the outliner.
    pub(crate) fn recipe_kind(self) -> RecipeKind {
        match self {
            Self::Goalpost => RecipeKind::Goalpost,
            Self::Box => RecipeKind::Box,
            Self::Ring => RecipeKind::Ring,
            Self::TowerPair => RecipeKind::TowerPair,
            Self::Arch => RecipeKind::Arch,
        }
    }
}

/// One kind's parameters. The Build tab keeps one of each, so switching
/// pads never loses what was typed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Recipe {
    Tower { height: f32, width: f32, yaw: f32 },
    Straight { length: f32, height: f32, yaw: f32, grounded: bool },
    Radius { radius: f32, arc: f32, height: f32, yaw: f32, grounded: bool },
    Goalpost { span: f32, height: f32, yaw: f32 },
    Box { width: f32, depth: f32, height: f32, yaw: f32, grounded: bool },
    Ring { radius: f32, height: f32 },
    TowerPair { spacing: f32, height: f32, width: f32, yaw: f32 },
    Arch { span: f32, rise: f32, yaw: f32 },
}

impl Recipe {
    pub(crate) fn default_for(kind: RecipeKind) -> Recipe {
        match kind {
            RecipeKind::Tower => Recipe::Tower { height: 3.2, width: 2.4, yaw: 0.0 },
            RecipeKind::Straight => {
                Recipe::Straight { length: 3.0, height: 3.2, yaw: 0.0, grounded: true }
            }
            RecipeKind::Radius => Recipe::Radius {
                radius: 2.0,
                arc: 90.0,
                height: 3.2,
                yaw: 0.0,
                grounded: false,
            },
            RecipeKind::Goalpost => Recipe::Goalpost { span: 4.0, height: 3.5, yaw: 0.0 },
            RecipeKind::Box => {
                Recipe::Box { width: 6.0, depth: 4.0, height: 4.0, yaw: 0.0, grounded: true }
            }
            RecipeKind::Ring => Recipe::Ring { radius: 2.0, height: 4.0 },
            RecipeKind::TowerPair => {
                Recipe::TowerPair { spacing: 4.0, height: 3.2, width: 2.4, yaw: 0.0 }
            }
            RecipeKind::Arch => Recipe::Arch { span: 4.0, rise: 3.2, yaw: 0.0 },
        }
    }

    pub(crate) fn kind(self) -> RecipeKind {
        match self {
            Recipe::Tower { .. } => RecipeKind::Tower,
            Recipe::Straight { .. } => RecipeKind::Straight,
            Recipe::Radius { .. } => RecipeKind::Radius,
            Recipe::Goalpost { .. } => RecipeKind::Goalpost,
            Recipe::Box { .. } => RecipeKind::Box,
            Recipe::Ring { .. } => RecipeKind::Ring,
            Recipe::TowerPair { .. } => RecipeKind::TowerPair,
            Recipe::Arch { .. } => RecipeKind::Arch,
        }
    }

    /// Every dimension inside the range its DragValue allows, headings
    /// wrapped into 0..360.
    pub(crate) fn clamped(self) -> Recipe {
        let turn = |v: f32| v.rem_euclid(360.0);
        match self {
            Recipe::Tower { height, width, yaw } => Recipe::Tower {
                height: height.clamp(1.2, 6.0),
                width: width.clamp(0.8, 4.0),
                yaw: turn(yaw),
            },
            Recipe::Straight { length, height, yaw, grounded } => Recipe::Straight {
                length: length.clamp(0.5, 6.0),
                height: height.clamp(0.5, 8.0),
                yaw: turn(yaw),
                grounded,
            },
            Recipe::Radius { radius, arc, height, yaw, grounded } => Recipe::Radius {
                radius: radius.clamp(0.5, 8.0),
                arc: arc.clamp(15.0, 360.0),
                height: height.clamp(0.5, 8.0),
                yaw: turn(yaw),
                grounded,
            },
            Recipe::Goalpost { span, height, yaw } => Recipe::Goalpost {
                span: span.clamp(2.0, 10.0),
                height: height.clamp(1.2, 6.0),
                yaw: turn(yaw),
            },
            Recipe::Box { width, depth, height, yaw, grounded } => Recipe::Box {
                width: width.clamp(2.0, 12.0),
                depth: depth.clamp(2.0, 12.0),
                height: height.clamp(0.5, 8.0),
                yaw: turn(yaw),
                grounded,
            },
            Recipe::Ring { radius, height } => {
                Recipe::Ring { radius: radius.clamp(0.5, 8.0), height: height.clamp(0.5, 8.0) }
            }
            Recipe::TowerPair { spacing, height, width, yaw } => Recipe::TowerPair {
                spacing: spacing.clamp(1.0, 10.0),
                height: height.clamp(1.2, 6.0),
                width: width.clamp(0.8, 4.0),
                yaw: turn(yaw),
            },
            Recipe::Arch { span, rise, yaw } => Recipe::Arch {
                span: span.clamp(2.0, 10.0),
                rise: rise.clamp(1.2, 6.0),
                yaw: turn(yaw),
            },
        }
    }

    /// The right-click menu for this kind's pad: (menu label, this recipe
    /// resized). Choosing one also becomes the remembered size.
    pub(crate) fn variants(self) -> Vec<(String, Recipe)> {
        match self {
            Recipe::Tower { width, yaw, .. } => HEIGHT_PRESETS
                .iter()
                .map(|&h| (format!("{h} m tall"), Recipe::Tower { height: h, width, yaw }))
                .collect(),
            Recipe::Straight { height, yaw, grounded, .. } => F34_LENGTHS
                .iter()
                .map(|&l| {
                    (format!("{l} m"), Recipe::Straight { length: l, height, yaw, grounded })
                })
                .collect(),
            Recipe::Radius { radius, height, yaw, grounded, .. } => ARC_PRESETS
                .iter()
                .map(|&a| {
                    (
                        format!("{a:.0}° at r {radius:.1} m"),
                        Recipe::Radius { radius, arc: a, height, yaw, grounded },
                    )
                })
                .collect(),
            Recipe::Goalpost { height, yaw, .. } => SPAN_PRESETS
                .iter()
                .map(|&s| (format!("span {s} m"), Recipe::Goalpost { span: s, height, yaw }))
                .collect(),
            Recipe::Box { height, yaw, grounded, .. } => {
                [(3.0, 3.0), (4.0, 3.0), (6.0, 4.0), (8.0, 6.0)]
                    .iter()
                    .map(|&(w, d)| {
                        (
                            format!("{w:.0} × {d:.0} m"),
                            Recipe::Box { width: w, depth: d, height, yaw, grounded },
                        )
                    })
                    .collect()
            }
            Recipe::Ring { height, .. } => RADIUS_PRESETS
                .iter()
                .map(|&r| (format!("r {r} m"), Recipe::Ring { radius: r, height }))
                .collect(),
            Recipe::TowerPair { height, width, yaw, .. } => SPAN_PRESETS
                .iter()
                .map(|&s| {
                    (
                        format!("{s} m apart"),
                        Recipe::TowerPair { spacing: s, height, width, yaw },
                    )
                })
                .collect(),
            Recipe::Arch { rise, yaw, .. } => [3.0f32, 4.0, 6.0]
                .iter()
                .map(|&s| (format!("span {s} m"), Recipe::Arch { span: s, rise, yaw }))
                .collect(),
        }
    }
}

/// Where a build lands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Placement {
    /// The next slot in the row the old Add buttons used.
    Stagger,
    /// This spot on the floor, nudged sideways until it is clear.
    At(V3),
}

/// What one [`StageView::build`] produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct BuildResult {
    pub added: Vec<ElementRef>,
    pub group: Option<u32>,
    /// The log line, e.g. "Goalpost 1 (2 towers + 1 truss)".
    pub summary: String,
}

/// One outliner row, read once per frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ElementInfo {
    pub r: ElementRef,
    /// What the row is titled (the operator name, else the default).
    pub label: String,
    /// The raw `name` field, so a rename starts from what was typed.
    pub name: String,
    /// "F34 3.0 m · 24 slots".
    pub caption: String,
    /// Which glyph the row wears; the Build tab maps it to an `Icon`.
    pub glyph: RecipeKind,
    pub group: Option<ElementGroup>,
    pub mounted: usize,
    pub slots: usize,
    pub hidden: bool,
}

/// What the rig adds up to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RigSummary {
    pub towers: usize,
    pub trusses: usize,
    pub groups: usize,
    pub slots_total: usize,
    pub slots_used: usize,
    pub lights_loose: usize,
}

/// What one file in the setups folder holds, without opening it again.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SetupInfo {
    pub name: String,
    pub towers: usize,
    pub trusses: usize,
    pub lights: usize,
    pub modified: Option<std::time::SystemTime>,
}

/// Turn a recipe into its parts. Pure: every part is placed relative to
/// `origin`, named `"{Kind seq}"` (single) or `"{Kind seq} · {role}"`
/// (composite) and tagged with `group`.
pub(crate) fn plan(
    recipe: Recipe,
    origin: V3,
    group: Option<ElementGroup>,
    seq: u32,
) -> (Vec<Tower>, Vec<Truss>) {
    let kind = recipe.kind();
    let base = format!("{} {}", kind.name_prefix(), seq);
    let part = |role: &str| format!("{base} · {role}");
    let o = v3(origin.x, 0.0, origin.z);
    let mut towers: Vec<Tower> = Vec::new();
    let mut trusses: Vec<Truss> = Vec::new();
    // A tower standing at `at`, facing `yaw`.
    let stand = |at: V3, yaw: f32, height: f32, width: f32, name: String| Tower {
        pos: at,
        yaw_deg: yaw,
        height,
        width,
        name,
        group,
        hidden: false,
    };
    match recipe {
        Recipe::Tower { height, width, yaw } => {
            towers.push(stand(o, yaw, height, width, base.clone()));
        }
        Recipe::Straight { length, height, yaw, grounded } => {
            let mut t = Truss::straight();
            t.pos = o + v3(0.0, height, 0.0);
            t.yaw_deg = yaw;
            t.length = length;
            t.grounded = grounded;
            t.name = base.clone();
            t.group = group;
            trusses.push(t);
        }
        Recipe::Radius { radius, arc, height, yaw, grounded } => {
            let mut t = Truss::radius();
            t.pos = o + v3(0.0, height, 0.0);
            t.yaw_deg = yaw;
            t.radius = radius;
            t.arc_deg = arc;
            t.grounded = grounded;
            t.name = base.clone();
            t.group = group;
            trusses.push(t);
        }
        Recipe::Goalpost { span, height, yaw } => {
            let side = dir_from_angles(yaw + 90.0, 0.0);
            towers.push(stand(o - side * (span * 0.5), yaw, height, 1.2, part("left leg")));
            towers.push(stand(o + side * (span * 0.5), yaw, height, 1.2, part("right leg")));
            let mut beam = Truss::straight();
            beam.pos = o + v3(0.0, height, 0.0);
            beam.yaw_deg = yaw + 90.0;
            beam.length = span;
            beam.grounded = false;
            beam.name = part("beam");
            beam.group = group;
            trusses.push(beam);
        }
        Recipe::Box { width, depth, height, yaw, grounded } => {
            let side = dir_from_angles(yaw + 90.0, 0.0);
            let fwd = dir_from_angles(yaw, 0.0);
            let run = |at: V3, heading: f32, length: f32, name: String| {
                let mut t = Truss::straight();
                t.pos = at + v3(0.0, height, 0.0);
                t.yaw_deg = heading;
                t.length = length;
                t.grounded = grounded;
                t.name = name;
                t.group = group;
                t
            };
            trusses.push(run(o + fwd * (depth * 0.5), yaw + 90.0, width, part("front")));
            trusses.push(run(o - fwd * (depth * 0.5), yaw + 90.0, width, part("back")));
            trusses.push(run(o - side * (width * 0.5), yaw, depth, part("left")));
            trusses.push(run(o + side * (width * 0.5), yaw, depth, part("right")));
        }
        Recipe::Ring { radius, height } => {
            let mut t = Truss::radius();
            t.pos = o + v3(0.0, height, 0.0);
            t.yaw_deg = 0.0;
            t.radius = radius;
            t.arc_deg = 360.0;
            t.grounded = false;
            t.name = base.clone();
            t.group = group;
            trusses.push(t);
        }
        Recipe::TowerPair { spacing, height, width, yaw } => {
            let side = dir_from_angles(yaw + 90.0, 0.0);
            towers.push(stand(o - side * (spacing * 0.5), yaw, height, width, part("left")));
            towers.push(stand(o + side * (spacing * 0.5), yaw, height, width, part("right")));
        }
        Recipe::Arch { span, rise, yaw } => {
            let side = dir_from_angles(yaw + 90.0, 0.0);
            towers.push(stand(o - side * (span * 0.5), yaw, rise, 1.2, part("left leg")));
            towers.push(stand(o + side * (span * 0.5), yaw, rise, 1.2, part("right leg")));
            // A half ring stood on edge: pitch 90 tips the run's plane
            // upright and roll = heading turns that plane to face the
            // towers, so the arc's ends land exactly on their tops.
            let mut t = Truss::radius();
            t.pos = o + v3(0.0, rise, 0.0);
            t.yaw_deg = 90.0;
            t.pitch_deg = 90.0;
            t.roll_deg = yaw;
            t.radius = span * 0.5;
            t.arc_deg = 180.0;
            t.grounded = false;
            t.name = part("arch");
            t.group = group;
            trusses.push(t);
        }
    }
    (towers, trusses)
}

/// Every `*.json` in `dir` described without opening it again. Files that
/// do not parse are still listed, with zero counts.
fn list_setup_infos(dir: &Path) -> Vec<SetupInfo> {
    let mut out: Vec<SetupInfo> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Some(name) = p.file_stem().and_then(|s| s.to_str()).map(|s| s.to_owned()) else {
            continue;
        };
        let modified = e.metadata().ok().and_then(|m| m.modified().ok());
        let lf = std::fs::read_to_string(&p)
            .ok()
            .and_then(|t| serde_json::from_str::<LayoutFile>(&t).ok());
        let (towers, trusses, lights) = match lf {
            Some(lf) => (lf.towers.len(), lf.trusses.len(), lf.instances.len()),
            None => (0, 0, 0),
        };
        out.push(SetupInfo { name, towers, trusses, lights, modified });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A tower reflected across the stage's centre line (x = 0).
fn mirror_tower(t: &Tower) -> Tower {
    let mut m = t.clone();
    m.pos = v3(-t.pos.x, t.pos.y, t.pos.z);
    m.yaw_deg = (-t.yaw_deg).rem_euclid(360.0);
    m.group = None;
    m
}

/// A truss reflected across x = 0, through the same pose maths the Selection
/// tab's mirror uses — reflection maps an angle `a` to `−a`, so an arc
/// `[yaw, yaw + arc]` becomes `[−(yaw + arc), −yaw]` and a pitched run leans
/// the other way.
fn mirror_truss(t: &Truss) -> Truss {
    let mut m = t.clone();
    m.pos = v3(-t.pos.x, t.pos.y, t.pos.z);
    let (yaw, pitch, roll) = super::arrange::mirror_truss_pose(
        t.kind,
        t.yaw_deg,
        t.arc_deg,
        t.pitch_deg,
        t.roll_deg,
        super::arrange::MirrorPlane::X,
    );
    m.yaw_deg = yaw.rem_euclid(360.0);
    m.pitch_deg = pitch;
    m.roll_deg = roll.rem_euclid(360.0);
    m.group = None;
    m
}

/// The name with one trailing number stripped ("F34 truss 3" → "F34 truss").
fn base_name(name: &str) -> &str {
    let t = name.trim_end();
    let stripped = t.trim_end_matches(|c: char| c.is_ascii_digit());
    if stripped.len() < t.len() && stripped.ends_with(' ') {
        stripped.trim_end()
    } else {
        t
    }
}

impl StageView {
    // ---- naming ----

    /// The element's operator name, else "Tower N" / "F34 truss N" /
    /// "Radius truss N" (1-based).
    pub(crate) fn element_name(&self, r: ElementRef) -> String {
        match r {
            ElementRef::Tower(i) => match self.towers.get(i) {
                Some(tw) if !tw.name.trim().is_empty() => tw.name.trim().to_owned(),
                _ => format!("Tower {}", i + 1),
            },
            ElementRef::Truss(i) => match self.trusses.get(i) {
                Some(tr) if !tr.name.trim().is_empty() => tr.name.trim().to_owned(),
                Some(tr) => match tr.kind {
                    TrussKind::Straight => format!("F34 truss {}", i + 1),
                    TrussKind::Radius => format!("Radius truss {}", i + 1),
                },
                None => format!("F34 truss {}", i + 1),
            },
        }
    }

    /// Every name in use: the elements' display names and the group names.
    fn names_in_use(&self) -> HashSet<String> {
        let mut taken: HashSet<String> = HashSet::new();
        for i in 0..self.towers.len() {
            taken.insert(self.element_name(ElementRef::Tower(i)));
        }
        for i in 0..self.trusses.len() {
            taken.insert(self.element_name(ElementRef::Truss(i)));
        }
        for gid in self.group_ids() {
            taken.insert(self.group_name(gid));
        }
        taken
    }

    /// `"{prefix} N"` for the smallest free N ≥ 1.
    pub(crate) fn next_element_name(&self, prefix: &str) -> String {
        let taken = self.names_in_use();
        for n in 1..=9999u32 {
            let candidate = format!("{prefix} {n}");
            if !taken.contains(&candidate) {
                return candidate;
            }
        }
        format!("{prefix} 1")
    }

    // ---- building ----

    /// The first point at or beside `base` (stepping ±`step` along X) with
    /// no tower or truss within 0.8 m in plan.
    pub(crate) fn free_spot(&self, base: V3, step: f32) -> V3 {
        let clear = |p: V3| {
            let near = |q: V3| {
                let (dx, dz) = (q.x - p.x, q.z - p.z);
                dx * dx + dz * dz < 0.8 * 0.8
            };
            !self.towers.iter().any(|t| near(t.pos)) && !self.trusses.iter().any(|t| near(t.pos))
        };
        if clear(base) {
            return base;
        }
        for k in 1..=8 {
            for sign in [1.0f32, -1.0] {
                let p = base + v3(step * k as f32 * sign, 0.0, 0.0);
                if clear(p) {
                    return p;
                }
            }
        }
        base
    }

    /// One past the highest group id in use; 1 when there are none.
    pub(crate) fn next_group_id(&self) -> u32 {
        let highest = self
            .towers
            .iter()
            .filter_map(|t| t.group)
            .chain(self.trusses.iter().filter_map(|t| t.group))
            .map(|g| g.id)
            .max()
            .unwrap_or(0);
        highest + 1
    }

    /// Add one recipe to the stage: one undo step, the parts pushed, the
    /// last part selected, one save.
    pub(crate) fn build(&mut self, patch: &Patch, recipe: Recipe, at: Placement) -> BuildResult {
        let recipe = recipe.clamped();
        let kind = recipe.kind();
        let origin = match at {
            Placement::Stagger => {
                let n = (self.towers.len() + self.trusses.len()) as f32;
                let z = match kind {
                    RecipeKind::Tower => 4.0,
                    RecipeKind::Straight | RecipeKind::Radius => -4.0,
                    _ => 0.0,
                };
                v3(n * 1.5 - 2.0, 0.0, z)
            }
            Placement::At(p) => self.free_spot(v3(p.x, 0.0, p.z), 1.5),
        };
        let name = self.next_element_name(kind.name_prefix());
        let seq = name
            .rsplit(' ')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1);
        let group = kind
            .composite()
            .map(|ck| ElementGroup { id: self.next_group_id(), kind: ck });
        let (tws, trs) = plan(recipe, origin, group, seq);
        let (n_tw, n_tr) = (tws.len(), trs.len());
        self.push_undo();
        let mut added: Vec<ElementRef> = Vec::new();
        for tw in tws {
            added.push(ElementRef::Tower(self.towers.len()));
            self.towers.push(tw);
        }
        for tr in trs {
            added.push(ElementRef::Truss(self.trusses.len()));
            self.trusses.push(tr);
        }
        self.selection.clear();
        self.sel_stage = false;
        self.hover_element = None;
        if n_tr > 0 {
            self.sel_truss = Some(self.trusses.len() - 1);
            self.sel_tower = None;
        } else {
            self.sel_tower = Some(self.towers.len().saturating_sub(1));
            self.sel_truss = None;
        }
        let summary = match recipe {
            Recipe::Tower { height, .. } => format!("{name} ({height:.1} m)"),
            Recipe::Straight { length, .. } => format!("F34 truss {length:.1} m ({name})"),
            Recipe::Radius { radius, arc, .. } => {
                format!("Radius truss r {radius:.1} m {arc:.0}° ({name})")
            }
            Recipe::Box { .. } => format!("{name} ({n_tr} trusses)"),
            Recipe::Ring { .. } | Recipe::TowerPair { .. } => name.clone(),
            Recipe::Goalpost { .. } | Recipe::Arch { .. } => {
                format!("{name} ({n_tw} towers + {n_tr} truss)")
            }
        };
        self.save(patch);
        BuildResult { added, group: group.map(|g| g.id), summary }
    }

    // ---- reading the rig ----

    fn element_group(&self, r: ElementRef) -> Option<ElementGroup> {
        match r {
            ElementRef::Tower(i) => self.towers.get(i).and_then(|t| t.group),
            ElementRef::Truss(i) => self.trusses.get(i).and_then(|t| t.group),
        }
    }

    /// The row's second line: what it measures and how full it is. Short
    /// enough to read whole in a 230 px panel — the row's glyph already
    /// says which kind it is.
    pub(crate) fn element_caption(&self, r: ElementRef) -> String {
        let (mounted, total) = self.element_slots(r);
        let fill = format!("{mounted}/{total}");
        match r {
            ElementRef::Tower(i) => match self.towers.get(i) {
                Some(tw) => format!("{:.1} m · {fill}", tw.height),
                None => String::new(),
            },
            ElementRef::Truss(i) => match self.trusses.get(i) {
                Some(tr) => match tr.kind {
                    TrussKind::Straight => format!("{:.1} m · {fill}", tr.length),
                    TrussKind::Radius if tr.pitch_deg.abs() > 0.5 => {
                        format!("arch r {:.1} m · {fill}", tr.radius)
                    }
                    TrussKind::Radius => {
                        format!("r {:.1} m {:.0}° · {fill}", tr.radius, tr.arc_deg)
                    }
                },
                None => String::new(),
            },
        }
    }

    fn element_glyph(&self, r: ElementRef) -> RecipeKind {
        match r {
            ElementRef::Tower(_) => RecipeKind::Tower,
            ElementRef::Truss(i) => match self.trusses.get(i) {
                Some(tr) if tr.kind == TrussKind::Radius => {
                    if tr.pitch_deg.abs() > 0.5 && tr.arc_deg.abs() <= 180.5 {
                        RecipeKind::Arch
                    } else {
                        RecipeKind::Radius
                    }
                }
                _ => RecipeKind::Straight,
            },
        }
    }

    /// (hung, total) slots on this element.
    pub(crate) fn element_slots(&self, r: ElementRef) -> (usize, usize) {
        let total = match r {
            ElementRef::Tower(i) => self.towers.get(i).map(|_| TOWER_SLOTS).unwrap_or(0),
            ElementRef::Truss(i) => self.trusses.get(i).map(|t| t.total_slots()).unwrap_or(0),
        };
        (self.mounted_on(r).len(), total)
    }

    /// Tower: mid-height above its base. Truss: its `pos`. None when out of
    /// range.
    pub(crate) fn element_centre(&self, r: ElementRef) -> Option<V3> {
        match r {
            ElementRef::Tower(i) => {
                self.towers.get(i).map(|tw| tw.pos + v3(0.0, tw.height * 0.5, 0.0))
            }
            ElementRef::Truss(i) => self.trusses.get(i).map(|tr| tr.pos),
        }
    }

    /// Which element the stage has picked, if any.
    pub(crate) fn selected_element(&self) -> Option<ElementRef> {
        self.sel_tower
            .filter(|&i| i < self.towers.len())
            .map(ElementRef::Tower)
            .or(self.sel_truss.filter(|&i| i < self.trusses.len()).map(ElementRef::Truss))
    }

    /// (instance index, slot) for every light hung on `r`, in slot order.
    pub(crate) fn mounted_on(&self, r: ElementRef) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = self
            .instances
            .iter()
            .enumerate()
            .filter_map(|(k, inst)| match r {
                ElementRef::Tower(i) => inst.mount.filter(|(t, _)| *t == i).map(|(_, s)| (k, s)),
                ElementRef::Truss(i) => {
                    inst.truss_mount.filter(|(t, _)| *t == i).map(|(_, s)| (k, s))
                }
            })
            .collect();
        out.sort_by_key(|&(_, s)| s);
        out
    }

    /// Which way a light in `slot` points.
    pub(crate) fn mount_face_label(&self, r: ElementRef, slot: usize) -> &'static str {
        match r {
            ElementRef::Tower(_) => {
                if Tower::slot_points_up(slot) {
                    "up"
                } else {
                    "under"
                }
            }
            ElementRef::Truss(i) => {
                let Some(tr) = self.trusses.get(i) else { return "top" };
                match slot / tr.slot_count().max(1) {
                    0 => "top",
                    1 => {
                        if tr.kind == TrussKind::Radius {
                            "under"
                        } else {
                            "bottom"
                        }
                    }
                    2 => "left",
                    _ => "right",
                }
            }
        }
    }

    fn element_info(&self, r: ElementRef) -> ElementInfo {
        let (mounted, slots) = self.element_slots(r);
        let (name, hidden) = match r {
            ElementRef::Tower(i) => match self.towers.get(i) {
                Some(t) => (t.name.clone(), t.hidden),
                None => (String::new(), false),
            },
            ElementRef::Truss(i) => match self.trusses.get(i) {
                Some(t) => (t.name.clone(), t.hidden),
                None => (String::new(), false),
            },
        };
        ElementInfo {
            r,
            label: self.element_name(r),
            name,
            caption: self.element_caption(r),
            glyph: self.element_glyph(r),
            group: self.element_group(r),
            mounted,
            slots,
            hidden,
        }
    }

    /// Every element as an outliner row: each group's parts together (by
    /// ascending group id), then the standalone towers, then the standalone
    /// trusses.
    pub(crate) fn elements(&self) -> Vec<ElementInfo> {
        let mut out: Vec<ElementInfo> = Vec::new();
        for gid in self.group_ids() {
            let members = self.group_members(gid);
            let lone = members.len() == 1;
            for r in members {
                let mut info = self.element_info(r);
                // A one-part group (a ring) has no header, so its row wears
                // the composite's own glyph.
                if lone {
                    if let Some(g) = info.group {
                        info.glyph = g.kind.recipe_kind();
                    }
                }
                out.push(info);
            }
        }
        for i in 0..self.towers.len() {
            if self.towers[i].group.is_none() {
                out.push(self.element_info(ElementRef::Tower(i)));
            }
        }
        for i in 0..self.trusses.len() {
            if self.trusses[i].group.is_none() {
                out.push(self.element_info(ElementRef::Truss(i)));
            }
        }
        out
    }

    /// Group ids in use, ascending.
    fn group_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self
            .towers
            .iter()
            .filter_map(|t| t.group.map(|g| g.id))
            .chain(self.trusses.iter().filter_map(|t| t.group.map(|g| g.id)))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// The group's parts: towers first, then trusses, in index order.
    pub(crate) fn group_members(&self, gid: u32) -> Vec<ElementRef> {
        let mut out: Vec<ElementRef> = self
            .towers
            .iter()
            .enumerate()
            .filter(|(_, t)| t.group.is_some_and(|g| g.id == gid))
            .map(|(i, _)| ElementRef::Tower(i))
            .collect();
        out.extend(
            self.trusses
                .iter()
                .enumerate()
                .filter(|(_, t)| t.group.is_some_and(|g| g.id == gid))
                .map(|(i, _)| ElementRef::Truss(i)),
        );
        out
    }

    fn group_kind(&self, gid: u32) -> Option<CompositeKind> {
        self.group_members(gid).first().and_then(|&r| self.element_group(r)).map(|g| g.kind)
    }

    /// The name the group's parts share, from the first member carrying a
    /// `" · role"` suffix; else the composite's own label and id.
    pub(crate) fn group_name(&self, gid: u32) -> String {
        for r in self.group_members(gid) {
            let name = self.element_name(r);
            if let Some((head, _)) = name.split_once(" · ") {
                if !head.trim().is_empty() {
                    return head.trim().to_owned();
                }
            }
        }
        match self.group_kind(gid) {
            Some(k) => format!("{} {gid}", k.label()),
            None => format!("Group {gid}"),
        }
    }

    /// The numbers the Elements header and the footer show.
    pub(crate) fn rig_summary(&self) -> RigSummary {
        let slots_total = self.towers.len() * TOWER_SLOTS
            + self.trusses.iter().map(|t| t.total_slots()).sum::<usize>();
        let slots_used = self
            .instances
            .iter()
            .filter(|i| i.mount.is_some() || i.truss_mount.is_some())
            .count();
        RigSummary {
            towers: self.towers.len(),
            trusses: self.trusses.len(),
            groups: self.group_ids().len(),
            slots_total,
            slots_used,
            lights_loose: self.instances.len() - slots_used,
        }
    }

    // ---- select and cosmetic edits (save, no undo step) ----

    /// Pick one element: clears lights, the stage box, the hover highlight
    /// and the other kind.
    pub(crate) fn select_element(&mut self, r: ElementRef) {
        self.selection.clear();
        self.sel_stage = false;
        self.hover_element = None;
        match r {
            ElementRef::Tower(i) => {
                self.sel_tower = Some(i);
                self.sel_truss = None;
            }
            ElementRef::Truss(i) => {
                self.sel_truss = Some(i);
                self.sel_tower = None;
            }
        }
    }

    /// Select every light hung on `r`; returns how many. Lights win, so the
    /// picked element and the stage box are cleared. Nothing hung → 0,
    /// untouched.
    pub(crate) fn select_mounted(&mut self, r: ElementRef) -> usize {
        let hung: Vec<usize> = self.mounted_on(r).into_iter().map(|(k, _)| k).collect();
        if hung.is_empty() {
            return 0;
        }
        self.selection.clear();
        self.sel_tower = None;
        self.sel_truss = None;
        self.sel_stage = false;
        self.last_selected = hung.first().map(|&k| self.instances[k].fixture);
        let n = hung.len();
        self.selection.extend(hung);
        n
    }

    /// Hide or show one element. Cosmetic, so it saves without an undo step
    /// — but it lives in the layout, so Ctrl+Shift+Z still sees it.
    pub(crate) fn set_hidden(&mut self, patch: &Patch, r: ElementRef, hidden: bool) {
        let changed = match r {
            ElementRef::Tower(i) => match self.towers.get_mut(i) {
                Some(t) => std::mem::replace(&mut t.hidden, hidden) != hidden,
                None => false,
            },
            ElementRef::Truss(i) => match self.trusses.get_mut(i) {
                Some(t) => std::mem::replace(&mut t.hidden, hidden) != hidden,
                None => false,
            },
        };
        if changed {
            self.hover_element = None;
            self.save(patch);
        }
    }

    pub(crate) fn set_group_hidden(&mut self, patch: &Patch, gid: u32, hidden: bool) {
        for r in self.group_members(gid) {
            match r {
                ElementRef::Tower(i) => {
                    if let Some(t) = self.towers.get_mut(i) {
                        t.hidden = hidden;
                    }
                }
                ElementRef::Truss(i) => {
                    if let Some(t) = self.trusses.get_mut(i) {
                        t.hidden = hidden;
                    }
                }
            }
        }
        self.hover_element = None;
        self.save(patch);
    }

    /// Rename one element; an empty name goes back to the default.
    pub(crate) fn rename_element(&mut self, patch: &Patch, r: ElementRef, name: &str) {
        let name: String = name.trim().chars().take(40).collect();
        match r {
            ElementRef::Tower(i) => match self.towers.get_mut(i) {
                Some(t) => t.name = name,
                None => return,
            },
            ElementRef::Truss(i) => match self.trusses.get_mut(i) {
                Some(t) => t.name = name,
                None => return,
            },
        }
        self.save(patch);
    }

    /// Rename a whole group by rewriting each member's `"{group} · {role}"`.
    pub(crate) fn rename_group(&mut self, patch: &Patch, gid: u32, name: &str) {
        let name: String = name.trim().chars().take(40).collect();
        if name.is_empty() {
            return;
        }
        for r in self.group_members(gid) {
            let old = self.element_name(r);
            let fresh = match old.split_once(" · ") {
                Some((_, role)) => format!("{name} · {role}"),
                None => name.clone(),
            };
            match r {
                ElementRef::Tower(i) => {
                    if let Some(t) = self.towers.get_mut(i) {
                        t.name = fresh;
                    }
                }
                ElementRef::Truss(i) => {
                    if let Some(t) = self.trusses.get_mut(i) {
                        t.name = fresh;
                    }
                }
            }
        }
        self.save(patch);
    }

    // ---- edits (push_undo first, save last) ----

    /// Delete one element; returns how many lights it was carrying.
    pub(crate) fn delete_element(&mut self, patch: &Patch, r: ElementRef) -> usize {
        let carried = self.mounted_on(r).len();
        match r {
            ElementRef::Tower(i) => self.delete_tower(patch, i),
            ElementRef::Truss(i) => self.delete_truss(patch, i),
        }
        self.hover_element = None;
        carried
    }

    /// Copy one element beside itself, standalone and unhung.
    pub(crate) fn duplicate_element(
        &mut self,
        patch: &Patch,
        r: ElementRef,
    ) -> Option<ElementRef> {
        let stem = base_name(&self.element_name(r)).to_owned();
        let name = self.next_element_name(&stem);
        self.push_undo();
        let made = match r {
            ElementRef::Tower(i) => {
                let mut t = self.towers.get(i)?.clone();
                t.pos = self.free_spot(t.pos + v3(1.5, 0.0, 0.0), 1.5);
                t.group = None;
                t.name = name;
                self.towers.push(t);
                ElementRef::Tower(self.towers.len() - 1)
            }
            ElementRef::Truss(i) => {
                let mut t = self.trusses.get(i)?.clone();
                let spot = self.free_spot(v3(t.pos.x + 1.5, 0.0, t.pos.z), 1.5);
                t.pos = v3(spot.x, t.pos.y, spot.z);
                t.group = None;
                t.name = name;
                self.trusses.push(t);
                ElementRef::Truss(self.trusses.len() - 1)
            }
        };
        self.select_element(made);
        self.save(patch);
        Some(made)
    }

    /// Reflect a copy of one element across the stage's centre line.
    pub(crate) fn mirror_element_copy(&mut self, patch: &Patch, r: ElementRef) -> Option<ElementRef> {
        let name = format!("{} (mirror)", self.element_name(r));
        self.push_undo();
        let made = match r {
            ElementRef::Tower(i) => {
                let mut t = mirror_tower(self.towers.get(i)?);
                t.name = name;
                self.towers.push(t);
                ElementRef::Tower(self.towers.len() - 1)
            }
            ElementRef::Truss(i) => {
                let mut t = mirror_truss(self.trusses.get(i)?);
                t.name = name;
                self.trusses.push(t);
                ElementRef::Truss(self.trusses.len() - 1)
            }
        };
        self.select_element(made);
        self.save(patch);
        Some(made)
    }

    /// Step one element up (`-1`) or down (`+1`) the list the outliner
    /// draws, taking its hung lights and its group tag with it. A no-op at
    /// the ends, and wherever the neighbouring row is of the other kind or
    /// belongs to another group: `elements()` buckets its rows by group tag
    /// first, so swapping across a bucket boundary would rewrite the layout
    /// and spend an undo step without moving a single row.
    pub(crate) fn move_element(&mut self, patch: &Patch, r: ElementRef, dir: i32) -> ElementRef {
        let rows: Vec<(ElementRef, Option<u32>)> =
            self.elements().iter().map(|e| (e.r, e.group.map(|g| g.id))).collect();
        let Some(pos) = rows.iter().position(|&(x, _)| x == r) else { return r };
        let k = pos as i32 + dir;
        if k < 0 || k as usize >= rows.len() {
            return r;
        }
        let (next, next_group) = rows[k as usize];
        if next_group != rows[pos].1 {
            return r;
        }
        let (i, j) = match (r, next) {
            (ElementRef::Tower(i), ElementRef::Tower(j)) => (i, j),
            (ElementRef::Truss(i), ElementRef::Truss(j)) => (i, j),
            _ => return r,
        };
        self.push_undo();
        let swap = |m: &mut Option<(usize, usize)>| {
            if let Some((t, _)) = m {
                if *t == i {
                    *t = j;
                } else if *t == j {
                    *t = i;
                }
            }
        };
        let out = match r {
            ElementRef::Tower(_) => {
                self.towers.swap(i, j);
                for inst in &mut self.instances {
                    swap(&mut inst.mount);
                }
                if self.sel_tower == Some(i) {
                    self.sel_tower = Some(j);
                } else if self.sel_tower == Some(j) {
                    self.sel_tower = Some(i);
                }
                ElementRef::Tower(j)
            }
            ElementRef::Truss(_) => {
                self.trusses.swap(i, j);
                for inst in &mut self.instances {
                    swap(&mut inst.truss_mount);
                }
                if self.sel_truss == Some(i) {
                    self.sel_truss = Some(j);
                } else if self.sel_truss == Some(j) {
                    self.sel_truss = Some(i);
                }
                ElementRef::Truss(j)
            }
        };
        self.hover_element = None;
        self.save(patch);
        out
    }

    /// Let go of every light on one element, leaving them where they hang.
    pub(crate) fn unhang_element(&mut self, patch: &Patch, r: ElementRef) -> usize {
        let hung = self.mounted_on(r);
        if hung.is_empty() {
            return 0;
        }
        self.push_undo();
        for &(k, _) in &hung {
            match r {
                ElementRef::Tower(_) => self.instances[k].mount = None,
                ElementRef::Truss(_) => self.instances[k].truss_mount = None,
            }
        }
        self.save(patch);
        hung.len()
    }

    /// Let go of every light on every element.
    pub(crate) fn unhang_all(&mut self, patch: &Patch) -> usize {
        let n = self
            .instances
            .iter()
            .filter(|i| i.mount.is_some() || i.truss_mount.is_some())
            .count();
        if n == 0 {
            return 0;
        }
        self.push_undo();
        for inst in &mut self.instances {
            inst.mount = None;
            inst.truss_mount = None;
        }
        self.save(patch);
        n
    }

    // ---- groups ----

    /// Remove tower `ti` and remap the mounts, with no undo step or save —
    /// for the bulk paths that remove several at once.
    fn remove_tower_raw(&mut self, ti: usize) {
        if ti >= self.towers.len() {
            return;
        }
        self.towers.remove(ti);
        for inst in &mut self.instances {
            match &mut inst.mount {
                Some((t, _)) if *t == ti => inst.mount = None,
                Some((t, _)) if *t > ti => *t -= 1,
                _ => {}
            }
        }
    }

    fn remove_truss_raw(&mut self, ti: usize) {
        if ti >= self.trusses.len() {
            return;
        }
        self.trusses.remove(ti);
        for inst in &mut self.instances {
            match &mut inst.truss_mount {
                Some((t, _)) if *t == ti => inst.truss_mount = None,
                Some((t, _)) if *t > ti => *t -= 1,
                _ => {}
            }
        }
    }

    /// Delete a whole composite in one undo step. Returns (parts, lights
    /// unhung).
    pub(crate) fn delete_group(&mut self, patch: &Patch, gid: u32) -> (usize, usize) {
        let members = self.group_members(gid);
        if members.is_empty() {
            return (0, 0);
        }
        let lights: usize = members.iter().map(|&r| self.mounted_on(r).len()).sum();
        self.push_undo();
        // Highest index first, so the lower ones stay valid.
        let mut trusses: Vec<usize> = members
            .iter()
            .filter_map(|r| match r {
                ElementRef::Truss(i) => Some(*i),
                _ => None,
            })
            .collect();
        trusses.sort_unstable_by(|a, b| b.cmp(a));
        for i in trusses {
            self.remove_truss_raw(i);
        }
        let mut towers: Vec<usize> = members
            .iter()
            .filter_map(|r| match r {
                ElementRef::Tower(i) => Some(*i),
                _ => None,
            })
            .collect();
        towers.sort_unstable_by(|a, b| b.cmp(a));
        for i in towers {
            self.remove_tower_raw(i);
        }
        if self.sel_tower.is_some_and(|i| i >= self.towers.len()) {
            self.sel_tower = None;
        }
        if self.sel_truss.is_some_and(|i| i >= self.trusses.len()) {
            self.sel_truss = None;
        }
        self.hover_element = None;
        self.save(patch);
        (members.len(), lights)
    }

    /// Break a composite apart, shortening each part's name when that still
    /// leaves it unique.
    pub(crate) fn ungroup(&mut self, patch: &Patch, gid: u32) {
        let members = self.group_members(gid);
        if members.is_empty() {
            return;
        }
        // Dropping the group tag is structural, not cosmetic: `group_members`,
        // `delete_group`, `move_group` and the outliner's nesting all key off
        // it, so it undoes as a step of its own like `delete_group` does.
        self.push_undo();
        let mut taken = self.names_in_use();
        for r in members {
            let old = self.element_name(r);
            taken.remove(&old);
            let fresh = match old.split_once(" · ") {
                Some((_, role)) if !role.trim().is_empty() && !taken.contains(role.trim()) => {
                    role.trim().to_owned()
                }
                _ => old.clone(),
            };
            taken.insert(fresh.clone());
            match r {
                ElementRef::Tower(i) => {
                    if let Some(t) = self.towers.get_mut(i) {
                        t.name = fresh;
                        t.group = None;
                    }
                }
                ElementRef::Truss(i) => {
                    if let Some(t) = self.trusses.get_mut(i) {
                        t.name = fresh;
                        t.group = None;
                    }
                }
            }
        }
        self.save(patch);
    }

    /// Slide a whole composite. `delta.y` raises the trusses and grows the
    /// towers, since a tower stands on the floor.
    pub(crate) fn move_group(&mut self, patch: &Patch, gid: u32, delta: V3) {
        let members = self.group_members(gid);
        if members.is_empty() {
            return;
        }
        for r in &members {
            match *r {
                ElementRef::Tower(i) => {
                    if let Some(t) = self.towers.get_mut(i) {
                        t.pos = t.pos + v3(delta.x, 0.0, delta.z);
                        t.height = (t.height + delta.y).clamp(1.2, 6.0);
                    }
                }
                ElementRef::Truss(i) => {
                    if let Some(t) = self.trusses.get_mut(i) {
                        t.pos = t.pos + delta;
                    }
                }
            }
        }
        for r in &members {
            match *r {
                ElementRef::Tower(i) => self.spin_tower_mounts(i, 0.0),
                ElementRef::Truss(i) => self.spin_truss_mounts(i, 0.0),
            }
        }
        self.reglue_mounts();
        self.save(patch);
    }

    /// Turn a whole composite about its own centre, keeping its parts rigid
    /// and its lights glued.
    pub(crate) fn rotate_group(&mut self, patch: &Patch, gid: u32, dturn: f32) {
        let members = self.group_members(gid);
        if members.is_empty() || dturn == 0.0 {
            return;
        }
        let positions: Vec<V3> = members
            .iter()
            .filter_map(|&r| match r {
                ElementRef::Tower(i) => self.towers.get(i).map(|t| t.pos),
                ElementRef::Truss(i) => self.trusses.get(i).map(|t| t.pos),
            })
            .collect();
        if positions.is_empty() {
            return;
        }
        let n = positions.len() as f32;
        let centre = v3(
            positions.iter().map(|p| p.x).sum::<f32>() / n,
            0.0,
            positions.iter().map(|p| p.z).sum::<f32>() / n,
        );
        let ang = dturn.to_radians();
        for r in &members {
            match *r {
                ElementRef::Tower(i) => {
                    if let Some(t) = self.towers.get_mut(i) {
                        let rel = v3(t.pos.x - centre.x, 0.0, t.pos.z - centre.z);
                        let spun = rotate_about(rel, v3(0.0, 1.0, 0.0), ang);
                        t.pos = v3(centre.x + spun.x, t.pos.y, centre.z + spun.z);
                        t.yaw_deg = (t.yaw_deg + dturn).rem_euclid(360.0);
                    }
                    self.spin_tower_mounts(i, dturn);
                }
                ElementRef::Truss(i) => {
                    let pitched = self.trusses.get(i).is_some_and(|t| t.pitch_deg.abs() > 0.5);
                    if let Some(t) = self.trusses.get_mut(i) {
                        let rel = v3(t.pos.x - centre.x, 0.0, t.pos.z - centre.z);
                        let spun = rotate_about(rel, v3(0.0, 1.0, 0.0), ang);
                        t.pos = v3(centre.x + spun.x, t.pos.y, centre.z + spun.z);
                        // A run stood on edge turns about its own lean, not
                        // its in-plane heading.
                        if pitched {
                            t.roll_deg = (t.roll_deg + dturn).rem_euclid(360.0);
                        } else {
                            t.yaw_deg = (t.yaw_deg + dturn).rem_euclid(360.0);
                        }
                    }
                    if pitched {
                        self.resync_truss_mounts(i);
                    } else {
                        self.spin_truss_mounts(i, dturn);
                    }
                }
            }
        }
        self.reglue_mounts();
        self.save(patch);
    }

    /// Mirror a whole composite into a new group across the centre line.
    pub(crate) fn mirror_group(&mut self, patch: &Patch, gid: u32) -> Option<u32> {
        let members = self.group_members(gid);
        if members.is_empty() {
            return None;
        }
        let kind = self.group_kind(gid)?;
        let name = format!("{} (mirror)", self.group_name(gid));
        let id = self.next_group_id();
        let group = ElementGroup { id, kind };
        self.push_undo();
        let mut last: Option<ElementRef> = None;
        for r in members {
            let old = self.element_name(r);
            let fresh = match old.split_once(" · ") {
                Some((_, role)) => format!("{name} · {role}"),
                None => name.clone(),
            };
            match r {
                ElementRef::Tower(i) => {
                    let Some(src) = self.towers.get(i) else { continue };
                    let mut t = mirror_tower(src);
                    t.name = fresh;
                    t.group = Some(group);
                    self.towers.push(t);
                    last = Some(ElementRef::Tower(self.towers.len() - 1));
                }
                ElementRef::Truss(i) => {
                    let Some(src) = self.trusses.get(i) else { continue };
                    let mut t = mirror_truss(src);
                    t.name = fresh;
                    t.group = Some(group);
                    self.trusses.push(t);
                    last = Some(ElementRef::Truss(self.trusses.len() - 1));
                }
            }
        }
        if let Some(r) = last {
            self.select_element(r);
        }
        self.save(patch);
        Some(id)
    }

    // ---- whole-rig edits ----

    /// Remove every tower and truss. The lights stay where they hang,
    /// unhung. Returns (elements removed, lights unhung).
    pub(crate) fn clear_rig(&mut self, patch: &Patch) -> (usize, usize) {
        let elements = self.towers.len() + self.trusses.len();
        if elements == 0 {
            return (0, 0);
        }
        self.push_undo();
        let mut lights = 0;
        for inst in &mut self.instances {
            if inst.mount.is_some() || inst.truss_mount.is_some() {
                lights += 1;
            }
            inst.mount = None;
            inst.truss_mount = None;
        }
        self.towers.clear();
        self.trusses.clear();
        self.sel_tower = None;
        self.sel_truss = None;
        self.hover_element = None;
        self.save(patch);
        (elements, lights)
    }

    /// Replace the rig with a saved one, leaving the lights where they are
    /// (unhung, since the slot numbers no longer mean anything).
    pub(crate) fn import_rig(&mut self, patch: &Patch, lf: LayoutFile) {
        self.push_undo();
        self.towers = lf.towers;
        self.trusses = lf.trusses;
        for inst in &mut self.instances {
            inst.mount = None;
            inst.truss_mount = None;
        }
        self.sel_tower = None;
        self.sel_truss = None;
        self.hover_element = None;
        self.save(patch);
    }

    /// Append a saved rig to this one, renumbering its groups and nudging
    /// its elements clear of what is already built. Returns how many were
    /// added.
    pub(crate) fn merge_rig(&mut self, patch: &Patch, lf: LayoutFile) -> usize {
        if lf.towers.is_empty() && lf.trusses.is_empty() {
            return 0;
        }
        let mut incoming: Vec<u32> = lf
            .towers
            .iter()
            .filter_map(|t| t.group.map(|g| g.id))
            .chain(lf.trusses.iter().filter_map(|t| t.group.map(|g| g.id)))
            .collect();
        incoming.sort_unstable();
        incoming.dedup();
        let base = self.next_group_id();
        let remap: BTreeMap<u32, u32> =
            incoming.iter().enumerate().map(|(k, &id)| (id, base + k as u32)).collect();
        self.push_undo();
        let mut added = 0;
        for mut tw in lf.towers {
            tw.group = tw.group.map(|g| ElementGroup { id: remap[&g.id], kind: g.kind });
            tw.pos = self.free_spot(v3(tw.pos.x, 0.0, tw.pos.z), 1.5);
            if tw.name.trim().is_empty() || self.names_in_use().contains(tw.name.trim()) {
                let stem = base_name(&tw.name).to_owned();
                let stem = if stem.is_empty() { "Tower".to_owned() } else { stem };
                tw.name = self.next_element_name(&stem);
            }
            self.towers.push(tw);
            added += 1;
        }
        for mut tr in lf.trusses {
            tr.group = tr.group.map(|g| ElementGroup { id: remap[&g.id], kind: g.kind });
            let spot = self.free_spot(v3(tr.pos.x, 0.0, tr.pos.z), 1.5);
            tr.pos = v3(spot.x, tr.pos.y, spot.z);
            if tr.name.trim().is_empty() || self.names_in_use().contains(tr.name.trim()) {
                let stem = match tr.kind {
                    TrussKind::Straight => "F34 truss",
                    TrussKind::Radius => "Radius truss",
                };
                tr.name = self.next_element_name(stem);
            }
            self.trusses.push(tr);
            added += 1;
        }
        self.hover_element = None;
        self.save(patch);
        added
    }

    /// Put every hung light back on its slot after the rig moved under it.
    fn reglue_mounts(&mut self) {
        for k in 0..self.instances.len() {
            if let Some((ti, slot)) = self.instances[k].mount {
                match self.towers.get(ti) {
                    Some(tw) => self.instances[k].t.pos = tw.slot_pos(slot),
                    None => self.instances[k].mount = None,
                }
            }
            if let Some((ti, slot)) = self.instances[k].truss_mount {
                match self.trusses.get(ti) {
                    Some(tr) => self.instances[k].t.pos = tr.slot_pos(slot),
                    None => self.instances[k].truss_mount = None,
                }
            }
        }
    }

    // ---- setups ----

    fn setups_dir() -> PathBuf {
        PathBuf::from(SETUPS_DIR)
    }

    /// Every setup in the app's folder, described. The Build tab caches
    /// this and only calls it again when something changed.
    pub(crate) fn setup_infos() -> Vec<SetupInfo> {
        list_setup_infos(&Self::setups_dir())
    }

    /// Read one setup file without applying it.
    pub(crate) fn read_setup(name: &str) -> Option<LayoutFile> {
        std::fs::read_to_string(Self::setup_path(name))
            .ok()
            .and_then(|t| serde_json::from_str::<LayoutFile>(&t).ok())
    }

    /// Rename a setup file. Refuses an empty name, a missing source and an
    /// existing target.
    pub(crate) fn rename_setup(old: &str, new: &str) -> bool {
        if new.trim().is_empty() {
            return false;
        }
        let from = Self::setup_path(old);
        let to = Self::setup_path(new);
        if from == to || to.exists() || !from.exists() {
            return false;
        }
        std::fs::rename(from, to).is_ok()
    }

    /// Save the towers and trusses only — a rig template with no light
    /// positions in it.
    pub(crate) fn save_setup_rig(&self, patch: &Patch, name: &str) -> bool {
        if name.trim().is_empty() {
            return false;
        }
        let _ = std::fs::create_dir_all(SETUPS_DIR);
        let mut lf = self.export_layout(patch);
        lf.instances.clear();
        serde_json::to_string_pretty(&lf)
            .ok()
            .and_then(|json| std::fs::write(Self::setup_path(name), json).ok())
            .is_some()
    }

    /// Load only the light positions from a setup, keeping the rig that is
    /// built. Mounts survive only where the slots still exist.
    pub(crate) fn load_setup_lights(
        &mut self,
        patch: &Patch,
        set: &Settings,
        name: &str,
    ) -> bool {
        let Some(lf) = Self::read_setup(name) else { return false };
        let merged = LayoutFile {
            instances: lf.instances,
            towers: self.towers.clone(),
            trusses: self.trusses.clone(),
        };
        self.import_layout(patch, set, merged);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::headless::{render_frames, save};

    fn view(name: &str) -> StageView {
        let mut v = StageView::new();
        v.layout_path = std::env::temp_dir().join(format!("dmxpress_builder_{name}.json"));
        let _ = std::fs::remove_file(&v.layout_path);
        v
    }

    fn patch0() -> Patch {
        Patch { fixtures: Vec::new(), warnings: Vec::new() }
    }

    /// Two built-in fixtures, so `sync` makes exactly two instances.
    fn patch2() -> Patch {
        let p = &crate::profiles::PROFILES[0];
        Patch {
            fixtures: vec![p.to_fixture("A".into(), 1), p.to_fixture("B".into(), 9)],
            warnings: Vec::new(),
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn presets_are_the_standard_sizes() {
        assert_eq!(F34_LENGTHS, [0.5, 1.0, 1.5, 2.0, 3.0]);
        for a in [90.0, 180.0, 360.0] {
            assert!(ARC_PRESETS.contains(&a), "{a} missing");
        }
        // Every preset survives the clamp unchanged.
        for &l in &F34_LENGTHS {
            let r = Recipe::Straight { length: l, height: 3.2, yaw: 0.0, grounded: true };
            assert_eq!(r.clamped(), r);
        }
        for &h in &HEIGHT_PRESETS {
            let r = Recipe::Tower { height: h, width: 2.4, yaw: 0.0 };
            assert_eq!(r.clamped(), r);
        }
        for &s in &SPAN_PRESETS {
            let r = Recipe::Goalpost { span: s, height: 3.5, yaw: 0.0 };
            assert_eq!(r.clamped(), r);
        }
        for &rad in &RADIUS_PRESETS {
            for &arc in &ARC_PRESETS {
                let r =
                    Recipe::Radius { radius: rad, arc, height: 3.2, yaw: 0.0, grounded: false };
                assert_eq!(r.clamped(), r);
            }
        }
    }

    #[test]
    fn kind_metadata_is_unique() {
        assert_eq!(RecipeKind::ALL.len(), RecipeKind::COUNT);
        for (i, a) in RecipeKind::ALL.iter().enumerate() {
            assert_eq!(RecipeKind::from_key(a.key()), Some(*a));
            assert!(!a.hover().is_empty() && a.hover().ends_with('.'));
            for b in &RecipeKind::ALL[i + 1..] {
                assert_ne!(a.key(), b.key());
                assert_ne!(a.label(), b.label());
            }
            assert_eq!(Recipe::default_for(*a).kind(), *a);
            assert!(!Recipe::default_for(*a).variants().is_empty());
        }
    }

    #[test]
    fn plan_goalpost_spans_its_legs() {
        let g = ElementGroup { id: 7, kind: CompositeKind::Goalpost };
        let (tws, trs) = plan(
            Recipe::Goalpost { span: 4.0, height: 3.5, yaw: 0.0 },
            v3(1.0, 0.0, -2.0),
            Some(g),
            7,
        );
        assert_eq!((tws.len(), trs.len()), (2, 1));
        assert!(near(tws[0].pos.x, -1.0) && near(tws[1].pos.x, 3.0));
        for t in &tws {
            assert!(near(t.pos.z, -2.0) && near(t.height, 3.5) && near(t.width, 1.2));
            assert_eq!(t.group.map(|g| g.id), Some(7));
        }
        assert_eq!(tws[0].name, "Goalpost 7 · left leg");
        assert_eq!(tws[1].name, "Goalpost 7 · right leg");
        let beam = &trs[0];
        assert_eq!(beam.name, "Goalpost 7 · beam");
        assert!(near(beam.pos.x, 1.0) && near(beam.pos.y, 3.5) && near(beam.pos.z, -2.0));
        assert!(near(beam.length, 4.0) && near(beam.yaw_deg, 90.0) && !beam.grounded);
        // The beam's ends sit on the tower tops.
        let side = dir_from_angles(90.0, 0.0);
        for (t, s) in tws.iter().zip([-1.0f32, 1.0]) {
            let end = beam.pos + side * (2.0 * s);
            let top = t.pos + v3(0.0, t.height, 0.0);
            assert!(near(end.x, top.x) && near(end.y, top.y) && near(end.z, top.z));
        }
    }

    #[test]
    fn plan_goalpost_follows_heading() {
        let (tws, trs) =
            plan(Recipe::Goalpost { span: 4.0, height: 3.0, yaw: 90.0 }, v3(0.0, 0.0, 0.0), None, 1);
        // side = dir_from_angles(180) = -Z, so the legs straddle Z.
        assert!(near(tws[0].pos.z, 2.0) && near(tws[1].pos.z, -2.0));
        assert!(near(tws[0].pos.x, 0.0) && near(tws[1].pos.x, 0.0));
        assert!(near(trs[0].yaw_deg, 180.0));
    }

    #[test]
    fn plan_box_is_a_rectangle_of_grounded_runs() {
        let (tws, trs) = plan(
            Recipe::Box { width: 6.0, depth: 4.0, height: 4.0, yaw: 0.0, grounded: true },
            v3(0.0, 0.0, 0.0),
            Some(ElementGroup { id: 2, kind: CompositeKind::Box }),
            2,
        );
        assert!(tws.is_empty());
        assert_eq!(trs.len(), 4);
        let mut lengths: Vec<f32> = trs.iter().map(|t| t.length).collect();
        lengths.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(lengths, vec![4.0, 4.0, 6.0, 6.0]);
        for t in &trs {
            assert!(near(t.pos.y, 4.0) && t.grounded);
            assert_eq!(t.group.map(|g| g.id), Some(2));
        }
        // front / back straddle Z at ±depth/2; left / right straddle X.
        assert!(near(trs[0].pos.z, 2.0) && near(trs[1].pos.z, -2.0));
        assert!(near(trs[2].pos.x, -3.0) && near(trs[3].pos.x, 3.0));
        assert!(near(trs[0].yaw_deg, 90.0) && near(trs[2].yaw_deg, 0.0));
    }

    #[test]
    fn plan_ring_is_one_closed_radius_run() {
        let (tws, trs) = plan(
            Recipe::Ring { radius: 2.5, height: 4.0 },
            v3(0.0, 0.0, 0.0),
            Some(ElementGroup { id: 3, kind: CompositeKind::Ring }),
            3,
        );
        assert!(tws.is_empty() && trs.len() == 1);
        let t = &trs[0];
        assert_eq!(t.kind, TrussKind::Radius);
        assert!(near(t.arc_deg, 360.0) && near(t.radius, 2.5) && !t.grounded);
        assert_eq!(t.name, "Ring 3");
    }

    #[test]
    fn plan_tower_pair_is_symmetric() {
        let (tws, trs) = plan(
            Recipe::TowerPair { spacing: 5.0, height: 4.0, width: 2.0, yaw: 30.0 },
            v3(0.0, 0.0, 0.0),
            None,
            1,
        );
        assert!(trs.is_empty() && tws.len() == 2);
        assert!(near(tws[0].height, tws[1].height) && near(tws[0].width, 2.0));
        assert!(near(tws[0].yaw_deg, 30.0) && near(tws[1].yaw_deg, 30.0));
        let side = dir_from_angles(120.0, 0.0);
        assert!(near(tws[1].pos.x - tws[0].pos.x, side.x * 5.0));
        assert!(near(tws[1].pos.z - tws[0].pos.z, side.z * 5.0));
    }

    #[test]
    fn plan_arch_ends_on_its_towers() {
        let (tws, trs) =
            plan(Recipe::Arch { span: 4.0, rise: 3.0, yaw: 30.0 }, v3(0.0, 0.0, 0.0), None, 1);
        assert_eq!((tws.len(), trs.len()), (2, 1));
        assert!(tws.iter().all(|t| near(t.height, 3.0)));
        let a = &trs[0];
        assert!(near(a.pitch_deg, 90.0) && near(a.roll_deg, 30.0) && near(a.yaw_deg, 90.0));
        assert!(near(a.arc_deg, 180.0) && near(a.radius, 2.0) && near(a.pos.y, 3.0));
        let last = a.slot_count() - 1;
        let ends = [a.slot_pos(0), a.slot_pos(last)];
        for t in &tws {
            let top = t.pos + v3(0.0, t.height, 0.0);
            let best = ends
                .iter()
                .map(|e| (*e - top).len())
                .fold(f32::INFINITY, f32::min);
            assert!(best < 0.3, "arch end {best:.3} m from a tower top");
        }
        // The middle of the sweep is the high point.
        let mid = a.slot_pos(a.slot_count() / 2);
        assert!(mid.y >= 3.0 + 2.0 - 0.3, "arch apex only {:.2} m", mid.y);
    }

    #[test]
    fn recipe_clamped_keeps_values_in_range() {
        let s = Recipe::Straight { length: 99.0, height: 3.2, yaw: 400.0, grounded: true };
        match s.clamped() {
            Recipe::Straight { length, yaw, .. } => {
                assert!(near(length, 6.0) && near(yaw, 40.0));
            }
            _ => panic!("kind changed"),
        }
        match (Recipe::Radius { radius: 0.1, arc: 5.0, height: 3.0, yaw: 0.0, grounded: false })
            .clamped()
        {
            Recipe::Radius { radius, arc, .. } => assert!(near(radius, 0.5) && near(arc, 15.0)),
            _ => panic!("kind changed"),
        }
        match (Recipe::Tower { height: 0.1, width: 9.0, yaw: -90.0 }).clamped() {
            Recipe::Tower { height, width, yaw } => {
                assert!(near(height, 1.2) && near(width, 4.0) && near(yaw, 270.0));
            }
            _ => panic!("kind changed"),
        }
        match (Recipe::Goalpost { span: 0.5, height: 9.0, yaw: 0.0 }).clamped() {
            Recipe::Goalpost { span, height, .. } => assert!(near(span, 2.0) && near(height, 6.0)),
            _ => panic!("kind changed"),
        }
    }

    #[test]
    fn build_pushes_one_undo_step_and_selects_the_beam() {
        let mut v = view("build_goalpost");
        let p = patch0();
        v.selection.insert(0);
        v.sel_stage = true;
        let res = v.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::Stagger);
        assert_eq!(v.undo_stack.len(), 1);
        assert_eq!(v.sel_truss, Some(0));
        assert_eq!(v.sel_tower, None);
        assert!(v.selection.is_empty() && !v.sel_stage);
        assert_eq!(res.group, Some(1));
        assert_eq!(res.added.len(), 3);
        assert_eq!(res.summary, "Goalpost 1 (2 towers + 1 truss)");
        assert!(v.undo(&p));
        assert!(v.towers.is_empty() && v.trusses.is_empty());
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn build_summaries_read_like_sentences() {
        let mut v = view("build_summaries");
        let p = patch0();
        assert_eq!(
            v.build(&p, Recipe::default_for(RecipeKind::Tower), Placement::Stagger).summary,
            "Tower 1 (3.2 m)"
        );
        assert_eq!(
            v.build(&p, Recipe::default_for(RecipeKind::Straight), Placement::Stagger).summary,
            "F34 truss 3.0 m (F34 truss 1)"
        );
        assert_eq!(
            v.build(&p, Recipe::default_for(RecipeKind::Radius), Placement::Stagger).summary,
            "Radius truss r 2.0 m 90° (Radius truss 1)"
        );
        assert_eq!(
            v.build(&p, Recipe::default_for(RecipeKind::Box), Placement::Stagger).summary,
            "Box 1 (4 trusses)"
        );
        assert_eq!(
            v.build(&p, Recipe::default_for(RecipeKind::Ring), Placement::Stagger).summary,
            "Ring 1"
        );
        assert_eq!(
            v.build(&p, Recipe::default_for(RecipeKind::Arch), Placement::Stagger).summary,
            "Arch 1 (2 towers + 1 truss)"
        );
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn stagger_matches_legacy_add_placement() {
        let mut v = view("stagger");
        let p = patch0();
        v.build(&p, Recipe::default_for(RecipeKind::Tower), Placement::Stagger);
        v.build(&p, Recipe::default_for(RecipeKind::Tower), Placement::Stagger);
        assert!(near(v.towers[1].pos.x, 1.5 - 2.0) && near(v.towers[1].pos.z, 4.0));
        v.build(&p, Recipe::default_for(RecipeKind::Straight), Placement::Stagger);
        assert!(near(v.trusses[0].pos.x, 2.0 * 1.5 - 2.0) && near(v.trusses[0].pos.z, -4.0));
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn free_spot_steps_sideways_past_existing_elements() {
        let mut v = view("free_spot");
        let mut tw = Tower::default();
        tw.pos = v3(0.0, 0.0, 4.0);
        v.towers.push(tw);
        let mut tr = Truss::straight();
        tr.pos = v3(1.5, 3.2, 4.0);
        v.trusses.push(tr);
        let spot = v.free_spot(v3(0.0, 0.0, 4.0), 1.5);
        assert!(near(spot.x, -1.5) && near(spot.z, 4.0), "{spot:?}");
    }

    #[test]
    fn next_element_name_skips_taken_numbers() {
        let mut v = view("names");
        for name in ["Tower 1", "SL stand", "Tower 3"] {
            let mut tw = Tower::default();
            tw.name = name.into();
            v.towers.push(tw);
        }
        assert_eq!(v.next_element_name("Tower"), "Tower 2");
        // An unnamed second tower displays "Tower 2", so that is taken too.
        v.towers[1].name = String::new();
        assert_eq!(v.next_element_name("Tower"), "Tower 4");
    }

    #[test]
    fn next_group_id_is_max_plus_one_from_one() {
        let mut v = view("gid");
        let p = patch0();
        assert_eq!(v.next_group_id(), 1);
        v.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::Stagger);
        assert_eq!(v.next_group_id(), 2);
        let mut tr = Truss::straight();
        tr.group = Some(ElementGroup { id: 9, kind: CompositeKind::Box });
        v.trusses.push(tr);
        assert_eq!(v.next_group_id(), 10);
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn element_names_match_the_legacy_inspector_strings() {
        let mut v = view("legacy_names");
        v.towers.push(Tower::default());
        v.trusses.push(Truss::straight());
        v.trusses.push(Truss::radius());
        assert_eq!(v.element_name(ElementRef::Tower(0)), "Tower 1");
        assert_eq!(v.element_name(ElementRef::Truss(0)), "F34 truss 1");
        assert_eq!(v.element_name(ElementRef::Truss(1)), "Radius truss 2");
        v.towers[0].name = "  ".into();
        assert_eq!(v.element_name(ElementRef::Tower(0)), "Tower 1");
        v.towers[0].name = " SL stand ".into();
        assert_eq!(v.element_name(ElementRef::Tower(0)), "SL stand");
    }

    #[test]
    fn group_name_derives_from_members_and_rename_rewrites_them() {
        let mut v = view("group_names");
        let p = patch0();
        v.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::Stagger);
        assert_eq!(v.group_name(1), "Goalpost 1");
        assert_eq!(v.group_members(1).len(), 3);
        v.rename_group(&p, 1, "US goalpost");
        assert_eq!(v.element_name(ElementRef::Tower(0)), "US goalpost · left leg");
        assert_eq!(v.element_name(ElementRef::Truss(0)), "US goalpost · beam");
        assert_eq!(v.group_name(1), "US goalpost");
        // One member renamed flat: the others still carry the group name.
        v.rename_element(&p, ElementRef::Tower(0), "Bar");
        assert_eq!(v.group_name(1), "US goalpost");
        // Every member flat: fall back to the kind and the id.
        v.rename_element(&p, ElementRef::Tower(1), "B");
        v.rename_element(&p, ElementRef::Truss(0), "C");
        assert_eq!(v.group_name(1), "Goalpost 1");
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn elements_nest_groups_then_standalone() {
        let mut v = view("outline");
        let p = patch0();
        v.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::Stagger);
        v.build(&p, Recipe::default_for(RecipeKind::Ring), Placement::Stagger);
        v.build(&p, Recipe::default_for(RecipeKind::Tower), Placement::Stagger);
        let rows = v.elements();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].group.map(|g| g.id), Some(1));
        assert_eq!(rows[3].group.map(|g| g.id), Some(2));
        // The ring is a one-part group, so its row wears the ring glyph.
        assert_eq!(rows[3].glyph, RecipeKind::Ring);
        assert_eq!(rows[4].group, None);
        assert_eq!(rows[4].glyph, RecipeKind::Tower);
        assert_eq!(rows[0].caption, "3.5 m · 0/8");
        assert_eq!(rows[2].caption, "4.0 m · 0/32");
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn arch_rows_wear_the_arch_glyph() {
        let mut v = view("arch_glyph");
        let p = patch0();
        v.build(&p, Recipe::default_for(RecipeKind::Arch), Placement::Stagger);
        let rows = v.elements();
        let arch = rows.iter().find(|r| matches!(r.r, ElementRef::Truss(_))).unwrap();
        assert_eq!(arch.glyph, RecipeKind::Arch);
        assert!(arch.caption.starts_with("arch r 2.0 m"), "{}", arch.caption);
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn delete_group_remaps_mounts_in_one_step() {
        let mut v = view("delete_group");
        let p = patch2();
        v.sync(&p, &Settings::default());
        assert_eq!(v.instances.len(), 2);
        v.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::Stagger);
        v.build(&p, Recipe::default_for(RecipeKind::Tower), Placement::Stagger);
        v.build(&p, Recipe::default_for(RecipeKind::Straight), Placement::Stagger);
        assert_eq!((v.towers.len(), v.trusses.len()), (3, 2));
        v.instances[0].mount = Some((2, 1));
        v.instances[1].truss_mount = Some((1, 0));
        let before = v.undo_stack.len();
        assert_eq!(v.delete_group(&p, 1), (3, 0));
        assert_eq!((v.towers.len(), v.trusses.len()), (1, 1));
        assert_eq!(v.instances[0].mount, Some((0, 1)));
        assert_eq!(v.instances[1].truss_mount, Some((0, 0)));
        assert_eq!(v.undo_stack.len(), before + 1);
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn delete_group_unhangs_lights_that_hung_on_it() {
        let mut v = view("delete_group_lights");
        let p = patch2();
        v.sync(&p, &Settings::default());
        v.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::Stagger);
        v.instances[0].truss_mount = Some((0, 2));
        let pos = v.instances[0].t.pos;
        assert_eq!(v.delete_group(&p, 1), (3, 1));
        assert_eq!(v.instances[0].truss_mount, None);
        assert!(near(v.instances[0].t.pos.x, pos.x) && near(v.instances[0].t.pos.y, pos.y));
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn undo_restores_a_deleted_group() {
        let mut v = view("undo_group");
        let p = patch0();
        v.build(&p, Recipe::default_for(RecipeKind::Ring), Placement::Stagger);
        v.delete_group(&p, 1);
        assert!(v.trusses.is_empty());
        assert!(v.undo(&p));
        assert_eq!(v.trusses.len(), 1);
        assert_eq!(v.trusses[0].group.map(|g| g.id), Some(1));
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn ungroup_shortens_names_and_drops_the_tag() {
        let mut v = view("ungroup");
        let p = patch0();
        v.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::Stagger);
        v.ungroup(&p, 1);
        assert!(v.towers.iter().all(|t| t.group.is_none()));
        assert_eq!(v.element_name(ElementRef::Tower(0)), "left leg");
        assert_eq!(v.element_name(ElementRef::Truss(0)), "beam");
        assert!(v.elements().iter().all(|r| r.group.is_none()));
        let _ = std::fs::remove_file(&v.layout_path);
    }

    /// Ungroup is a structural change, so it undoes on its own. It used to
    /// save without an undo step, which left the top of the stack pointing
    /// at the state before the composite was *built* — one ⌘Z then deleted
    /// the whole goalpost instead of putting the grouping back.
    #[test]
    fn ungroup_undoes_on_its_own_instead_of_deleting_the_composite() {
        let mut v = view("ungroup_undo");
        let p = patch0();
        v.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::Stagger);
        let parts = (v.towers.len(), v.trusses.len());
        let depth = v.undo_stack.len();
        v.ungroup(&p, 1);
        assert_eq!(v.undo_stack.len(), depth + 1);
        assert!(v.undo(&p));
        assert_eq!((v.towers.len(), v.trusses.len()), parts, "undo deleted the composite");
        assert!(v.towers.iter().all(|t| t.group.is_some()));
        assert_eq!(v.element_name(ElementRef::Tower(0)), "Goalpost 1 · left leg");
        // Ungrouping nothing still costs nothing.
        let depth = v.undo_stack.len();
        v.ungroup(&p, 99);
        assert_eq!(v.undo_stack.len(), depth);
        let _ = std::fs::remove_file(&v.layout_path);
    }

    /// Move up / down steps through the rows the outliner draws. Those are
    /// bucketed by group tag, not by vec index, so a swap with a neighbour
    /// from another bucket (or of the other kind) leaves the list looking
    /// exactly as it was — it used to do it anyway, spending an undo step
    /// and logging "Moved 'X' up" for a move nobody could see.
    #[test]
    fn move_element_steps_through_outliner_rows_not_vec_indices() {
        let mut v = view("move_rows");
        let p = patch0();
        v.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::Stagger);
        v.build(&p, Recipe::default_for(RecipeKind::Tower), Placement::Stagger);
        let rows = |v: &StageView| v.elements().iter().map(|e| e.label.clone()).collect::<Vec<_>>();
        // Two legs and a beam under the composite, then the lone tower.
        let before = rows(&v);
        assert_eq!(before.len(), 4);
        let lone = ElementRef::Tower(2);
        // Up from the lone tower is the composite's beam: another bucket
        // and another kind, so nothing happens at all.
        let depth = v.undo_stack.len();
        assert_eq!(v.move_element(&p, lone, -1), lone);
        assert_eq!(v.undo_stack.len(), depth);
        assert_eq!(rows(&v), before);
        // The beam's neighbour above is a leg of its own group — same
        // bucket, wrong kind — so that is a no-op too.
        assert_eq!(v.move_element(&p, ElementRef::Truss(0), -1), ElementRef::Truss(0));
        assert_eq!(v.undo_stack.len(), depth);
        assert_eq!(rows(&v), before);
        // Inside the group the two legs really do swap places.
        assert_eq!(v.move_element(&p, ElementRef::Tower(0), 1), ElementRef::Tower(1));
        assert_eq!(v.undo_stack.len(), depth + 1);
        let after = rows(&v);
        assert_eq!(after[0], before[1]);
        assert_eq!(after[1], before[0]);
        assert_eq!(after[2..], before[2..]);
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn move_element_swaps_and_remaps_mounts() {
        let mut v = view("move_element");
        let p = patch2();
        v.sync(&p, &Settings::default());
        for _ in 0..3 {
            v.build(&p, Recipe::default_for(RecipeKind::Tower), Placement::Stagger);
        }
        v.instances[0].mount = Some((0, 2));
        v.instances[1].mount = Some((2, 2));
        v.sel_tower = Some(0);
        let before = v.undo_stack.len();
        assert_eq!(v.move_element(&p, ElementRef::Tower(0), 1), ElementRef::Tower(1));
        assert_eq!(v.instances[0].mount, Some((1, 2)));
        assert_eq!(v.instances[1].mount, Some((2, 2)));
        assert_eq!(v.sel_tower, Some(1));
        assert_eq!(v.undo_stack.len(), before + 1);
        // At the top it does nothing at all.
        let steady = v.undo_stack.len();
        assert_eq!(v.move_element(&p, ElementRef::Tower(0), -1), ElementRef::Tower(0));
        assert_eq!(v.undo_stack.len(), steady);
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn move_group_translates_and_rotate_group_keeps_parts_rigid() {
        let mut v = view("move_group");
        let p = patch2();
        v.sync(&p, &Settings::default());
        v.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::At(v3(0.0, 0.0, 0.0)));
        v.instances[0].truss_mount = Some((0, 1));
        let tower_before: Vec<V3> = v.towers.iter().map(|t| t.pos).collect();
        let truss_before = v.trusses[0].pos;
        let height_before = v.towers[0].height;
        v.move_group(&p, 1, v3(1.0, 0.5, -2.0));
        for (t, was) in v.towers.iter().zip(&tower_before) {
            assert!(near(t.pos.x, was.x + 1.0) && near(t.pos.z, was.z - 2.0));
            assert!(near(t.pos.y, 0.0));
        }
        assert!(near(v.towers[0].height, height_before + 0.5));
        assert!(near(v.trusses[0].pos.x, truss_before.x + 1.0));
        assert!(near(v.trusses[0].pos.y, truss_before.y + 0.5));
        // Rigid: pairwise distances survive a turn, and the light re-glues.
        let pairs = |v: &StageView| -> Vec<f32> {
            let mut pts: Vec<V3> = v.towers.iter().map(|t| t.pos).collect();
            pts.extend(v.trusses.iter().map(|t| t.pos));
            let mut out = Vec::new();
            for i in 0..pts.len() {
                for j in i + 1..pts.len() {
                    out.push((pts[i] - pts[j]).len());
                }
            }
            out
        };
        let before = pairs(&v);
        let yaw_before = v.towers[0].yaw_deg;
        v.rotate_group(&p, 1, 90.0);
        for (a, b) in pairs(&v).iter().zip(&before) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
        assert!(near(v.towers[0].yaw_deg, (yaw_before + 90.0).rem_euclid(360.0)));
        let slot = v.trusses[0].slot_pos(1);
        let at = v.instances[0].t.pos;
        assert!(near(at.x, slot.x) && near(at.y, slot.y) && near(at.z, slot.z));
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn mirror_element_copy_reflects_across_centre() {
        let mut v = view("mirror");
        let p = patch0();
        let mut tw = Tower::default();
        tw.pos = v3(-3.0, 0.0, 2.0);
        tw.yaw_deg = 30.0;
        tw.name = "SL stand".into();
        tw.group = Some(ElementGroup { id: 4, kind: CompositeKind::TowerPair });
        v.towers.push(tw);
        let made = v.mirror_element_copy(&p, ElementRef::Tower(0)).unwrap();
        assert_eq!(made, ElementRef::Tower(1));
        assert!(near(v.towers[1].pos.x, 3.0) && near(v.towers[1].pos.z, 2.0));
        assert!(near(v.towers[1].yaw_deg, 330.0));
        assert_eq!(v.towers[1].name, "SL stand (mirror)");
        assert_eq!(v.towers[1].group, None);
        let mut tr = Truss::radius();
        tr.yaw_deg = 20.0;
        tr.arc_deg = 90.0;
        tr.roll_deg = 30.0;
        v.trusses.push(tr);
        v.mirror_element_copy(&p, ElementRef::Truss(0)).unwrap();
        assert!(near(v.trusses[1].yaw_deg, 250.0), "{}", v.trusses[1].yaw_deg);
        assert!(near(v.trusses[1].roll_deg, 330.0));
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn mirror_group_makes_a_new_group() {
        let mut v = view("mirror_group");
        let p = patch0();
        v.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::At(v3(3.0, 0.0, 0.0)));
        let gid = v.mirror_group(&p, 1).unwrap();
        assert_eq!(gid, 2);
        assert_eq!(v.group_members(2).len(), 3);
        assert_eq!(v.group_name(2), "Goalpost 1 (mirror)");
        assert!(near(v.towers[2].pos.x, -v.towers[0].pos.x));
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn duplicate_element_is_standalone_and_offset() {
        let mut v = view("duplicate");
        let p = patch0();
        v.build(&p, Recipe::default_for(RecipeKind::Straight), Placement::At(v3(0.0, 0.0, 0.0)));
        let src = v.trusses[0].pos;
        let before = v.undo_stack.len();
        let made = v.duplicate_element(&p, ElementRef::Truss(0)).unwrap();
        assert_eq!(made, ElementRef::Truss(1));
        assert!(near(v.trusses[1].pos.x, src.x + 1.5));
        assert!(near(v.trusses[1].pos.y, src.y));
        assert_eq!(v.trusses[1].group, None);
        assert_eq!(v.trusses[1].name, "F34 truss 2");
        assert_eq!(v.sel_truss, Some(1));
        assert_eq!(v.undo_stack.len(), before + 1);
        // A composite part loses its group when copied.
        v.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::At(v3(-6.0, 0.0, 0.0)));
        let leg = v.duplicate_element(&p, ElementRef::Tower(0)).unwrap();
        match leg {
            ElementRef::Tower(i) => assert_eq!(v.towers[i].group, None),
            _ => panic!("a tower was copied"),
        }
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn unhang_and_clear() {
        let mut v = view("unhang");
        let p = patch2();
        v.sync(&p, &Settings::default());
        v.build(&p, Recipe::default_for(RecipeKind::Tower), Placement::Stagger);
        v.build(&p, Recipe::default_for(RecipeKind::Straight), Placement::Stagger);
        v.instances[0].mount = Some((0, 0));
        v.instances[1].truss_mount = Some((0, 3));
        let kept = v.instances[1].t.pos;
        let before = v.undo_stack.len();
        assert_eq!(v.unhang_element(&p, ElementRef::Tower(0)), 1);
        assert_eq!(v.instances[0].mount, None);
        assert_eq!(v.instances[1].truss_mount, Some((0, 3)));
        assert_eq!(v.clear_rig(&p), (2, 1));
        assert!(v.towers.is_empty() && v.trusses.is_empty());
        assert!(near(v.instances[1].t.pos.x, kept.x) && near(v.instances[1].t.pos.z, kept.z));
        assert_eq!(v.undo_stack.len(), before + 2);
        // Nothing left to unhang or clear: no further undo steps.
        let steady = v.undo_stack.len();
        assert_eq!(v.unhang_all(&p), 0);
        assert_eq!(v.clear_rig(&p), (0, 0));
        assert_eq!(v.undo_stack.len(), steady);
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn unhang_all_lets_go_of_everything() {
        let mut v = view("unhang_all");
        let p = patch2();
        v.sync(&p, &Settings::default());
        v.build(&p, Recipe::default_for(RecipeKind::Tower), Placement::Stagger);
        v.instances[0].mount = Some((0, 0));
        v.instances[1].mount = Some((0, 1));
        assert_eq!(v.unhang_all(&p), 2);
        assert!(v.instances.iter().all(|i| i.mount.is_none()));
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn select_element_is_exclusive_and_select_mounted_wins() {
        let mut v = view("select");
        let p = patch2();
        v.sync(&p, &Settings::default());
        v.build(&p, Recipe::default_for(RecipeKind::Straight), Placement::Stagger);
        v.selection.insert(0);
        v.sel_stage = true;
        v.select_element(ElementRef::Truss(0));
        assert!(v.selection.is_empty() && !v.sel_stage);
        assert_eq!((v.sel_tower, v.sel_truss), (None, Some(0)));
        assert_eq!(v.select_mounted(ElementRef::Truss(0)), 0);
        assert_eq!(v.sel_truss, Some(0));
        v.instances[1].truss_mount = Some((0, 5));
        assert_eq!(v.select_mounted(ElementRef::Truss(0)), 1);
        assert_eq!(v.sel_truss, None);
        assert_eq!(v.last_selected, Some(v.instances[1].fixture));
        assert_eq!(v.mount_face_label(ElementRef::Truss(0), 5), "top");
        let face = v.trusses[0].slot_count();
        assert_eq!(v.mount_face_label(ElementRef::Truss(0), face), "bottom");
        assert_eq!(v.mount_face_label(ElementRef::Tower(0), 5), "under");
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn hidden_elements_are_skipped_by_snap() {
        let mut v = view("hidden");
        let p = patch2();
        v.sync(&p, &Settings::default());
        v.towers.push(Tower::default());
        let slot = v.towers[0].slot_pos(0);
        v.instances[0].t.pos = slot;
        v.selection.insert(0);
        let rect = eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(800.0, 600.0),
        );
        assert!(!v.compute_snap(rect).is_empty(), "a visible tower should catch it");
        v.set_hidden(&p, ElementRef::Tower(0), true);
        assert!(v.compute_snap(rect).is_empty(), "a hidden tower must not catch it");
        assert!(v.elements()[0].hidden);
        v.set_group_hidden(&p, 1, false);
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn rig_summary_counts_slots_and_loose_lights() {
        let mut v = view("summary");
        let p = patch2();
        v.sync(&p, &Settings::default());
        v.build(
            &p,
            Recipe::Straight { length: 3.0, height: 3.2, yaw: 0.0, grounded: true },
            Placement::Stagger,
        );
        v.build(&p, Recipe::default_for(RecipeKind::Tower), Placement::Stagger);
        v.instances[0].truss_mount = Some((0, 0));
        let s = v.rig_summary();
        assert_eq!((s.towers, s.trusses, s.groups), (1, 1, 0));
        assert_eq!(s.slots_total, 32);
        assert_eq!((s.slots_used, s.lights_loose), (1, 1));
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn import_rig_replaces_elements_and_keeps_light_positions() {
        let mut v = view("import_rig");
        let p = patch2();
        v.sync(&p, &Settings::default());
        v.build(&p, Recipe::default_for(RecipeKind::Straight), Placement::Stagger);
        v.instances[0].truss_mount = Some((0, 0));
        let was = v.instances[0].t.pos;
        let before = v.undo_stack.len();
        let lf = LayoutFile {
            instances: Vec::new(),
            towers: vec![Tower::default(), Tower::default()],
            trusses: Vec::new(),
        };
        v.import_rig(&p, lf);
        assert_eq!((v.towers.len(), v.trusses.len()), (2, 0));
        assert!(v.instances.iter().all(|i| i.mount.is_none() && i.truss_mount.is_none()));
        assert!(near(v.instances[0].t.pos.x, was.x) && near(v.instances[0].t.pos.z, was.z));
        assert_eq!(v.undo_stack.len(), before + 1);
        let _ = std::fs::remove_file(&v.layout_path);
    }

    #[test]
    fn merge_rig_renumbers_groups_and_spaces_elements() {
        let mut v = view("merge_rig");
        let p = patch0();
        v.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::At(v3(0.0, 0.0, 0.0)));
        let mut incoming = StageView::new();
        incoming.layout_path = std::env::temp_dir().join("dmxpress_builder_merge_src.json");
        let _ = std::fs::remove_file(&incoming.layout_path);
        incoming.build(&p, Recipe::default_for(RecipeKind::Goalpost), Placement::At(v3(0.0, 0.0, 0.0)));
        let lf = incoming.export_layout(&p);
        assert_eq!(v.merge_rig(&p, lf), 3);
        assert_eq!(v.group_ids(), vec![1, 2]);
        assert_eq!(v.group_members(2).len(), 3);
        let _ = std::fs::remove_file(&v.layout_path);
        let _ = std::fs::remove_file(&incoming.layout_path);
    }

    #[test]
    fn list_setup_infos_reads_counts() {
        let dir = std::env::temp_dir().join("dmxpress_setups_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut lf = LayoutFile { instances: Vec::new(), towers: vec![Tower::default()], trusses: Vec::new() };
        for k in 0..2 {
            lf.instances.push(super::super::layout::SavedInstance {
                key: format!("L{k}@1"),
                t: super::super::layout::LightTransform {
                    pos: v3(0.0, 0.0, 0.0),
                    yaw_deg: 0.0,
                    pitch_deg: 0.0,
                    roll_deg: 0.0,
                    scale: 1.0,
                },
                opacity: 1.0,
                mount: None,
                truss_mount: None,
            });
        }
        std::fs::write(dir.join("A.json"), serde_json::to_string(&lf).unwrap()).unwrap();
        std::fs::write(dir.join("broken.json"), "not json").unwrap();
        std::fs::write(dir.join("ignored.txt"), "hi").unwrap();
        let infos = list_setup_infos(&dir);
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].name, "A");
        assert_eq!((infos[0].towers, infos[0].trusses, infos[0].lights), (1, 0, 2));
        assert_eq!((infos[1].name.as_str(), infos[1].lights), ("broken", 0));
        assert!(infos[0].modified.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_layout_json_without_element_fields_still_loads() {
        let tw: Tower = serde_json::from_str(
            r#"{"pos":{"x":0,"y":0,"z":4},"yaw_deg":0,"height":3.2,"width":2.4}"#,
        )
        .unwrap();
        assert!(tw.name.is_empty() && tw.group.is_none() && !tw.hidden);
        let tr: Truss = serde_json::from_str(
            r#"{"kind":"Straight","pos":{"x":0,"y":3.2,"z":-4},"yaw_deg":0,"length":3,
                "radius":2,"arc_deg":90,"grounded":true}"#,
        )
        .unwrap();
        assert!(tr.name.is_empty() && tr.group.is_none() && !tr.hidden);
        assert!(near(tr.pitch_deg, 0.0) && near(tr.roll_deg, 0.0));
    }

    /// Touches the real `setups/` folder, so the names are unmistakable and
    /// both files go away again whatever happens.
    #[test]
    fn rename_setup_refuses_to_clobber() {
        let (a, b) = ("__builder_test_a", "__builder_test_b");
        let v = view("rename_setup");
        let p = patch0();
        let _ = std::fs::remove_file(StageView::setup_path(a));
        let _ = std::fs::remove_file(StageView::setup_path(b));
        assert!(v.save_setup_rig(&p, a));
        assert!(StageView::rename_setup(a, b));
        assert!(StageView::setup_infos().iter().any(|s| s.name == b));
        assert!(!StageView::setup_infos().iter().any(|s| s.name == a));
        assert!(!StageView::rename_setup(b, b), "renaming onto itself must fail");
        assert!(!StageView::rename_setup(a, b), "a missing source must fail");
        assert!(!StageView::rename_setup(b, "  "), "an empty name must fail");
        let read = StageView::read_setup(b).expect("the rig template reads back");
        assert!(read.instances.is_empty());
        StageView::delete_setup(a);
        StageView::delete_setup(b);
        assert!(!StageView::setup_path(b).exists());
    }

    /// The stage itself, with one of each composite built, so the geometry
    /// can be looked at rather than only asserted.
    #[test]
    fn composites_render_on_the_stage_headless() {
        let mut settings = Settings::default();
        let patch = patch0();
        let mut stage = view("composite_render");
        stage.build(&patch, Recipe::Goalpost { span: 5.0, height: 4.0, yaw: 0.0 }, Placement::At(v3(-7.0, 0.0, -3.0)));
        stage.build(
            &patch,
            Recipe::Box { width: 5.0, depth: 4.0, height: 4.5, yaw: 0.0, grounded: true },
            Placement::At(v3(0.0, 0.0, -3.0)),
        );
        stage.build(&patch, Recipe::Ring { radius: 2.0, height: 4.5 }, Placement::At(v3(7.0, 0.0, -3.0)));
        stage.build(&patch, Recipe::Arch { span: 5.0, rise: 3.5, yaw: 0.0 }, Placement::At(v3(-7.0, 0.0, 4.0)));
        stage.build(
            &patch,
            Recipe::TowerPair { spacing: 4.0, height: 3.2, width: 2.4, yaw: 0.0 },
            Placement::At(v3(0.0, 0.0, 4.0)),
        );
        stage.build(
            &patch,
            Recipe::Radius { radius: 2.5, arc: 180.0, height: 4.0, yaw: 0.0, grounded: false },
            Placement::At(v3(7.0, 0.0, 4.0)),
        );
        assert_eq!(stage.elements().len(), 14);
        // Look at the rig square on, with nothing picked, so the geometry
        // is what the picture shows.
        stage.sel_tower = None;
        stage.sel_truss = None;
        stage.show.help = false;
        stage.show.gizmo = false;
        stage.show.slot_rings = false;
        stage.show.grid = false;
        stage.cam.dist = 22.0;
        stage.cam.yaw = 0.0;
        stage.cam.pitch = -0.42;
        stage.cam.target = v3(0.0, 2.0, 0.0);
        let gobos = crate::gobo::Catalogue::default();
        let buf = [0u8; crate::net::DMX_SLOTS];
        let size = [1200, 760];
        let Some(pixels) = render_frames(3, size, |ctx, _| {
            eframe::egui::CentralPanel::default().show(ctx, |ui| {
                stage.ui(ui, &patch, &buf, &gobos, 600.0, &mut settings, None, None, None);
            });
        }) else {
            eprintln!("no GPU adapter — skipping");
            let _ = std::fs::remove_file(&stage.layout_path);
            return;
        };
        save(&pixels, size, "builder_composites_headless");
        let lit = pixels.chunks(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 120).count();
        assert!(lit > 1000, "the composites came out black");
        let _ = std::fs::remove_file(&stage.layout_path);
    }
}
