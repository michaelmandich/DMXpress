//! Plugins — sandboxed Rhai scripts that extend the console without a
//! recompile: paint their own mixer layer every frame, put up a small window
//! of controls, or tint the whole theme.
//!
//! A plugin is one `.rhai` file in `plugins/` with `//!` headers for its
//! name/author/blend and up to four well-known functions:
//!
//! - `controls()` — declares the widgets of the plugin's window as an array
//!   of maps (`kind` of `slider`, `checkbox`, `button` or `label`). The host
//!   renders them; the values live in a state map the script gets back.
//! - `frame(t, rig, state)` — called every tick while enabled; returns an
//!   array of `[address, value]` pairs (1-based DMX addresses) painted onto
//!   the plugin's own layer with the blend its header asked for.
//! - `button(id, t, state)` — a window button was pressed; returns the new
//!   state map (or nothing to keep it).
//! - `theme()` — a map of `#RRGGBB` strings (`accent`, `surface`, `text`)
//!   applied over the built-in look while the plugin is enabled.
//!
//! Scripts run inside Rhai's sandbox with an operation budget, so an
//! accidental `while true {}` aborts with an error instead of hanging the
//! desk. A script that errors has its hooks parked until Reload.

use std::path::{Path, PathBuf};

use rhai::{Array, Dynamic, Engine, Map, Scope, AST};
use serde::{Deserialize, Serialize};

use crate::engine::{Blend, Layer};
use crate::net::Frame;

/// Folder scanned for `*.rhai`, relative to the show directory.
pub const PLUGINS_DIR: &str = "plugins";
/// Which plugins are enabled, by file name.
const STATE_FILE: &str = "plugins.json";
/// Abort a script call after this many VM operations (runaway-loop guard).
const MAX_OPS: u64 = 250_000;
/// Where the Plugins window's "Get plugins" button points.
pub const SITE_PLUGINS_URL: &str = "https://dmxexpress.com/plugins.html";

/// One control in a plugin's window, declared by `controls()`.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    Slider { id: String, label: String, min: f32, max: f32, default: f32 },
    Checkbox { id: String, label: String, default: bool },
    Button { id: String, label: String },
    Label { text: String },
}

/// Theme tint from `theme()`: any subset of the keys, as RGB.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThemeOverride {
    pub accent: Option<[u8; 3]>,
    pub surface: Option<[u8; 3]>,
    pub text: Option<[u8; 3]>,
}

impl ThemeOverride {
    pub fn is_empty(&self) -> bool {
        self.accent.is_none() && self.surface.is_none() && self.text.is_none()
    }
}

/// `//!` header block at the top of a script.
#[derive(Debug, Clone)]
pub struct PluginMeta {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub blend: Blend,
}

pub struct Plugin {
    /// Full path of the `.rhai` file.
    pub file: PathBuf,
    pub meta: PluginMeta,
    pub enabled: bool,
    /// Last compile/runtime error, shown in the Plugins window.
    pub error: Option<String>,
    pub controls: Vec<Control>,
    /// Whether the script defines `frame` / `theme` (parked on error).
    pub has_frame: bool,
    pub has_theme: bool,
    /// The plugin's own window is showing.
    pub show_window: bool,
    /// Control values plus whatever the script stashes between calls.
    pub state: Map,
    pub theme: ThemeOverride,
    ast: Option<AST>,
}

#[derive(Serialize, Deserialize)]
struct SavedState {
    file: String,
    enabled: bool,
}

/// The sandboxed engine every plugin call runs through. `print` goes to the
/// app log via [`drain_prints`].
pub fn engine() -> Engine {
    let mut e = Engine::new();
    e.set_max_operations(MAX_OPS);
    e.set_max_expr_depths(48, 48);
    e.set_max_array_size(8_192);
    e.set_max_map_size(1_024);
    e.set_max_string_size(16_384);
    e.set_max_call_levels(24);
    e.on_print(|s| {
        let mut log = PRINTS.lock();
        if log.len() < 64 {
            log.push(s.to_string());
        }
    });
    // hsv(h 0..1, s 0..1, v 0..1) -> [r, g, b] 0..255, for colour maths.
    e.register_fn("hsv", |h: f64, s: f64, v: f64| -> Array {
        let (r, g, b) = hsv_rgb(h as f32, s as f32, v as f32);
        vec![
            Dynamic::from_int(r as i64),
            Dynamic::from_int(g as i64),
            Dynamic::from_int(b as i64),
        ]
    });
    e
}

static PRINTS: parking_lot::Mutex<Vec<String>> = parking_lot::Mutex::new(Vec::new());

/// Script `print` output since the last drain, prefixed for the app log.
pub fn drain_prints() -> Vec<String> {
    std::mem::take(&mut *PRINTS.lock())
}

fn hsv_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = (h.rem_euclid(1.0)) * 6.0;
    let i = h.floor();
    let f = h - i;
    let (s, v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    let c = |x: f32| (x * 255.0).round().clamp(0.0, 255.0) as u8;
    (c(r), c(g), c(b))
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// Parse the `//! key: value` header block. Unknown keys are ignored so the
/// format can grow without breaking older builds.
fn parse_meta(source: &str, file: &Path) -> PluginMeta {
    let mut meta = PluginMeta {
        name: file
            .file_stem()
            .map(|s| s.to_string_lossy().replace('_', " "))
            .unwrap_or_else(|| "plugin".into()),
        version: "0.1".into(),
        author: String::new(),
        description: String::new(),
        blend: Blend::Mix,
    };
    for line in source.lines() {
        let Some(rest) = line.trim_start().strip_prefix("//!") else {
            if line.trim().is_empty() || line.trim_start().starts_with("//") {
                continue;
            }
            break; // headers stop at the first code line
        };
        let Some((key, value)) = rest.split_once(':') else { continue };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "name" => meta.name = value.to_string(),
            "version" => meta.version = value.to_string(),
            "author" => meta.author = value.to_string(),
            "description" => meta.description = value.to_string(),
            "blend" => {
                meta.blend = match value.to_ascii_lowercase().as_str() {
                    "max" | "highest" => Blend::Max,
                    "add" => Blend::Add,
                    _ => Blend::Mix,
                }
            }
            _ => {}
        }
    }
    meta
}

fn parse_controls(list: Array) -> Vec<Control> {
    let mut out = Vec::new();
    for item in list {
        let Some(map) = item.try_cast::<Map>() else { continue };
        let get = |k: &str| -> Option<String> {
            map.get(k).map(|v| v.to_string()).filter(|s| !s.is_empty())
        };
        let num = |k: &str, d: f32| -> f32 {
            map.get(k)
                .and_then(|v| v.as_float().ok().map(|f| f as f32).or(v.as_int().ok().map(|i| i as f32)))
                .unwrap_or(d)
        };
        let kind = get("kind").unwrap_or_default();
        let id = get("id").unwrap_or_else(|| format!("c{}", out.len()));
        let label = get("label").unwrap_or_else(|| id.clone());
        match kind.as_str() {
            "slider" => {
                let min = num("min", 0.0);
                out.push(Control::Slider {
                    id,
                    label,
                    min,
                    max: num("max", 1.0),
                    // `default` is a reserved word in Rhai, so scripts say `value`.
                    default: num("value", min),
                });
            }
            "checkbox" => out.push(Control::Checkbox {
                id,
                label,
                default: map.get("value").and_then(|v| v.as_bool().ok()).unwrap_or(false),
            }),
            "button" => out.push(Control::Button { id, label }),
            "label" => out.push(Control::Label {
                text: get("text").unwrap_or(label),
            }),
            _ => {}
        }
    }
    out
}

impl Plugin {
    /// Compile `source`, probe its hooks and pull its declared controls.
    fn compile(engine: &Engine, file: PathBuf, source: &str) -> Self {
        let meta = parse_meta(source, &file);
        let mut plugin = Plugin {
            file,
            meta,
            enabled: false,
            error: None,
            controls: Vec::new(),
            has_frame: false,
            has_theme: false,
            show_window: false,
            state: Map::new(),
            theme: ThemeOverride::default(),
            ast: None,
        };
        let ast = match engine.compile(source) {
            Ok(ast) => ast,
            Err(e) => {
                plugin.error = Some(e.to_string());
                return plugin;
            }
        };
        let mut has_controls = false;
        let mut has_button = false;
        for f in ast.iter_functions() {
            match f.name {
                "frame" => plugin.has_frame = true,
                "theme" => plugin.has_theme = true,
                "controls" => has_controls = true,
                "button" => has_button = true,
                _ => {}
            }
        }
        let _ = has_button;
        plugin.ast = Some(ast);
        if has_controls {
            match plugin.call::<Array>(engine, "controls", ()) {
                Ok(list) => {
                    plugin.controls = parse_controls(list);
                    plugin.init_state();
                }
                Err(e) => plugin.error = Some(e),
            }
        }
        if plugin.has_theme {
            match plugin.call::<Map>(engine, "theme", ()) {
                Ok(map) => {
                    let hex = |k: &str| map.get(k).and_then(|v| parse_hex(&v.to_string()));
                    plugin.theme = ThemeOverride {
                        accent: hex("accent"),
                        surface: hex("surface"),
                        text: hex("text"),
                    };
                }
                Err(e) => {
                    plugin.has_theme = false;
                    plugin.error = Some(e);
                }
            }
        }
        plugin
    }

    /// Seed the state map with each control's default.
    fn init_state(&mut self) {
        for c in &self.controls {
            match c {
                Control::Slider { id, default, .. } => {
                    self.state
                        .entry(id.as_str().into())
                        .or_insert(Dynamic::from_float(*default as f64));
                }
                Control::Checkbox { id, default, .. } => {
                    self.state
                        .entry(id.as_str().into())
                        .or_insert(Dynamic::from_bool(*default));
                }
                _ => {}
            }
        }
    }

    fn call<T: Clone + 'static>(
        &self,
        engine: &Engine,
        name: &str,
        args: impl rhai::FuncArgs,
    ) -> Result<T, String> {
        let ast = self.ast.as_ref().ok_or("not compiled")?;
        let mut scope = Scope::new();
        engine
            .call_fn::<T>(&mut scope, ast, name, args)
            .map_err(|e| e.to_string())
    }

    /// Ask the script for this frame's channel writes; errors park the hook.
    pub fn frame_writes(&mut self, engine: &Engine, t: f64, rig: Array) -> Vec<(usize, u8)> {
        if !self.enabled || !self.has_frame {
            return Vec::new();
        }
        let state = self.state.clone();
        match self.call::<Array>(engine, "frame", (t, rig, state)) {
            Ok(list) => {
                let mut out = Vec::with_capacity(list.len());
                for pair in list {
                    let Some(pair) = pair.try_cast::<Array>() else { continue };
                    if pair.len() < 2 {
                        continue;
                    }
                    let addr = pair[0].as_int().unwrap_or(0);
                    let value = pair[1]
                        .as_int()
                        .unwrap_or_else(|_| pair[1].as_float().map(|f| f as i64).unwrap_or(0));
                    if addr >= 1 && addr as usize <= crate::net::DMX_SLOTS {
                        out.push(((addr - 1) as usize, value.clamp(0, 255) as u8));
                    }
                }
                out
            }
            Err(e) => {
                self.error = Some(format!("frame: {e}"));
                self.has_frame = false;
                Vec::new()
            }
        }
    }

    /// A window button was pressed. The script may return a new state map.
    pub fn press(&mut self, engine: &Engine, id: &str, t: f64) {
        let state = self.state.clone();
        match self.call::<Dynamic>(engine, "button", (id.to_string(), t, state)) {
            Ok(result) => {
                if let Some(map) = result.try_cast::<Map>() {
                    self.state = map;
                }
            }
            Err(e) => self.error = Some(format!("button: {e}")),
        }
    }

    /// The layer this plugin contributes this frame, if any.
    pub fn layer(&mut self, engine: &Engine, t: f64, rig: Array) -> Option<Layer> {
        let writes = self.frame_writes(engine, t, rig);
        if writes.is_empty() {
            return None;
        }
        let mut frame = Frame::default();
        let mut weights = Vec::with_capacity(writes.len());
        for (a, v) in writes {
            frame[a] = v;
            weights.push((a, 1.0));
        }
        Some(Layer::overlay(frame, weights).with_blend(self.meta.blend))
    }

    /// File name shown in the pool and stored in `plugins.json`.
    pub fn file_name(&self) -> String {
        self.file
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// Scan `plugins/` and compile everything, applying saved enabled flags.
pub fn load_all(engine: &Engine) -> Vec<Plugin> {
    let _ = std::fs::create_dir_all(PLUGINS_DIR);
    let saved: Vec<SavedState> = std::fs::read_to_string(STATE_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(PLUGINS_DIR) else {
        return out;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("rhai")))
        .collect();
    files.sort();
    for file in files {
        let Ok(source) = std::fs::read_to_string(&file) else { continue };
        let mut plugin = Plugin::compile(engine, file, &source);
        plugin.enabled = saved
            .iter()
            .find(|s| s.file == plugin.file_name())
            .map_or(false, |s| s.enabled);
        out.push(plugin);
    }
    out
}

/// Persist which plugins are enabled.
pub fn save_state(plugins: &[Plugin]) {
    let saved: Vec<SavedState> = plugins
        .iter()
        .map(|p| SavedState {
            file: p.file_name(),
            enabled: p.enabled,
        })
        .collect();
    if let Ok(text) = serde_json::to_string_pretty(&saved) {
        let _ = std::fs::write(STATE_FILE, text);
    }
}

/// Reload one plugin from disk in place, keeping its enabled flag and state.
pub fn reload(engine: &Engine, plugin: &mut Plugin) {
    let enabled = plugin.enabled;
    let show = plugin.show_window;
    let state = plugin.state.clone();
    match std::fs::read_to_string(&plugin.file) {
        Ok(source) => {
            *plugin = Plugin::compile(engine, plugin.file.clone(), &source);
            plugin.enabled = enabled;
            plugin.show_window = show;
            // Keep slider positions the user already set where ids survive.
            for (k, v) in state {
                plugin.state.insert(k, v);
            }
        }
        Err(e) => plugin.error = Some(format!("read: {e}")),
    }
}

/// The merged theme tint of every enabled plugin (later plugins win).
pub fn merged_theme(plugins: &[Plugin]) -> ThemeOverride {
    let mut out = ThemeOverride::default();
    for p in plugins.iter().filter(|p| p.enabled) {
        if p.theme.accent.is_some() {
            out.accent = p.theme.accent;
        }
        if p.theme.surface.is_some() {
            out.surface = p.theme.surface;
        }
        if p.theme.text.is_some() {
            out.text = p.theme.text;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r##"//! name: Test Wave
//! author: Tester
//! blend: highest
//! description: A test.

fn controls() {
    [
        #{ kind: "slider", id: "depth", label: "Depth", min: 0.0, max: 1.0 },
        #{ kind: "button", id: "boom", label: "Boom" },
    ]
}

fn frame(t, rig, state) {
    let out = [];
    for f in rig {
        if f.dimmer > 0 {
            out.push([f.dimmer, 200]);
        }
    }
    out
}

fn button(id, t, state) {
    state.pressed = id;
    state
}

fn theme() {
    #{ accent: "#FF2E88" }
}
"##;

    fn test_plugin() -> (Engine, Plugin) {
        let e = engine();
        let mut p = Plugin::compile(&e, PathBuf::from("test_wave.rhai"), SCRIPT);
        p.enabled = true;
        (e, p)
    }

    #[test]
    fn headers_and_hooks_parse() {
        let (_, p) = test_plugin();
        assert_eq!(p.meta.name, "Test Wave");
        assert_eq!(p.meta.author, "Tester");
        assert!(matches!(p.meta.blend, Blend::Max));
        assert!(p.has_frame);
        assert!(p.has_theme);
        assert_eq!(p.controls.len(), 2);
        assert_eq!(p.theme.accent, Some([0xFF, 0x2E, 0x88]));
        assert!(p.error.is_none(), "{:?}", p.error);
    }

    #[test]
    fn frame_returns_writes() {
        let (e, mut p) = test_plugin();
        let mut fixture = Map::new();
        fixture.insert("dimmer".into(), Dynamic::from_int(5));
        let rig: Array = vec![Dynamic::from_map(fixture)];
        let writes = p.frame_writes(&e, 0.0, rig);
        assert_eq!(writes, vec![(4, 200)]);
        assert!(p.error.is_none(), "{:?}", p.error);
    }

    #[test]
    fn button_updates_state() {
        let (e, mut p) = test_plugin();
        p.press(&e, "boom", 1.0);
        assert_eq!(p.state.get("pressed").map(|v| v.to_string()), Some("boom".into()));
    }

    #[test]
    fn runaway_scripts_are_stopped() {
        let e = engine();
        let mut p = Plugin::compile(
            &e,
            PathBuf::from("loop.rhai"),
            "fn frame(t, rig, state) { let x = 0; while true { x += 1; } [] }",
        );
        p.enabled = true;
        let writes = p.frame_writes(&e, 0.0, Array::new());
        assert!(writes.is_empty());
        assert!(p.error.is_some());
        assert!(!p.has_frame, "errored hook must be parked");
    }

    /// One synthetic fixture shaped like the maps `App::plugin_rig` builds.
    fn test_rig() -> Array {
        let mut m = Map::new();
        for (k, v) in [("i", 0i64), ("addr", 1), ("channels", 6), ("dimmer", 1), ("red", 2), ("green", 3), ("blue", 4), ("white", 0)] {
            m.insert(k.into(), Dynamic::from_int(v));
        }
        for k in ["x", "y", "z"] {
            m.insert(k.into(), Dynamic::from_float(1.0));
        }
        m.insert("name".into(), Dynamic::from("Test"));
        vec![Dynamic::from_map(m)]
    }

    #[test]
    fn bundled_samples_compile_and_run() {
        let e = engine();
        for (name, src) in [
            ("neon_night.rhai", include_str!("../plugins/neon_night.rhai")),
            ("breathing_floor.rhai", include_str!("../plugins/breathing_floor.rhai")),
            ("strobe_burst.rhai", include_str!("../plugins/strobe_burst.rhai")),
        ] {
            let mut p = Plugin::compile(&e, PathBuf::from(name), src);
            assert!(p.error.is_none(), "{name}: {:?}", p.error);
            p.enabled = true;
            if p.has_frame {
                let buttons: Vec<String> = p
                    .controls
                    .iter()
                    .filter_map(|c| match c {
                        Control::Button { id, .. } => Some(id.clone()),
                        _ => None,
                    })
                    .collect();
                for id in buttons {
                    p.press(&e, &id, 0.25);
                }
                let _ = p.frame_writes(&e, 0.5, test_rig());
                assert!(p.error.is_none(), "{name} frame: {:?}", p.error);
            }
        }
    }
}
