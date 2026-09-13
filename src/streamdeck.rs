//! Elgato Stream Deck control surface for the Palettes tab.
//!
//! The HID device lives on its own thread (same idea as `audio.rs`'s cpal
//! engine — the hardware handle isn't `Send`-friendly to share, so the UI
//! thread only ever posts the key images it wants and drains presses/encoder
//! turns through small, mutex-guarded queues). Whichever Stream Deck answers
//! first is used; if it's unplugged the thread quietly falls back to
//! rescanning until one reappears.

use std::collections::{HashMap, VecDeque};
use std::f32::consts::{PI, TAU};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont};
use elgato_streamdeck::images::convert_image_with_format;
use elgato_streamdeck::info::Kind;
use elgato_streamdeck::{list_devices, new_hidapi, StreamDeck, StreamDeckInput};
use image::{DynamicImage, RgbImage};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::chase::ChaseKind;
use crate::encoder::ClearStage;
use crate::palette::{hsv, Feature, Palette, SeqPattern};
use crate::showbuddy::PresetBank;
use crate::wheels::effect_islands;

/// Which page the deck is currently showing — cycled by the Mode knob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DeckPage {
    /// Page 1: the ShowBuddy General banks, OFF, and the rainbows — whole
    /// looks at one press.
    #[default]
    Presets,
    /// Palettes: colours, then (Page knob) gobos and prisms, picked into
    /// the palette cycle.
    Palettes,
    Phasers,
    Chases,
}

impl DeckPage {
    pub fn next(self) -> Self {
        match self {
            DeckPage::Presets => DeckPage::Palettes,
            DeckPage::Palettes => DeckPage::Phasers,
            DeckPage::Phasers => DeckPage::Chases,
            DeckPage::Chases => DeckPage::Presets,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DeckPage::Presets => "PRESETS",
            DeckPage::Palettes => "PALETTES",
            DeckPage::Phasers => "PHASERS",
            DeckPage::Chases => "CHASES",
        }
    }

    /// 1-based position in the Mode knob's cycle.
    pub fn number(self) -> usize {
        match self {
            DeckPage::Presets => 1,
            DeckPage::Palettes => 2,
            DeckPage::Phasers => 3,
            DeckPage::Chases => 4,
        }
    }

    pub const COUNT: usize = 4;
}

/// The Palettes page's sub-pages, flipped by the Page knob. Colours carry
/// the cycle's pattern/spacing/snap keys; Gobos lists every gobo in the
/// rig; Effects lists every other wheel and effect as islands, spilling
/// onto as many pages as the rig needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PaletteSub {
    #[default]
    Colors,
    Gobos,
    /// The n-th (0-based) page of effect islands.
    Effects(usize),
}

impl PaletteSub {
    pub fn name(self) -> &'static str {
        match self {
            PaletteSub::Colors => "COLORS",
            PaletteSub::Gobos => "GOBOS",
            PaletteSub::Effects(_) => "EFFECTS",
        }
    }

    /// 1-based position and total in the Page knob's cycle, given how many
    /// effect pages the rig fills (an empty rig still shows one).
    pub fn position(self, fx_pages: usize) -> (usize, usize) {
        let fx = fx_pages.max(1);
        let pos = match self {
            PaletteSub::Colors => 1,
            PaletteSub::Gobos => 2,
            PaletteSub::Effects(i) => 3 + i.min(fx - 1),
        };
        (pos, 2 + fx)
    }

    /// `delta` detents along the cycle, wrapping both ways.
    pub fn step(self, delta: i32, fx_pages: usize) -> Self {
        let (pos, count) = self.position(fx_pages);
        match (pos as i32 - 1 + delta).rem_euclid(count as i32) as usize {
            0 => PaletteSub::Colors,
            1 => PaletteSub::Gobos,
            n => PaletteSub::Effects(n - 2),
        }
    }

    /// Whether this sub-page lists wheel/effect islands.
    pub fn is_wheel(self) -> bool {
        !matches!(self, PaletteSub::Colors)
    }
}

/// Artwork drawn behind a key's symbol, as a watermark in a contrasting
/// shade of the key's own colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum KeyIcon {
    #[default]
    None,
    /// Concentric rings.
    Bullseye,
    /// Par can with a beam cone — a little stage light.
    StageLight,
    /// Horizontal double arrow (back and forth).
    LeftRight,
    /// Vertical double arrow (up and down).
    UpDown,
    /// A sine wave across the key.
    Wave,
    /// Radiating starburst.
    Burst,
    /// A single open ring.
    Ring,
    /// A rainbow arch, in real hues rather than the key's watermark shade.
    Rainbow,
}

impl KeyIcon {
    pub const ALL: [KeyIcon; 9] = [
        KeyIcon::None,
        KeyIcon::Bullseye,
        KeyIcon::StageLight,
        KeyIcon::LeftRight,
        KeyIcon::UpDown,
        KeyIcon::Wave,
        KeyIcon::Burst,
        KeyIcon::Ring,
        KeyIcon::Rainbow,
    ];

    pub fn label(self) -> &'static str {
        match self {
            KeyIcon::None => "No artwork",
            KeyIcon::Bullseye => "Bullseye",
            KeyIcon::StageLight => "Stage light",
            KeyIcon::LeftRight => "Back and forth",
            KeyIcon::UpDown => "Up and down",
            KeyIcon::Wave => "Wave",
            KeyIcon::Burst => "Burst",
            KeyIcon::Ring => "Ring",
            KeyIcon::Rainbow => "Rainbow",
        }
    }
}

/// One configured slot on the Phaser page: which phaser it toggles, and how
/// it's drawn (independent of the phaser's own pool colour, since a page
/// slot's colour/symbol/artwork is chosen when it's placed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaserSlot {
    pub phaser: String,
    pub color: [u8; 3],
    pub label: String,
    /// Absent in pages saved before key artwork existed.
    #[serde(default)]
    pub icon: KeyIcon,
}

/// Slots per Phaser page, one per deck key (36 for the Plus XL). The deck
/// is any number of these pages back to back — page `p`'s pads live at
/// `p * PHASER_DECK_SLOTS + slot` — and the Page knob scrolls through them.
pub const PHASER_DECK_SLOTS: usize = 36;
const PHASER_DECK_FILE: &str = "phaser_deck.json";

pub fn load_phaser_deck() -> Vec<Option<PhaserSlot>> {
    let mut slots: Vec<Option<PhaserSlot>> = std::fs::read_to_string(PHASER_DECK_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    pad_phaser_deck(&mut slots);
    slots
}

/// Rounds the deck up to whole pages (at least one), so every page's slots
/// are addressable and a file from before paging still loads as page 1.
pub fn pad_phaser_deck(slots: &mut Vec<Option<PhaserSlot>>) {
    slots.resize(phaser_deck_pages(slots) * PHASER_DECK_SLOTS, None);
}

/// How many pages of pads the deck holds (never fewer than one).
pub fn phaser_deck_pages(slots: &[Option<PhaserSlot>]) -> usize {
    slots.len().div_ceil(PHASER_DECK_SLOTS).max(1)
}

pub fn save_phaser_deck(slots: &[Option<PhaserSlot>]) {
    if let Ok(json) = serde_json::to_string_pretty(slots) {
        let _ = std::fs::write(PHASER_DECK_FILE, json);
    }
}

/// One key's desired appearance.
#[derive(Clone, PartialEq)]
pub struct KeySpec {
    pub index: u8,
    pub rgb: [u8; 3],
    pub label: Option<String>,
    pub icon: KeyIcon,
    /// A wheel or effect position to picture instead of `icon`: the
    /// island's name and the slot's name, drawn by [`draw_wheel_art`].
    pub wheel: Option<(String, String)>,
}

impl KeySpec {
    /// A plain coloured key with no artwork.
    fn plain(index: u8, rgb: [u8; 3], label: Option<String>) -> Self {
        Self { index, rgb, label, icon: KeyIcon::None, wheel: None }
    }
}

struct Shared {
    kind: Mutex<Option<Kind>>,
    pressed: Mutex<VecDeque<u8>>,
    encoder_pressed: Mutex<VecDeque<u8>>,
    encoder_delta: Mutex<Vec<i32>>,
    error: Mutex<Option<String>>,
}

enum Ctrl {
    SetKeys(Vec<KeySpec>),
    SetLcd { w: u32, h: u32, rgb: Vec<u8> },
    Clear,
}

/// Handle owned by the App; the HID device lives on the engine thread.
pub struct DeckEngine {
    shared: Arc<Shared>,
    ctrl: crossbeam_channel::Sender<Ctrl>,
}

impl DeckEngine {
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            kind: Mutex::new(None),
            pressed: Mutex::new(VecDeque::new()),
            encoder_pressed: Mutex::new(VecDeque::new()),
            encoder_delta: Mutex::new(Vec::new()),
            error: Mutex::new(None),
        });
        let (ctrl, ctrl_rx) = crossbeam_channel::unbounded();
        let thread_shared = shared.clone();
        std::thread::Builder::new()
            .name("streamdeck-engine".into())
            .spawn(move || engine_thread(thread_shared, ctrl_rx))
            .expect("spawn streamdeck engine");
        Self { shared, ctrl }
    }

    pub fn kind(&self) -> Option<Kind> {
        *self.shared.kind.lock()
    }

    pub fn device_label(&self) -> Option<String> {
        self.kind().map(|k| kind_label(k).to_string())
    }

    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().clone()
    }

    pub fn set_keys(&self, keys: Vec<KeySpec>) {
        let _ = self.ctrl.send(Ctrl::SetKeys(keys));
    }

    pub fn set_lcd(&self, w: u32, h: u32, rgb: Vec<u8>) {
        let _ = self.ctrl.send(Ctrl::SetLcd { w, h, rgb });
    }

    pub fn clear(&self) {
        let _ = self.ctrl.send(Ctrl::Clear);
    }

    pub fn take_presses(&self) -> Vec<u8> {
        self.shared.pressed.lock().drain(..).collect()
    }

    pub fn take_encoder_presses(&self) -> Vec<u8> {
        self.shared.encoder_pressed.lock().drain(..).collect()
    }

    pub fn take_encoder_deltas(&self) -> Vec<i32> {
        std::mem::take(&mut *self.shared.encoder_delta.lock())
    }
}

fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Original | Kind::OriginalV2 => "Stream Deck",
        Kind::Mini | Kind::MiniMk2 | Kind::MiniDiscord | Kind::MiniMk2Module => "Stream Deck Mini",
        Kind::Xl | Kind::XlV2 | Kind::XlV2Module => "Stream Deck XL",
        Kind::Mk2 | Kind::Mk2Scissor | Kind::Mk2Module => "Stream Deck MK2",
        Kind::Neo => "Stream Deck Neo",
        Kind::Pedal => "Stream Deck Pedal",
        Kind::Plus => "Stream Deck Plus",
        Kind::PlusXl => "Stream Deck Plus XL",
    }
}

/// Owns the HID handle; everything here stays on the engine thread.
fn engine_thread(shared: Arc<Shared>, ctrl: crossbeam_channel::Receiver<Ctrl>) {
    let hid = match new_hidapi() {
        Ok(h) => h,
        Err(e) => {
            *shared.error.lock() = Some(format!("HID subsystem unavailable: {e}"));
            return;
        }
    };
    let mut device: Option<StreamDeck> = None;
    let mut img_size: u32 = 72;
    let mut sent: Vec<KeySpec> = Vec::new();
    let mut last_lcd: Vec<u8> = Vec::new();
    let mut buttons: Vec<bool> = Vec::new();
    let mut enc_buttons: Vec<bool> = Vec::new();
    let mut last_scan = Instant::now() - Duration::from_secs(5);

    loop {
        // Only the newest desired frame matters for each channel; a Clear
        // wins outright over any pending key/LCD update.
        let mut latest_keys: Option<Vec<KeySpec>> = None;
        let mut latest_lcd: Option<(u32, u32, Vec<u8>)> = None;
        let mut do_clear = false;
        while let Ok(msg) = ctrl.try_recv() {
            match msg {
                Ctrl::SetKeys(k) => latest_keys = Some(k),
                Ctrl::SetLcd { w, h, rgb } => latest_lcd = Some((w, h, rgb)),
                Ctrl::Clear => do_clear = true,
            }
        }

        if device.is_none() {
            if last_scan.elapsed() >= Duration::from_secs(1) {
                last_scan = Instant::now();
                if let Some((kind, serial)) = list_devices(&hid).into_iter().next() {
                    match StreamDeck::connect(&hid, kind, &serial) {
                        Ok(d) => {
                            let _ = d.reset();
                            img_size = kind.key_image_format().size.0 as u32;
                            buttons = vec![false; kind.key_count() as usize];
                            enc_buttons = vec![false; kind.encoder_count() as usize];
                            sent.clear();
                            last_lcd.clear();
                            *shared.kind.lock() = Some(kind);
                            *shared.error.lock() = None;
                            device = Some(d);
                        }
                        Err(e) => *shared.error.lock() = Some(format!("{e}")),
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(150));
            continue;
        }

        let dev = device.as_ref().unwrap();

        if do_clear {
            let _ = dev.clear_all_button_images();
            let _ = dev.flush();
            sent.clear();
            if dev.kind() == Kind::PlusXl {
                let black = RgbImage::from_pixel(LCD_W, LCD_H, image::Rgb([0, 0, 0]));
                if let Some(fmt) = dev.kind().lcd_image_format() {
                    if let Ok(bytes) = convert_image_with_format(fmt, DynamicImage::ImageRgb8(black)) {
                        let _ = dev.write_lcd_fill(&bytes);
                    }
                }
            }
            last_lcd.clear();
        } else {
            if let Some(keys) = latest_keys {
                let mut wrote_any = false;
                let mut lost = false;
                for spec in &keys {
                    if sent.get(spec.index as usize) == Some(spec) {
                        continue;
                    }
                    if dev.set_button_image(spec.index, render_key(spec, img_size)).is_err() {
                        lost = true;
                        break;
                    }
                    wrote_any = true;
                }
                if lost {
                    device = None;
                    *shared.kind.lock() = None;
                    continue;
                }
                if wrote_any {
                    let _ = dev.flush();
                }
                sent = keys;
            }
            if let Some((w, h, rgb)) = latest_lcd {
                if rgb != last_lcd && dev.kind() == Kind::PlusXl {
                    if let Some(fmt) = dev.kind().lcd_image_format() {
                        if let Some(src) = RgbImage::from_raw(w, h, rgb.clone()) {
                            match convert_image_with_format(fmt, DynamicImage::ImageRgb8(src)) {
                                Ok(bytes) => {
                                    if dev.write_lcd_fill(&bytes).is_err() {
                                        device = None;
                                        *shared.kind.lock() = None;
                                        continue;
                                    }
                                    last_lcd = rgb;
                                }
                                Err(_) => {}
                            }
                        }
                    }
                }
            }
        }

        match dev.read_input(Some(Duration::from_millis(30))) {
            Ok(StreamDeckInput::ButtonStateChange(states)) => {
                for (i, &down) in states.iter().enumerate() {
                    let was = buttons.get(i).copied().unwrap_or(false);
                    if down && !was {
                        shared.pressed.lock().push_back(i as u8);
                    }
                }
                buttons = states;
            }
            Ok(StreamDeckInput::EncoderStateChange(states)) => {
                for (i, &down) in states.iter().enumerate() {
                    let was = enc_buttons.get(i).copied().unwrap_or(false);
                    if down && !was {
                        shared.encoder_pressed.lock().push_back(i as u8);
                    }
                }
                enc_buttons = states;
            }
            Ok(StreamDeckInput::EncoderTwist(deltas)) => {
                let mut acc = shared.encoder_delta.lock();
                if acc.len() < deltas.len() {
                    acc.resize(deltas.len(), 0);
                }
                for (i, d) in deltas.iter().enumerate() {
                    acc[i] += *d as i32;
                }
            }
            Ok(_) => {}
            Err(_) => {
                // Most likely unplugged mid-session; fall back to rescanning.
                device = None;
                *shared.kind.lock() = None;
            }
        }
    }
}

// -------------------------------------------------------------- rendering --

const GLYPH_W: usize = 5;
const GLYPH_H: usize = 7;

/// Built-in 5x7 dot font. Three pixels wide was not enough to tell M from N
/// or W from U — "FWD" came out as "FUD" — and these labels are the only
/// thing naming a key on the device, so they have to be unambiguous.
fn glyph(c: char) -> [u8; GLYPH_H] {
    match c {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        '%' => [0b11001, 0b11010, 0b00010, 0b00100, 0b01000, 0b01011, 0b10011],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        _ => [0; GLYPH_H],
    }
}

/// The typeface for keys and the strip: Ubuntu-Light, borrowed from the
/// fonts egui already ships, so nothing new is bundled. `None` only if that
/// lookup ever fails, in which case the bitmap font below takes over.
fn text_font() -> Option<&'static FontRef<'static>> {
    static FONT: OnceLock<Option<FontRef<'static>>> = OnceLock::new();
    FONT.get_or_init(|| {
        let defs = eframe::egui::FontDefinitions::default();
        let data = defs.font_data.get("Ubuntu-Light")?;
        let bytes: &'static [u8] = match &data.font {
            std::borrow::Cow::Borrowed(b) => b,
            std::borrow::Cow::Owned(v) => Box::leak(v.clone().into_boxed_slice()),
        };
        FontRef::try_from_slice(bytes).ok()
    })
    .as_ref()
}

/// Draws `text` centred in the box with anti-aliased glyphs, sized so the
/// capitals fill most of the box's height without overrunning its width.
fn draw_text_ttf(img: &mut RgbImage, font: &FontRef, x0: i32, y0: i32, x1: i32, y1: i32, text: &str, fg: [u8; 3]) {
    let (bw, bh) = ((x1 - x0) as f32, (y1 - y0) as f32);
    if bw <= 0.0 || bh <= 0.0 || text.is_empty() {
        return;
    }
    // Measure at a reference size, then scale to fit.
    let probe = font.as_scaled(PxScale::from(100.0));
    let mut w100 = 0.0;
    let mut prev: Option<GlyphId> = None;
    for c in text.chars() {
        let id = font.glyph_id(c);
        if let Some(p) = prev {
            w100 += probe.kern(p, id);
        }
        w100 += probe.h_advance(id);
        prev = Some(id);
    }
    // Ubuntu's cap height is about three quarters of its ascent.
    let cap100 = probe.ascent() * 0.744;
    let px = (100.0 * bw * 0.96 / w100.max(1.0)).min(100.0 * bh * 0.80 / cap100).max(4.0);
    let scaled = font.as_scaled(PxScale::from(px));
    let width = w100 * px / 100.0;
    let cap = cap100 * px / 100.0;
    let baseline = y0 as f32 + (bh + cap) * 0.5;
    let mut x = x0 as f32 + (bw - width) * 0.5;
    prev = None;
    for c in text.chars() {
        let id = font.glyph_id(c);
        if let Some(p) = prev {
            x += scaled.kern(p, id);
        }
        let glyph = id.with_scale_and_position(PxScale::from(px), ab_glyph::point(x, baseline));
        if let Some(og) = font.outline_glyph(glyph) {
            let b = og.px_bounds();
            og.draw(|gx, gy, cov| {
                blend_px(img, b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, fg, cov);
            });
        }
        x += scaled.h_advance(id);
        prev = Some(id);
    }
}

/// Draws `text` centred inside the `[x0,y0)..(x1,y1)` box, scaled to fill
/// most of it — smooth glyphs when the typeface is available, the built-in
/// dot font otherwise.
fn draw_label_in(img: &mut RgbImage, x0: i32, y0: i32, x1: i32, y1: i32, text: &str, fg: [u8; 3]) {
    match text_font() {
        Some(font) => draw_text_ttf(img, font, x0, y0, x1, y1, text, fg),
        None => draw_label_bitmap(img, x0, y0, x1, y1, text, fg),
    }
}

/// The dot-font fallback (pixels are still clipped to the whole image's bounds).
fn draw_label_bitmap(img: &mut RgbImage, x0: i32, y0: i32, x1: i32, y1: i32, text: &str, fg: [u8; 3]) {
    let (img_w, img_h) = (img.width() as i32, img.height() as i32);
    let (bw, bh) = (x1 - x0, y1 - y0);
    let n = text.chars().count().max(1) as i32;
    let cell = (bw / (n * (GLYPH_W as i32 + 1))).min(bh / (GLYPH_H as i32 + 2)).max(1);
    let total_w = n * (GLYPH_W as i32 + 1) * cell;
    let start_x = x0 + (bw - total_w) / 2;
    let start_y = y0 + (bh - GLYPH_H as i32 * cell) / 2;
    for (gi, c) in text.chars().enumerate() {
        let bits = glyph(c.to_ascii_uppercase());
        let gx = start_x + gi as i32 * (GLYPH_W as i32 + 1) * cell;
        for (row, bits_row) in bits.iter().enumerate() {
            for col in 0..GLYPH_W {
                if bits_row & (1 << (GLYPH_W - 1 - col)) == 0 {
                    continue;
                }
                let px0 = gx + col as i32 * cell;
                let py0 = start_y + row as i32 * cell;
                for dy in 0..cell {
                    for dx in 0..cell {
                        let (x, y) = (px0 + dx, py0 + dy);
                        if x >= 0 && y >= 0 && x < img_w && y < img_h {
                            img.put_pixel(x as u32, y as u32, image::Rgb(fg));
                        }
                    }
                }
            }
        }
    }
}

/// Draws `text` centred on the whole image in `fg`, scaled to fill most of
/// it. A small margin is kept so four-character tags don't run into the
/// bezel — edge-to-edge text is hard to read on a lit key.
fn draw_label(img: &mut RgbImage, text: &str, fg: [u8; 3]) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let inset = (w / 9).max(2);
    draw_label_in(img, inset, inset, w - inset, h - inset, text, fg);
}

/// The dark deck showing through a key's rounded corners.
const KEY_GAP: [u8; 3] = [10, 10, 13];

fn fill_round_rect(img: &mut RgbImage, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, color: [u8; 3]) {
    let r = r.min((x1 - x0) * 0.5).min((y1 - y0) * 0.5).max(0.0);
    fill_rect(img, x0 + r, y0, x1 - r, y1, color);
    fill_rect(img, x0, y0 + r, x1, y1 - r, color);
    for (cx, cy) in [(x0 + r, y0 + r), (x1 - r, y0 + r), (x0 + r, y1 - r), (x1 - r, y1 - r)] {
        fill_circle(img, cx, cy, r, color);
    }
}

/// A key's face: a rounded plate with a contrasting rim and a darker inner
/// line, so every key reads as a button rather than a flat square. Painted
/// first, over the bare gap colour, so the art and label go on top.
fn draw_key_face(img: &mut RgbImage, bg: [u8; 3]) {
    let w = img.width() as f32;
    let (rim_t, line_t, radius) = (w * 0.045, w * 0.018, w * 0.16);
    let rim = lerp_rgb(bg, readable_on(bg), 0.55);
    let line = lerp_rgb(bg, [0, 0, 0], 0.45);
    fill_round_rect(img, 0.0, 0.0, w, w, radius, rim);
    fill_round_rect(img, rim_t, rim_t, w - rim_t, w - rim_t, radius - rim_t, line);
    let inset = rim_t + line_t;
    fill_round_rect(img, inset, inset, w - inset, w - inset, radius - inset, bg);
}

fn render_key(spec: &KeySpec, size: u32) -> DynamicImage {
    // Drawn at `SS` times the key size and filtered down, like the strip.
    let big = size * SS;
    let mut img = RgbImage::from_pixel(big, big, image::Rgb(KEY_GAP));
    draw_key_face(&mut img, spec.rgb);
    let s = big as f32;
    let art = spec.icon != KeyIcon::None || spec.wheel.is_some();
    if art {
        // A watermark shade of the key's own colour, so the artwork reads as
        // a backing rather than competing with the symbol on top of it.
        let ink = lerp_rgb(spec.rgb, readable_on(spec.rgb), 0.38);
        // Sits high when a symbol needs the bottom strip, centred otherwise.
        let (cy, r) = if spec.label.is_some() { (0.42 * s, 0.30 * s) } else { (0.5 * s, 0.36 * s) };
        match &spec.wheel {
            Some((island, name)) => draw_wheel_art(&mut img, island, name, 0.5 * s, cy, r, ink, spec.rgb),
            None => draw_key_icon(&mut img, spec.icon, 0.5 * s, cy, r, ink, spec.rgb),
        }
    }
    if let Some(label) = &spec.label {
        let fg = readable_on(spec.rgb);
        if art {
            let (x0, x1) = ((0.10 * s) as i32, (0.90 * s) as i32);
            draw_label_in(&mut img, x0, (0.66 * s) as i32, x1, (0.92 * s) as i32, label, fg);
        } else if let Some((top, bottom)) = label.split_once('\n') {
            // Two lines, for an island header written out in full.
            let (x0, x1) = ((0.08 * s) as i32, (0.92 * s) as i32);
            draw_label_in(&mut img, x0, (0.16 * s) as i32, x1, (0.48 * s) as i32, top, fg);
            draw_label_in(&mut img, x0, (0.52 * s) as i32, x1, (0.84 * s) as i32, bottom, fg);
        } else {
            draw_label(&mut img, label, fg);
        }
    }
    DynamicImage::ImageRgb8(downsample(&img, SS))
}

/// Desaturated/dimmed version of a colour, for a key that isn't "on" (a
/// Cycle colour not currently selected, or a phaser pad that isn't running)
/// — keeps a hint of the hue so keys stay distinguishable.
fn mute_rgb(c: [u8; 3]) -> [u8; 3] {
    let mix = |ch: u8| ((ch as u16 * 35 + 40 * 65) / 100) as u8;
    [mix(c[0]), mix(c[1]), mix(c[2])]
}

// ---------------------------------------------------------- LCD strip icons

/// The Stream Deck Plus XL's touch strip, in the "visual" (pre-rotation)
/// orientation the crate expects — see `convert_image_with_format`: feeding
/// it a 1200x100 image lets it rotate+encode straight into the device's raw
/// layout with no resampling. Six 200px-wide panels, left to right, one per
/// encoder; only the first four (Dimmer/Beat/Mode/Page) are wired up so far.
const LCD_W: u32 = 1200;
const LCD_H: u32 = 100;
const PANEL_W: f32 = 200.0;

fn fill_rect(img: &mut RgbImage, x0: f32, y0: f32, x1: f32, y1: f32, color: [u8; 3]) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let xs = (x0.floor() as i32).max(0);
    let xe = (x1.ceil() as i32).min(w - 1);
    let ys = (y0.floor() as i32).max(0);
    let ye = (y1.ceil() as i32).min(h - 1);
    for y in ys..=ye {
        for x in xs..=xe {
            img.put_pixel(x as u32, y as u32, image::Rgb(color));
        }
    }
}

fn fill_ellipse(img: &mut RgbImage, cx: f32, cy: f32, rx: f32, ry: f32, color: [u8; 3]) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let x0 = ((cx - rx).floor() as i32).max(0);
    let x1 = ((cx + rx).ceil() as i32).min(w - 1);
    let y0 = ((cy - ry).floor() as i32).max(0);
    let y1 = ((cy + ry).ceil() as i32).min(h - 1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 + 0.5 - cx) / rx.max(0.001);
            let dy = (y as f32 + 0.5 - cy) / ry.max(0.001);
            if dx * dx + dy * dy <= 1.0 {
                img.put_pixel(x as u32, y as u32, image::Rgb(color));
            }
        }
    }
}

fn fill_circle(img: &mut RgbImage, cx: f32, cy: f32, r: f32, color: [u8; 3]) {
    fill_ellipse(img, cx, cy, r, r, color);
}

fn lerp_rgb(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2])]
}

/// Split a preset name across at most two lines at a space, so it can be
/// drawn large enough to read on a 200px panel. Falls back to one line
/// (truncated) when there is nowhere sensible to break.
fn split_label(name: &str) -> (String, Option<String>) {
    let up = name.to_uppercase();
    if up.chars().count() <= 7 {
        return (up, None);
    }
    // Break at whichever space sits closest to the middle, so "Warm Wash 3"
    // splits as WARM / WASH 3 rather than stranding the 3 on its own line.
    let mid = up.len() as i64 / 2;
    let best = up
        .match_indices(' ')
        .min_by_key(|(i, _)| (*i as i64 - mid).abs())
        .map(|(i, _)| i);
    if let Some(at) = best {
        let (a, b) = up.split_at(at);
        let b = b.trim_start();
        if !a.is_empty() && !b.is_empty() {
            return (a.chars().take(9).collect(), Some(b.chars().take(9).collect()));
        }
    }
    (up.chars().take(9).collect(), None)
}

/// Dark or light text, whichever reads on top of `bg`.
fn readable_on(bg: [u8; 3]) -> [u8; 3] {
    let lum = 0.299 * bg[0] as f32 + 0.587 * bg[1] as f32 + 0.114 * bg[2] as f32;
    if lum > 140.0 { [12, 12, 12] } else { [235, 235, 235] }
}

// ------------------------------------------------------------ lcd artwork --
//
// The touch strip is where the deck earns its looks, so this is a proper art
// pass rather than flat fills: tie-dye backgrounds, outlined silhouettes,
// glow, and supersampled edges. Everything paints into a canvas `SS` times
// the strip's size and box-filters down — at 100px tall, that's the
// difference between pixel staircases and smooth curves.
//
// The motifs (sun, bolt, rose, dancing bear, terrapin, skull-and-rainbow)
// are original silhouettes in the Dead's visual vocabulary, not tracings of
// the band's own trademarked designs.

/// Supersampling factor for the strip.
const SS: u32 = 2;

/// Everything one frame of the strip needs, separated from `App` so the
/// painting can be exercised and previewed without a running show.
pub(crate) struct StripState {
    /// Grand master, 0..1.
    pub level: f32,
    pub blackout: bool,
    pub bpm: i32,
    /// Beat flash, 0..1 — 1 right on the beat, decaying to 0.
    pub pulse: f32,
    pub frozen: bool,
    pub page: DeckPage,
    /// `Some` when the fourth panel is naming the chase's inject preset
    /// (Chases page); the inner value is the name, if one is picked.
    pub inject: Option<Option<String>>,
    /// The programmer knob's readout — (channel name, value) — or `None`
    /// with nothing selected.
    pub encoder: Option<(String, String)>,
    /// `Some` where the Page knob scrolls something: the Palettes page's
    /// sub-page, or the Phaser page's page of pads — a name, then its
    /// position and count in the knob's cycle.
    pub sub: Option<(&'static str, usize, usize)>,
}

fn blend_px(img: &mut RgbImage, x: i32, y: i32, rgb: [u8; 3], a: f32) {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
        return;
    }
    let a = a.clamp(0.0, 1.0);
    let p = img.get_pixel_mut(x as u32, y as u32);
    for k in 0..3 {
        p.0[k] = (p.0[k] as f32 * (1.0 - a) + rgb[k] as f32 * a).round() as u8;
    }
}

/// Even-odd scanline fill, so concave outlines like a lightning bolt work.
fn fill_poly(img: &mut RgbImage, pts: &[(f32, f32)], color: [u8; 3]) {
    if pts.len() < 3 {
        return;
    }
    let (w, h) = (img.width() as i32, img.height() as i32);
    let y0 = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
    let y1 = (pts.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil() as i32).min(h - 1);
    let mut xs: Vec<f32> = Vec::with_capacity(8);
    for y in y0..=y1 {
        let sy = y as f32 + 0.5;
        xs.clear();
        for i in 0..pts.len() {
            let (ax, ay) = pts[i];
            let (bx, by) = pts[(i + 1) % pts.len()];
            if (ay <= sy) != (by <= sy) {
                xs.push(ax + (sy - ay) / (by - ay) * (bx - ax));
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in xs.chunks(2) {
            if let [a, b] = pair {
                let (xa, xb) = ((a.round() as i32).max(0), (b.round() as i32).min(w));
                for x in xa..xb {
                    img.put_pixel(x as u32, y as u32, image::Rgb(color));
                }
            }
        }
    }
}

/// Soft radial light that falls off to nothing at `r`.
fn glow(img: &mut RgbImage, cx: f32, cy: f32, r: f32, color: [u8; 3], strength: f32) {
    if strength <= 0.0 || r <= 0.0 {
        return;
    }
    let (w, h) = (img.width() as i32, img.height() as i32);
    let x0 = ((cx - r).floor() as i32).max(0);
    let x1 = ((cx + r).ceil() as i32).min(w - 1);
    let y0 = ((cy - r).floor() as i32).max(0);
    let y1 = ((cy + r).ceil() as i32).min(h - 1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt() / r;
            if d < 1.0 {
                blend_px(img, x, y, color, strength * (1.0 - d).powi(2));
            }
        }
    }
}

/// A thick line with round ends — limbs, stems, frost arms.
fn capsule(img: &mut RgbImage, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, color: [u8; 3]) {
    let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    let steps = (len / (r * 0.5).max(0.5)).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        fill_circle(img, x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, r, color);
    }
}

fn blit(dst: &mut RgbImage, src: &RgbImage, ox: u32, oy: u32) {
    let (dw, dh) = (dst.width() as usize, dst.height() as usize);
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (ox, oy) = (ox as usize, oy as usize);
    let w = sw.min(dw.saturating_sub(ox));
    let sraw = src.as_raw();
    let draw: &mut [u8] = &mut **dst;
    for y in 0..sh {
        let dy = oy + y;
        if dy >= dh || w == 0 {
            break;
        }
        let (s0, d0) = (y * sw * 3, (dy * dw + ox) * 3);
        draw[d0..d0 + w * 3].copy_from_slice(&sraw[s0..s0 + w * 3]);
    }
}

/// One panel's tie-dye backdrop: a spiral swirl crossed with rings, hues
/// running `span` degrees from `hue0`, kept dim enough (`bright`) that the
/// white text and glowing shapes on top still win. Cached — the per-pixel
/// trig is the one expensive thing here, and a backdrop only depends on
/// these three numbers.
fn tie_dye_panel(hue0: f32, span: f32, bright: f32) -> Arc<RgbImage> {
    static CACHE: OnceLock<Mutex<HashMap<(i32, i32, i32), Arc<RgbImage>>>> = OnceLock::new();
    let key = (hue0.round() as i32, span.round() as i32, (bright * 100.0).round() as i32);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(p) = cache.lock().get(&key) {
        return p.clone();
    }
    let (w, h) = (PANEL_W as u32 * SS, LCD_H * SS);
    let mut img = RgbImage::new(w, h);
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f32 + 0.5 - cx) / (w as f32 * 0.5);
            let dy = (y as f32 + 0.5 - cy) / (h as f32 * 0.5);
            let r = (dx * dx + dy * dy).sqrt();
            let th = dy.atan2(dx);
            let swirl = (th * 3.0 + r * 7.0).sin() * 0.5 + 0.5;
            let ripple = (r * 16.0 - th * 2.0).sin() * 0.5 + 0.5;
            let hue = hue0 + span * (swirl * 0.65 + ripple * 0.35);
            let band = 0.72 + 0.28 * ripple;
            let vignette = 1.0 - 0.45 * r.min(1.4) / 1.4;
            img.put_pixel(x, y, image::Rgb(hsv(hue, 0.88, bright * band * vignette)));
        }
    }
    let arc = Arc::new(img);
    cache.lock().insert(key, arc.clone());
    arc
}

/// Text with a dark stroke around it, so it reads on any tie-dye.
fn outlined_label(img: &mut RgbImage, x0: i32, y0: i32, x1: i32, y1: i32, text: &str, fg: [u8; 3], ow: i32) {
    let ink = [10, 8, 14];
    for (dx, dy) in [(-ow, 0), (ow, 0), (0, -ow), (0, ow), (-ow, -ow), (ow, -ow), (-ow, ow), (ow, ow)] {
        draw_label_in(img, x0 + dx, y0 + dy, x1 + dx, y1 + dy, text, ink);
    }
    draw_label_in(img, x0, y0, x1, y1, text, fg);
}

/// Draw a shape in dark ink at eight offsets, then for real on top — a cheap
/// stroke that works for any composite of fills. `mono` tells the shape to
/// paint everything in one colour (the outline passes).
fn outlined(img: &mut RgbImage, ow: f32, draw: impl Fn(&mut RgbImage, f32, f32, Option<[u8; 3]>)) {
    let ink = [12, 8, 16];
    for (dx, dy) in [
        (-1.0, 0.0),
        (1.0, 0.0),
        (0.0, -1.0),
        (0.0, 1.0),
        (-0.7, -0.7),
        (0.7, -0.7),
        (-0.7, 0.7),
        (0.7, 0.7),
    ] {
        draw(img, dx * ow, dy * ow, Some(ink));
    }
    draw(img, 0.0, 0.0, None);
}

/// Box-filter the supersampled canvas back down to panel resolution.
fn downsample(src: &RgbImage, f: u32) -> RgbImage {
    let f = f as usize;
    let sw = src.width() as usize;
    let (w, h) = (sw / f, src.height() as usize / f);
    let raw = src.as_raw();
    let mut out = vec![0u8; w * h * 3];
    let n = (f * f) as u32;
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 3];
            for dy in 0..f {
                let row = (y * f + dy) * sw * 3;
                for dx in 0..f {
                    let i = row + (x * f + dx) * 3;
                    acc[0] += raw[i] as u32;
                    acc[1] += raw[i + 1] as u32;
                    acc[2] += raw[i + 2] as u32;
                }
            }
            let o = (y * w + x) * 3;
            out[o] = (acc[0] / n) as u8;
            out[o + 1] = (acc[1] / n) as u8;
            out[o + 2] = (acc[2] / n) as u8;
        }
    }
    RgbImage::from_raw(w as u32, h as u32, out).expect("downsample buffer sized to fit")
}

// --- the motifs -----------------------------------------------------------

/// A blazing sun with twelve alternating rays. `level` runs it from a dim
/// ember up to full white-hot.
fn draw_sun(img: &mut RgbImage, cx: f32, cy: f32, size: f32, level: f32, mono: Option<[u8; 3]>) {
    let core = mono.unwrap_or(lerp_rgb([150, 70, 25], [255, 235, 90], level));
    let ray = mono.unwrap_or(lerp_rgb([120, 55, 20], [255, 190, 50], level));
    for k in 0..12 {
        let a = k as f32 * TAU / 12.0;
        let tip = size * if k % 2 == 0 { 1.0 } else { 0.78 };
        let base = size * 0.46;
        let w = 0.16;
        fill_tri(
            img,
            (cx + (a - w).cos() * base, cy + (a - w).sin() * base),
            (cx + (a + w).cos() * base, cy + (a + w).sin() * base),
            (cx + a.cos() * tip, cy + a.sin() * tip),
            ray,
        );
    }
    fill_circle(img, cx, cy, size * 0.42, core);
    if mono.is_none() {
        fill_circle(img, cx, cy, size * 0.26, lerp_rgb(core, [255, 255, 220], 0.5 * level));
    }
}

/// The sun gone dark: a black disc with a corona — what blackout looks like.
fn draw_eclipse(img: &mut RgbImage, cx: f32, cy: f32, size: f32) {
    glow(img, cx, cy, size * 1.3, [255, 200, 120], 0.9);
    let corona = [255, 215, 150];
    for k in 0..16 {
        let a = k as f32 * TAU / 16.0 + 0.1;
        let (r0, r1) = (size * 0.66, size * if k % 2 == 0 { 0.98 } else { 0.84 });
        capsule(img, cx + a.cos() * r0, cy + a.sin() * r0, cx + a.cos() * r1, cy + a.sin() * r1, size * 0.03, corona);
    }
    fill_circle(img, cx, cy, size * 0.62, [8, 6, 10]);
}

/// A lightning bolt, `flash` lifting it from deep violet to white.
fn draw_bolt(img: &mut RgbImage, cx: f32, cy: f32, size: f32, flash: f32, mono: Option<[u8; 3]>) {
    const PTS: [(f32, f32); 7] =
        [(0.55, 0.0), (0.75, 0.0), (0.55, 0.40), (0.78, 0.40), (0.35, 1.0), (0.48, 0.55), (0.22, 0.55)];
    let (w, h) = (size * 1.15, size * 2.0);
    let poly: Vec<(f32, f32)> =
        PTS.iter().map(|(x, y)| (cx - w * 0.5 + x * w, cy - h * 0.5 + y * h)).collect();
    let fill = mono.unwrap_or(lerp_rgb([125, 60, 205], [255, 255, 255], flash));
    fill_poly(img, &poly, fill);
}

/// Six-petalled rose, leaves and stem.
fn draw_rose(img: &mut RgbImage, cx: f32, cy: f32, size: f32, mono: Option<[u8; 3]>) {
    let petal = mono.unwrap_or([215, 40, 70]);
    let inner = mono.unwrap_or([150, 20, 50]);
    let heart = mono.unwrap_or([90, 10, 35]);
    let leaf = mono.unwrap_or([50, 140, 60]);
    let stem = mono.unwrap_or([40, 110, 50]);
    let bloom_y = cy - size * 0.12;
    capsule(img, cx, bloom_y + size * 0.20, cx, cy + size * 0.86, size * 0.05, stem);
    fill_ellipse(img, cx - size * 0.22, cy + size * 0.58, size * 0.20, size * 0.09, leaf);
    fill_ellipse(img, cx + size * 0.22, cy + size * 0.76, size * 0.20, size * 0.09, leaf);
    for k in 0..6 {
        let a = k as f32 * TAU / 6.0;
        fill_circle(img, cx + a.cos() * size * 0.30, bloom_y + a.sin() * size * 0.30, size * 0.24, petal);
    }
    for k in 0..6 {
        let a = k as f32 * TAU / 6.0 + TAU / 12.0;
        fill_circle(img, cx + a.cos() * size * 0.16, bloom_y + a.sin() * size * 0.16, size * 0.17, inner);
    }
    fill_circle(img, cx, bloom_y, size * 0.11, heart);
}

/// A bear mid-dance — arms up, one leg kicked — in an original pose.
fn draw_bear(img: &mut RgbImage, cx: f32, cy: f32, size: f32, mono: Option<[u8; 3]>) {
    let fur = mono.unwrap_or([120, 78, 40]);
    let light = mono.unwrap_or([170, 120, 70]);
    let dark = mono.unwrap_or([40, 25, 15]);
    capsule(img, cx - size * 0.12, cy + size * 0.30, cx - size * 0.22, cy + size * 0.62, size * 0.10, fur);
    capsule(img, cx + size * 0.12, cy + size * 0.30, cx + size * 0.34, cy + size * 0.52, size * 0.10, fur);
    capsule(img, cx - size * 0.20, cy - size * 0.05, cx - size * 0.46, cy - size * 0.42, size * 0.09, fur);
    capsule(img, cx + size * 0.20, cy - size * 0.05, cx + size * 0.46, cy - size * 0.42, size * 0.09, fur);
    fill_ellipse(img, cx, cy + size * 0.10, size * 0.28, size * 0.32, fur);
    if mono.is_none() {
        fill_ellipse(img, cx, cy + size * 0.16, size * 0.16, size * 0.20, light);
    }
    fill_circle(img, cx - size * 0.16, cy - size * 0.44, size * 0.09, fur);
    fill_circle(img, cx + size * 0.16, cy - size * 0.44, size * 0.09, fur);
    fill_circle(img, cx, cy - size * 0.32, size * 0.20, fur);
    if mono.is_none() {
        fill_ellipse(img, cx, cy - size * 0.26, size * 0.10, size * 0.07, light);
        fill_circle(img, cx - size * 0.07, cy - size * 0.36, size * 0.025, dark);
        fill_circle(img, cx + size * 0.07, cy - size * 0.36, size * 0.025, dark);
        fill_circle(img, cx, cy - size * 0.28, size * 0.03, dark);
    }
}

/// A terrapin marching to the right.
fn draw_terrapin(img: &mut RgbImage, cx: f32, cy: f32, size: f32, mono: Option<[u8; 3]>) {
    let shell = mono.unwrap_or([60, 130, 70]);
    let scute = mono.unwrap_or([35, 95, 50]);
    let skin = mono.unwrap_or([150, 170, 80]);
    let belly = mono.unwrap_or([200, 190, 120]);
    let dark = mono.unwrap_or([30, 30, 20]);
    for (x0, x1) in [(-0.30, -0.42), (-0.10, -0.16), (0.20, 0.32), (0.40, 0.48)] {
        capsule(img, cx + size * x0, cy + size * 0.12, cx + size * x1, cy + size * 0.42, size * 0.08, skin);
    }
    fill_tri(
        img,
        (cx - size * 0.50, cy + size * 0.05),
        (cx - size * 0.50, cy + size * 0.20),
        (cx - size * 0.72, cy + size * 0.16),
        skin,
    );
    fill_circle(img, cx + size * 0.62, cy - size * 0.02, size * 0.15, skin);
    if mono.is_none() {
        fill_circle(img, cx + size * 0.68, cy - size * 0.06, size * 0.03, dark);
    }
    fill_ellipse(img, cx, cy - size * 0.02, size * 0.55, size * 0.36, shell);
    fill_ellipse(img, cx, cy + size * 0.16, size * 0.58, size * 0.10, belly);
    if mono.is_none() {
        for (px, py, r) in [(0.0, -0.10, 0.13), (-0.28, -0.02, 0.10), (0.28, -0.02, 0.10), (-0.14, -0.24, 0.08), (0.14, -0.24, 0.08)] {
            fill_circle(img, cx + size * px, cy + size * py, size * r, scute);
        }
    }
}

/// A skull, with a rainbow arcing over it.
fn draw_skull(img: &mut RgbImage, cx: f32, cy: f32, size: f32, mono: Option<[u8; 3]>) {
    let bone = mono.unwrap_or([235, 228, 210]);
    let shade = mono.unwrap_or([180, 170, 150]);
    let hole = mono.unwrap_or([18, 14, 22]);
    fill_circle(img, cx, cy - size * 0.12, size * 0.44, bone);
    fill_rect(img, cx - size * 0.30, cy + size * 0.10, cx + size * 0.30, cy + size * 0.44, bone);
    fill_circle(img, cx - size * 0.22, cy + size * 0.40, size * 0.08, bone);
    fill_circle(img, cx + size * 0.22, cy + size * 0.40, size * 0.08, bone);
    if mono.is_none() {
        fill_circle(img, cx - size * 0.36, cy + size * 0.06, size * 0.08, shade);
        fill_circle(img, cx + size * 0.36, cy + size * 0.06, size * 0.08, shade);
        fill_circle(img, cx - size * 0.17, cy - size * 0.10, size * 0.13, hole);
        fill_circle(img, cx + size * 0.17, cy - size * 0.10, size * 0.13, hole);
        fill_tri(img, (cx - size * 0.07, cy + size * 0.16), (cx + size * 0.07, cy + size * 0.16), (cx, cy + size * 0.02), hole);
        for i in 0..4 {
            let x = cx - size * 0.20 + i as f32 * size * 0.13;
            fill_rect(img, x, cy + size * 0.26, x + size * 0.02, cy + size * 0.42, hole);
        }
    }
}

fn draw_rainbow(img: &mut RgbImage, cx: f32, cy: f32, r_outer: f32, thickness: f32) {
    for i in 0..6 {
        let col = hsv(i as f32 * 300.0 / 6.0, 0.85, 0.95);
        let r = r_outer - i as f32 * thickness;
        let steps = (r * 2.5).max(12.0) as i32;
        for k in 0..=steps {
            let a = PI + PI * k as f32 / steps as f32;
            fill_circle(img, cx + a.cos() * r, cy + a.sin() * r, thickness * 0.55, col);
        }
    }
}

/// Ice crystals scattered over a frozen panel.
fn draw_frost(img: &mut RgbImage, x0: f32, y0: f32, w: f32, h: f32) {
    let ice = [225, 245, 255];
    for i in 0..9u32 {
        let px = x0 + crate::chase::rand01(i * 977 + 5) * w;
        let py = y0 + crate::chase::rand01(i * 613 + 11) * h;
        let r = w * (0.025 + 0.02 * crate::chase::rand01(i * 331 + 7));
        for k in 0..3 {
            let a = k as f32 * PI / 3.0 + 0.3;
            capsule(img, px - a.cos() * r, py - a.sin() * r, px + a.cos() * r, py + a.sin() * r, r * 0.09, ice);
        }
    }
}

/// Which panel to paint, with everything that panel's picture depends on.
/// Doubles as the cache key: a panel only repaints when its key changes,
/// which is what keeps a continuously pulsing beat from redrawing the
/// whole strip every frame. `level` and `pulse` are quantised so a knob
/// sweep or a flash decays through a handful of frames, not hundreds.
#[derive(Clone, PartialEq, Eq, Hash)]
enum PanelKey {
    Dimmer { level: u8, blackout: bool },
    Beat { bpm: i32, pulse: u8, frozen: bool },
    Mode(DeckPage),
    Page,
    Inject(Option<String>),
    /// The Page knob on the Palettes page: sub-page name, position, count.
    Sub(&'static str, usize, usize),
    /// The programmer knob: channel name over its value, or a dimmed
    /// prompt when nothing is selected.
    Encoder(Option<(String, String)>),
    Idle,
}

/// Paints one 200x100 panel from its key.
fn paint_panel(key: &PanelKey) -> RgbImage {
    let s = SS as f32;
    let (pw, ph) = (PANEL_W * s, LCD_H as f32 * s);
    let cx = pw * 0.5;
    let (icon_cy, icon_size) = (ph * 0.62, ph * 0.31);
    let band = ((6.0 * s) as i32, (3.0 * s) as i32, (pw - 6.0 * s) as i32, (27.0 * s) as i32);
    let ow = 1.6 * s;
    let text_ow = (ow * 0.8).round() as i32;
    let paper = [255, 246, 226];
    let label = |img: &mut RgbImage, text: &str| {
        outlined_label(img, band.0, band.1, band.2, band.3, text, paper, text_ow);
    };
    let canvas = |hue0: f32, span: f32, bright: f32| (*tie_dye_panel(hue0, span, bright)).clone();

    let img = match key {
        PanelKey::Dimmer { level, blackout: true } => {
            let mut img = canvas(0.0, 40.0, 0.20);
            draw_eclipse(&mut img, cx, icon_cy, icon_size);
            label(&mut img, &format!("{level}%"));
            img
        }
        PanelKey::Dimmer { level, blackout: false } => {
            let lv = *level as f32 / 100.0;
            let mut img = canvas(5.0, 55.0, 0.30 + 0.22 * lv);
            glow(&mut img, cx, icon_cy, icon_size * 1.8, [255, 200, 90], 0.8 * lv);
            outlined(&mut img, ow, |img, dx, dy, mono| draw_sun(img, cx + dx, icon_cy + dy, icon_size, lv, mono));
            label(&mut img, &format!("{level}%"));
            img
        }
        PanelKey::Beat { bpm, frozen: true, .. } => {
            let mut img = canvas(195.0, 30.0, 0.42);
            outlined(&mut img, ow, |img, dx, dy, mono| {
                draw_bolt(img, cx + dx, icon_cy + dy, icon_size, 0.0, mono.or(Some([185, 225, 245])))
            });
            draw_frost(&mut img, 0.0, 0.0, pw, ph);
            label(&mut img, &bpm.to_string());
            img
        }
        PanelKey::Beat { bpm, pulse, frozen: false } => {
            let flash = *pulse as f32 / PULSE_STEPS as f32;
            let mut img = canvas(235.0, 80.0, 0.34);
            glow(&mut img, cx, icon_cy, icon_size * 1.9, [220, 190, 255], flash);
            outlined(&mut img, ow, |img, dx, dy, mono| draw_bolt(img, cx + dx, icon_cy + dy, icon_size, flash, mono));
            label(&mut img, &bpm.to_string());
            img
        }
        PanelKey::Mode(page) => {
            let mut img = match page {
                DeckPage::Presets => canvas(35.0, 50.0, 0.34),
                DeckPage::Palettes => canvas(0.0, 360.0, 0.34),
                DeckPage::Phasers => canvas(110.0, 90.0, 0.32),
                DeckPage::Chases => canvas(330.0, 75.0, 0.34),
            };
            // The creature takes the left, the page's full name and its
            // place in the cycle fill the right.
            let (icx, icy, isz) = (pw * 0.25, ph * 0.52, ph * 0.36);
            outlined(&mut img, ow, |img, dx, dy, mono| match page {
                DeckPage::Presets => draw_sun(img, icx + dx, icy + dy, isz, 1.0, mono),
                DeckPage::Palettes => draw_rose(img, icx + dx, icy + dy, isz, mono),
                DeckPage::Phasers => draw_bear(img, icx + dx, icy + dy, isz, mono),
                DeckPage::Chases => draw_terrapin(img, icx + dx, icy + dy, isz, mono),
            });
            let (tx0, tx1) = ((pw * 0.47) as i32, (pw - 6.0 * s) as i32);
            outlined_label(&mut img, tx0, (8.0 * s) as i32, tx1, (54.0 * s) as i32, page.name(), paper, text_ow);
            let pos = format!("{} / {}", page.number(), DeckPage::COUNT);
            outlined_label(&mut img, tx0, (58.0 * s) as i32, tx1, (92.0 * s) as i32, &pos, paper, text_ow);
            img
        }
        PanelKey::Page => {
            let mut img = canvas(265.0, 55.0, 0.32);
            draw_rainbow(&mut img, cx, icon_cy + icon_size * 0.10, icon_size * 0.88, icon_size * 0.065);
            outlined(&mut img, ow, |img, dx, dy, mono| draw_skull(img, cx + dx, icon_cy + dy, icon_size, mono));
            label(&mut img, "PAGE");
            img
        }
        PanelKey::Inject(name) => {
            let mut img = canvas(300.0, 60.0, 0.30);
            label(&mut img, "INJECT");
            let row = |a: f32, b: f32| ((a * s) as i32, (b * s) as i32);
            match split_label(name.as_deref().unwrap_or("NONE")) {
                (top, Some(bottom)) => {
                    let (ya, yb) = row(30.0, 62.0);
                    outlined_label(&mut img, band.0, ya, band.2, yb, &top, paper, text_ow);
                    let (ya, yb) = row(62.0, 94.0);
                    outlined_label(&mut img, band.0, ya, band.2, yb, &bottom, paper, text_ow);
                }
                (one, None) => {
                    let (ya, yb) = row(32.0, 92.0);
                    outlined_label(&mut img, band.0, ya, band.2, yb, &one, paper, text_ow);
                }
            }
            img
        }
        PanelKey::Sub(name, pos, count) => {
            let mut img = canvas(265.0, 55.0, 0.32);
            label(&mut img, "PAGE");
            let row = |a: f32, b: f32| ((a * s) as i32, (b * s) as i32);
            let (ya, yb) = row(30.0, 62.0);
            outlined_label(&mut img, band.0, ya, band.2, yb, name, paper, text_ow);
            let (ya, yb) = row(62.0, 94.0);
            let pos = format!("{pos} / {count}");
            outlined_label(&mut img, band.0, ya, band.2, yb, &pos, paper, text_ow);
            img
        }
        PanelKey::Encoder(Some((name, value))) => {
            let mut img = canvas(150.0, 70.0, 0.30);
            let name: String = name.chars().take(12).collect::<String>().to_uppercase();
            label(&mut img, &name);
            let row = |a: f32, b: f32| ((a * s) as i32, (b * s) as i32);
            match split_label(value) {
                (top, Some(bottom)) => {
                    let (ya, yb) = row(30.0, 62.0);
                    outlined_label(&mut img, band.0, ya, band.2, yb, &top, paper, text_ow);
                    let (ya, yb) = row(62.0, 94.0);
                    outlined_label(&mut img, band.0, ya, band.2, yb, &bottom, paper, text_ow);
                }
                (one, None) => {
                    let (ya, yb) = row(32.0, 92.0);
                    outlined_label(&mut img, band.0, ya, band.2, yb, &one, paper, text_ow);
                }
            }
            img
        }
        PanelKey::Encoder(None) => {
            let mut img = canvas(150.0, 70.0, 0.14);
            label(&mut img, "ENCODER");
            let (ya, yb) = ((32.0 * s) as i32, (92.0 * s) as i32);
            outlined_label(&mut img, band.0, ya, band.2, yb, "NO SEL", paper, text_ow);
            img
        }
        PanelKey::Idle => canvas(260.0, 60.0, 0.11),
    };
    downsample(&img, SS)
}

/// How finely the beat flash is stepped for caching. Twenty frames per
/// flash reads as smooth on the strip and bounds the cache.
const PULSE_STEPS: u8 = 20;

/// A finished panel, by key. Bounded so a long session can't grow it
/// without limit — the working set is a few dozen at most.
fn cached_panel(key: PanelKey) -> Arc<RgbImage> {
    static CACHE: OnceLock<Mutex<HashMap<PanelKey, Arc<RgbImage>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(p) = cache.lock().get(&key) {
        return p.clone();
    }
    let panel = Arc::new(paint_panel(&key));
    let mut guard = cache.lock();
    if guard.len() >= 96 {
        guard.clear();
    }
    guard.insert(key, panel.clone());
    panel
}

/// Paints the whole strip for one frame. Panels, left to right: Dimmer
/// (sun, or an eclipse on blackout), Beat (bolt, frosted when frozen), Mode
/// (rose / bear / terrapin for the three pages), Page (skull under a
/// rainbow, or the inject preset's name on the Chases page), an idle knob
/// dimmed down, and the programmer knob's channel and value. Each panel
/// comes from the cache, so a frame where nothing changed is six memcpys.
pub(crate) fn paint_strip(st: &StripState) -> RgbImage {
    let keys = [
        PanelKey::Dimmer { level: (st.level.clamp(0.0, 1.0) * 100.0).round() as u8, blackout: st.blackout },
        PanelKey::Beat {
            bpm: st.bpm,
            pulse: (st.pulse.clamp(0.0, 1.0) * PULSE_STEPS as f32).round() as u8,
            frozen: st.frozen,
        },
        PanelKey::Mode(st.page),
        match (&st.inject, st.sub) {
            (Some(name), _) => PanelKey::Inject(name.clone()),
            (None, Some((name, pos, count))) => PanelKey::Sub(name, pos, count),
            (None, None) => PanelKey::Page,
        },
        PanelKey::Idle,
        PanelKey::Encoder(st.encoder.clone()),
    ];
    let mut img = RgbImage::new(LCD_W, LCD_H);
    for (i, key) in keys.into_iter().enumerate() {
        blit(&mut img, &cached_panel(key), i as u32 * PANEL_W as u32, 0);
    }
    img
}

// --------------------------------------------------------------- wheel art

/// A key label for a wheel slot or palette: its name, cut to fit.
fn wheel_label(name: &str) -> String {
    name.trim().to_uppercase().chars().take(7).collect()
}

/// An outline circle.
fn ring(img: &mut RgbImage, cx: f32, cy: f32, r: f32, thickness: f32, ink: [u8; 3], bg: [u8; 3]) {
    fill_circle(img, cx, cy, r, ink);
    fill_circle(img, cx, cy, r - thickness, bg);
}

fn regular_polygon(cx: f32, cy: f32, r: f32, n: usize, rot: f32) -> Vec<(f32, f32)> {
    (0..n)
        .map(|k| {
            let a = rot + k as f32 * TAU / n as f32;
            (cx + a.cos() * r, cy + a.sin() * r)
        })
        .collect()
}

/// Three arrow heads chasing round a ring of radius `r`.
fn draw_rotation(img: &mut RgbImage, cx: f32, cy: f32, r: f32, size: f32, ink: [u8; 3]) {
    for k in 0..3 {
        let a = k as f32 * TAU / 3.0;
        let (x, y) = (cx + a.cos() * r, cy + a.sin() * r);
        let (tx, ty) = (-a.sin(), a.cos());
        let (nx, ny) = (a.cos(), a.sin());
        let tip = (x + tx * 0.8 * size, y + ty * 0.8 * size);
        let b1 = (x - tx * 0.4 * size + nx * 0.6 * size, y - ty * 0.4 * size + ny * 0.6 * size);
        let b2 = (x - tx * 0.4 * size - nx * 0.6 * size, y - ty * 0.4 * size - ny * 0.6 * size);
        fill_tri(img, tip, b1, b2, ink);
    }
}

/// Motion strokes either side of a gobo that shakes.
fn draw_shake(img: &mut RgbImage, cx: f32, cy: f32, r: f32, ink: [u8; 3]) {
    for side in [-1.0, 1.0] {
        let x = cx + side * 1.08 * r;
        capsule(img, x, cy - 0.34 * r, x, cy + 0.34 * r, 0.05 * r, ink);
        let x = cx + side * 1.26 * r;
        capsule(img, x, cy - 0.18 * r, x, cy + 0.18 * r, 0.05 * r, ink);
    }
}

/// The level a slot name spells, for the meter beside an effect picture.
fn level_of(label: &str) -> Option<f32> {
    match label {
        "off" | "none" | "no" | "0" => Some(0.0),
        "low" => Some(0.33),
        "mid" | "medium" => Some(0.66),
        "full" | "high" | "max" => Some(1.0),
        _ => None,
    }
}

/// A picture for a wheel or effect position, from the island it belongs
/// to and its name. Gobos and prisms have their own vocabularies; the
/// other effects get a glyph per island (a pinwheel, a light ring, a
/// program's play mark, fog…) and, when the slot is a level, a small meter
/// beside it. Anything unknown gets a dotted ring under the name.
fn draw_wheel_art(
    img: &mut RgbImage,
    island: &str,
    label: &str,
    cx: f32,
    cy: f32,
    r: f32,
    ink: [u8; 3],
    bg: [u8; 3],
) {
    let isl = island.trim().to_lowercase();
    if isl.contains("gobo") {
        draw_gobo_art(img, label, cx, cy, r, ink, bg);
        return;
    }
    if isl.contains("prism") {
        draw_prism_art(img, label, cx, cy, r, ink, bg);
        return;
    }
    let l = label.trim().to_lowercase();
    let level = level_of(&l);
    let off = level == Some(0.0);
    if isl.contains("spin") {
        if off {
            ring(img, cx, cy, 0.62 * r, 0.12 * r, ink, bg);
        } else {
            // A pinwheel: three fan blades round a hub.
            for k in 0..3 {
                let a0 = k as f32 * TAU / 3.0;
                let mut pts = vec![(cx, cy)];
                for i in 0..=8 {
                    let a = a0 + i as f32 / 8.0 * 1.25;
                    let rad = 0.88 * r * (0.55 + 0.45 * i as f32 / 8.0);
                    pts.push((cx + a.cos() * rad, cy + a.sin() * rad));
                }
                fill_poly(img, &pts, ink);
            }
            fill_circle(img, cx, cy, 0.2 * r, bg);
            fill_circle(img, cx, cy, 0.1 * r, ink);
            if l.contains('>') || l.contains("rot") {
                draw_rotation(img, cx, cy, 1.02 * r, 0.14 * r, ink);
            }
        }
    } else if isl.contains("ring") {
        // A light ring: thicker the brighter.
        let t = level.unwrap_or(1.0);
        ring(img, cx, cy, 0.85 * r, (0.1 + 0.32 * t) * r, ink, bg);
        if t > 0.0 {
            fill_circle(img, cx, cy, 0.12 * r, ink);
        }
    } else if isl.contains("macro") || isl.contains("auto") {
        let colour_program = ["col", "clr", "grad", "jmp", "jump", "cmbo", "combo", "rainbow"]
            .iter()
            .any(|k| l.contains(k));
        let sound = (l.starts_with('s') && l.len() <= 3 && l[1..].chars().all(|c| c.is_ascii_digit()))
            || l.contains("snd")
            || l.contains("sound");
        if colour_program {
            for (k, h) in [0.0, 120.0, 240.0].into_iter().enumerate() {
                let x = cx + (k as f32 - 1.0) * 0.58 * r;
                fill_circle(img, x, cy, 0.27 * r, hsv(h, 0.85, 0.95));
            }
        } else if sound {
            for (k, h) in [0.35, 0.75, 0.5, 1.0].into_iter().enumerate() {
                let x0 = cx - 0.8 * r + k as f32 * 0.44 * r;
                fill_rect(img, x0, cy + 0.65 * r - h * 1.2 * r, x0 + 0.3 * r, cy + 0.65 * r, ink);
            }
        } else if off {
            ring(img, cx, cy, 0.62 * r, 0.12 * r, ink, bg);
        } else {
            fill_poly(img, &[(cx - 0.5 * r, cy - 0.7 * r), (cx + 0.75 * r, cy), (cx - 0.5 * r, cy + 0.7 * r)], ink);
        }
    } else if isl.contains("fog") || isl.contains("smoke") || isl.contains("haze") {
        let t = level.unwrap_or(1.0);
        if off {
            ring(img, cx, cy, 0.62 * r, 0.12 * r, ink, bg);
        } else {
            let scale = 0.7 + 0.3 * t;
            for (dx, dy, pr) in [(-0.5, 0.12, 0.34), (0.05, -0.2, 0.46), (0.55, 0.12, 0.34)] {
                fill_circle(img, cx + dx * r, cy + dy * r, pr * r * scale, ink);
            }
            fill_rect(img, cx - 0.8 * r, cy + 0.1 * r, cx + 0.85 * r, cy + 0.46 * r, ink);
        }
    } else if isl.contains("circ") {
        ring(img, cx, cy, 0.78 * r, 0.11 * r, ink, bg);
        if !off {
            let a = -TAU / 8.0;
            fill_circle(img, cx + a.cos() * 0.78 * r, cy + a.sin() * 0.78 * r, 0.2 * r, ink);
        }
    } else if isl.contains("frost") {
        draw_frost(img, cx - r, cy - r, 2.0 * r, 2.0 * r);
    } else {
        for k in 0..10 {
            let a = k as f32 * TAU / 10.0;
            fill_circle(img, cx + a.cos() * 0.72 * r, cy + a.sin() * 0.72 * r, 0.1 * r, ink);
        }
    }
    if let Some(t) = level {
        // A meter beside the glyph: how far up the channel this slot sits.
        let (x0, x1) = (cx + 1.08 * r, cx + 1.24 * r);
        fill_rect(img, x0, cy - 0.65 * r, x1, cy + 0.65 * r, lerp_rgb(bg, ink, 0.35));
        fill_rect(img, x0, cy + 0.65 * r - 1.3 * r * t, x1, cy + 0.65 * r, ink);
    }
}

/// Prisms: off is an empty ring, on a prism, spinning adds arrows.
fn draw_prism_art(img: &mut RgbImage, label: &str, cx: f32, cy: f32, r: f32, ink: [u8; 3], bg: [u8; 3]) {
    let l = label.trim().to_lowercase();
    if l.contains("off") || l == "0" || l.contains("out") || l.contains("open") {
        ring(img, cx, cy, 0.7 * r, 0.12 * r, ink, bg);
    } else {
        fill_poly(img, &[(cx, cy - 0.8 * r), (cx + 0.85 * r, cy + 0.55 * r), (cx - 0.85 * r, cy + 0.55 * r)], ink);
        fill_poly(img, &[(cx, cy - 0.32 * r), (cx + 0.42 * r, cy + 0.34 * r), (cx - 0.42 * r, cy + 0.34 * r)], bg);
        if l.contains("spin") || l.contains("rot") || l.contains('>') {
            draw_rotation(img, cx, cy, 1.0 * r, 0.16 * r, ink);
        }
    }
}

/// Gobos, from the loose names fixture files use ("Flwr", "invVortex",
/// "Sspot" for a shaking spot): matched on fragments, a dotted ring for
/// anything unknown.
fn draw_gobo_art(img: &mut RgbImage, label: &str, cx: f32, cy: f32, r: f32, ink: [u8; 3], bg: [u8; 3]) {
    let l = label.trim().to_lowercase();
    // "S<gobo>" is how this rig's files spell a shaking gobo.
    let (base, shake) = match l.strip_prefix('s') {
        Some(rest)
            if ["spot", "small", "fl", "vort", "sun", "star", "dais", "dot"]
                .iter()
                .any(|k| rest.starts_with(k)) =>
        {
            (rest.to_string(), true)
        }
        _ => (l.clone(), l.contains("shake")),
    };
    let has = |k: &str| base.contains(k);
    if has("auto") || has(">") || has("rot") {
        ring(img, cx, cy, 0.62 * r, 0.12 * r, ink, bg);
        draw_rotation(img, cx, cy, 0.85 * r, 0.16 * r, ink);
    } else if has("small") {
        fill_circle(img, cx, cy, 0.3 * r, ink);
    } else if has("spot") || has("open") {
        fill_circle(img, cx, cy, 0.62 * r, ink);
    } else if has("dais") {
        for k in 0..8 {
            let a = k as f32 * TAU / 8.0;
            fill_circle(img, cx + a.cos() * 0.62 * r, cy + a.sin() * 0.62 * r, 0.22 * r, ink);
        }
        fill_circle(img, cx, cy, 0.24 * r, ink);
    } else if has("fl") {
        for k in 0..6 {
            let a = k as f32 * TAU / 6.0 - TAU / 4.0;
            fill_circle(img, cx + a.cos() * 0.5 * r, cy + a.sin() * 0.5 * r, 0.32 * r, ink);
        }
        fill_circle(img, cx, cy, 0.24 * r, bg);
    } else if has("vort") || has("spir") || has("swirl") {
        let dir = if has("inv") { -1.0 } else { 1.0 };
        let steps = 48;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let a = dir * t * TAU * 2.0;
            let rad = 0.04 * r + 0.9 * r * t;
            fill_circle(img, cx + a.cos() * rad, cy + a.sin() * rad, 0.09 * r + 0.07 * r * t, ink);
        }
    } else if has("dot") || has("ring") {
        for k in 0..8 {
            let a = k as f32 * TAU / 8.0;
            fill_circle(img, cx + a.cos() * 0.68 * r, cy + a.sin() * 0.68 * r, 0.16 * r, ink);
        }
    } else if has("sun") {
        draw_sun(img, cx, cy, 0.85 * r, 1.0, Some(ink));
    } else if has("star") {
        let pts: Vec<(f32, f32)> = (0..10)
            .map(|k| {
                let a = -TAU / 4.0 + k as f32 * TAU / 10.0;
                let rad = if k % 2 == 0 { r } else { 0.42 * r };
                (cx + a.cos() * rad, cy + a.sin() * rad)
            })
            .collect();
        fill_poly(img, &pts, ink);
    } else if has("note") {
        fill_ellipse(img, cx - 0.22 * r, cy + 0.45 * r, 0.34 * r, 0.24 * r, ink);
        capsule(img, cx + 0.08 * r, cy + 0.4 * r, cx + 0.08 * r, cy - 0.8 * r, 0.07 * r, ink);
        fill_tri(img, (cx + 0.08 * r, cy - 0.8 * r), (cx + 0.55 * r, cy - 0.45 * r), (cx + 0.08 * r, cy - 0.3 * r), ink);
    } else if has("squig") || has("wav") {
        let steps = (r * 4.0).max(8.0) as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let x = cx - r + 2.0 * r * t;
            let y = cy + 0.5 * r * (TAU * 1.5 * t).sin();
            fill_circle(img, x, y, 0.12 * r, ink);
        }
    } else if base == "x" || has("cross") {
        capsule(img, cx - 0.7 * r, cy - 0.7 * r, cx + 0.7 * r, cy + 0.7 * r, 0.11 * r, ink);
        capsule(img, cx - 0.7 * r, cy + 0.7 * r, cx + 0.7 * r, cy - 0.7 * r, 0.11 * r, ink);
    } else if has("merc") || has("peace") {
        ring(img, cx, cy, 0.9 * r, 0.13 * r, ink, bg);
        for a in [-TAU / 4.0, TAU / 12.0, 5.0 * TAU / 12.0] {
            capsule(img, cx, cy, cx + a.cos() * 0.82 * r, cy + a.sin() * 0.82 * r, 0.065 * r, ink);
        }
    } else if has("oct") || has("hex") {
        let n = if has("oct") { 8 } else { 6 };
        fill_poly(img, &regular_polygon(cx, cy, 0.95 * r, n, TAU / (2 * n) as f32), ink);
        fill_poly(img, &regular_polygon(cx, cy, 0.7 * r, n, TAU / (2 * n) as f32), bg);
    } else if has("tri") {
        fill_poly(img, &regular_polygon(cx, cy, 0.95 * r, 3, -TAU / 4.0), ink);
        fill_poly(img, &regular_polygon(cx, cy, 0.6 * r, 3, -TAU / 4.0), bg);
    } else if has("sq") || has("box") {
        fill_rect(img, cx - 0.75 * r, cy - 0.75 * r, cx + 0.75 * r, cy + 0.75 * r, ink);
        fill_rect(img, cx - 0.5 * r, cy - 0.5 * r, cx + 0.5 * r, cy + 0.5 * r, bg);
    } else if has("bar") || has("line") || has("slot") {
        capsule(img, cx - 0.8 * r, cy, cx + 0.8 * r, cy, 0.14 * r, ink);
    } else {
        for k in 0..10 {
            let a = k as f32 * TAU / 10.0;
            fill_circle(img, cx + a.cos() * 0.72 * r, cy + a.sin() * 0.72 * r, 0.1 * r, ink);
        }
    }
    if shake {
        draw_shake(img, cx, cy, r, ink);
    }
}

// -------------------------------------------------------------- key artwork

fn fill_tri(img: &mut RgbImage, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), color: [u8; 3]) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let x0 = (p0.0.min(p1.0).min(p2.0).floor().max(0.0)) as i32;
    let x1 = (p0.0.max(p1.0).max(p2.0).ceil() as i32).min(w - 1);
    let y0 = (p0.1.min(p1.1).min(p2.1).floor().max(0.0)) as i32;
    let y1 = (p0.1.max(p1.1).max(p2.1).ceil() as i32).min(h - 1);
    let edge = |a: (f32, f32), b: (f32, f32), px: f32, py: f32| {
        (b.0 - a.0) * (py - a.1) - (b.1 - a.1) * (px - a.0)
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let (e0, e1, e2) =
                (edge(p0, p1, px, py), edge(p1, p2, px, py), edge(p2, p0, px, py));
            let inside = (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0)
                || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0);
            if inside {
                img.put_pixel(x as u32, y as u32, image::Rgb(color));
            }
        }
    }
}

/// Draws a key's artwork centred on `(cx, cy)` with half-extent `r`. `fg` is
/// the watermark colour and `bg` the key's own fill, used to punch holes
/// (rings, the gap inside a bullseye) back out of the art.
fn draw_key_icon(
    img: &mut RgbImage,
    icon: KeyIcon,
    cx: f32,
    cy: f32,
    r: f32,
    fg: [u8; 3],
    bg: [u8; 3],
) {
    match icon {
        KeyIcon::None => {}
        KeyIcon::Bullseye => {
            for (k, ring) in [1.0, 0.79, 0.58, 0.37, 0.16].into_iter().enumerate() {
                let color = if k % 2 == 0 { fg } else { bg };
                fill_circle(img, cx, cy, r * ring, color);
            }
        }
        KeyIcon::Ring => {
            fill_circle(img, cx, cy, r, fg);
            fill_circle(img, cx, cy, r * 0.72, bg);
        }
        KeyIcon::Rainbow => {
            // The strip's arch, sat low so its span fills the key's width.
            draw_rainbow(img, cx, cy + 0.45 * r, r, r * 0.11);
        }
        KeyIcon::StageLight => {
            // Par can body, its lens, then the beam spilling down the key.
            fill_rect(img, cx - 0.30 * r, cy - 0.95 * r, cx + 0.30 * r, cy - 0.34 * r, fg);
            fill_rect(img, cx - 0.40 * r, cy - 0.36 * r, cx + 0.40 * r, cy - 0.22 * r, fg);
            let (ly, by) = (cy - 0.20 * r, cy + 0.95 * r);
            fill_tri(img, (cx - 0.30 * r, ly), (cx + 0.30 * r, ly), (cx + 0.85 * r, by), fg);
            fill_tri(img, (cx - 0.30 * r, ly), (cx + 0.85 * r, by), (cx - 0.85 * r, by), fg);
        }
        KeyIcon::LeftRight => {
            fill_rect(img, cx - 0.78 * r, cy - 0.14 * r, cx + 0.78 * r, cy + 0.14 * r, fg);
            fill_tri(
                img,
                (cx - 1.0 * r, cy),
                (cx - 0.52 * r, cy - 0.46 * r),
                (cx - 0.52 * r, cy + 0.46 * r),
                fg,
            );
            fill_tri(
                img,
                (cx + 1.0 * r, cy),
                (cx + 0.52 * r, cy - 0.46 * r),
                (cx + 0.52 * r, cy + 0.46 * r),
                fg,
            );
        }
        KeyIcon::UpDown => {
            fill_rect(img, cx - 0.14 * r, cy - 0.78 * r, cx + 0.14 * r, cy + 0.78 * r, fg);
            fill_tri(
                img,
                (cx, cy - 1.0 * r),
                (cx - 0.46 * r, cy - 0.52 * r),
                (cx + 0.46 * r, cy - 0.52 * r),
                fg,
            );
            fill_tri(
                img,
                (cx, cy + 1.0 * r),
                (cx - 0.46 * r, cy + 0.52 * r),
                (cx + 0.46 * r, cy + 0.52 * r),
                fg,
            );
        }
        KeyIcon::Wave => {
            let steps = (r * 4.0).max(8.0) as i32;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let x = cx - r + 2.0 * r * t;
                let y = cy + 0.55 * r * (std::f32::consts::TAU * 1.5 * t).sin();
                fill_circle(img, x, y, 0.13 * r, fg);
            }
        }
        KeyIcon::Burst => {
            fill_circle(img, cx, cy, 0.22 * r, fg);
            for k in 0..12 {
                let ang = k as f32 * std::f32::consts::TAU / 12.0;
                let (dx, dy) = (ang.cos(), ang.sin());
                let mut t = 0.30;
                while t <= 1.0 {
                    // Taper the ray as it travels out, so it reads as a spark.
                    let dot = 0.15 * r * (1.0 - t * 0.75);
                    fill_circle(img, cx + dx * r * t, cy + dy * r * t, dot, fg);
                    t += 0.12;
                }
            }
        }
    }
}

const ACCENT_SOFT: [u8; 3] = [0x6F, 0xB6, 0xB5];
const ACCENT: [u8; 3] = [0x1F, 0x6F, 0x78];
/// Snap's accent — deliberately a different hue from Spacing's, since both
/// groups otherwise show the same four percentages.
const SNAP_ACCENT: [u8; 3] = [250, 165, 60];
/// Select all, on the Phaser page — yellow, so it reads as the odd one out
/// among the phaser pads rather than as another phaser.
const SELECT_ALL_ACCENT: [u8; 3] = [245, 210, 70];
/// The Chases page's shape keys.
const CHASE_ACCENT: [u8; 3] = [150, 190, 250];
/// A chase that is actually running, on its shape key and on Go.
const CHASE_LIVE: [u8; 3] = [120, 235, 150];
/// Clear when its next press is the full blackout — red, so you know.
const CLEAR_BLACKOUT: [u8; 3] = [215, 70, 60];
/// Gobo tiles: warm, like a beam through glass.
const GOBO_ACCENT: [u8; 3] = [232, 190, 90];
/// Effect islands, coloured in turn so each reads as its own cluster.
const ISLAND_ACCENTS: [[u8; 3]; 6] = [
    [170, 140, 240],
    [90, 200, 190],
    [240, 150, 70],
    [120, 210, 110],
    [235, 120, 170],
    [110, 150, 240],
];
/// A stored palette that sets a wheel (as opposed to a single slot).
const WHEEL_PALETTE_ACCENT: [u8; 3] = [205, 165, 130];
/// How long the deck's fade-start / fade-stop key takes.
const CYCLE_FADE_S: f32 = 2.0;

/// Dimmed version of an accent colour, for a control key that isn't active.
fn dim(rgb: [u8; 3]) -> [u8; 3] {
    let mix = |v: u8| ((v as u16 * 35 + 40 * 65) / 100) as u8;
    [mix(rgb[0]), mix(rgb[1]), mix(rgb[2])]
}

fn control_spec(index: u8, label: &str, active: bool, accent: [u8; 3]) -> KeySpec {
    let rgb = if active { accent } else { dim(accent) };
    KeySpec::plain(index, rgb, Some(label.to_string()))
}

/// The lane a stored (hand-built) palette joins from a wheel sub-page: the
/// Gobo lane on the Gobos page, its own "Stored" lane on the Effects pages;
/// none on Colors, where palettes go to the colour cycle instead.
fn stored_lane(sub: PaletteSub) -> Option<&'static str> {
    match sub {
        PaletteSub::Colors => None,
        PaletteSub::Gobos => Some("Gobo"),
        PaletteSub::Effects(_) => Some("Stored"),
    }
}

/// An island's header key: its name written out in full, over two lines
/// when it has two words, on a darker shade of the island's colour.
fn header_spec(index: u8, name: &str, accent: [u8; 3]) -> KeySpec {
    let upper = name.trim().to_uppercase();
    let label = match upper.split_once(' ') {
        Some((a, b)) => format!("{a}\n{}", b.replace(' ', "")),
        None => upper,
    };
    KeySpec::plain(index, lerp_rgb(accent, [0, 0, 0], 0.55), Some(label))
}

fn pattern_label(p: SeqPattern) -> &'static str {
    match p {
        SeqPattern::Wave => "WAVE",
        SeqPattern::Wings => "WING",
        SeqPattern::Random => "RAND",
    }
}

/// The four discrete levels exposed for Spacing/Snap (deck keys aren't
/// sliders), matching `cycle_spread`/`cycle_shape`'s 0..1 range. Spacing and
/// Snap both step through the same four percentages, so each key's label is
/// prefixed ("SP"/"SN") on top of the colour difference above.
const LEVELS: [(f32, &str); 4] = [(0.0, "0"), (0.33, "33"), (0.66, "66"), (1.0, "100")];

fn level_num(v: f32) -> &'static str {
    LEVELS.iter().find(|(lv, _)| (*lv - v).abs() < 0.001).map_or("?", |(_, s)| *s)
}

// ------------------------------------------------------------------- App --

/// What one deck key currently shows/does. Rebuilt fresh every frame from
/// `self.palettes`/`self.deck.kind()` so rendering and press-handling can
/// never drift apart from each other.
#[derive(Clone, Copy)]
enum DeckItem {
    /// A ShowBuddy preset, by (bank, index) — the Presets page.
    Preset(usize, usize),
    Color(u32),
    Pattern(SeqPattern),
    Spacing(f32),
    Snap(f32),
    Tap,
    /// A slot index into `App.phaser_deck` (Phaser page only).
    PhaserPad(usize),
    /// Grab the whole rig, so a phaser applies to everything.
    SelectAll,
    /// The staged Clear: encoders, then effects, then blackout.
    Clear,
    /// Start / stop the palette cycle on the spot.
    CycleGo,
    /// Start / stop the palette cycle through a short fade.
    CycleFade,
    /// A wheel or effect position: (island, slot) indices into the
    /// sub-page's `App::wheel_islands`.
    Wheel(usize, usize),
    /// An island's header key — its name written out; pressing does nothing.
    Island(usize),
    /// A stored palette that sets a wheel, by palette id.
    WheelPalette(u32),
    // --- Chases page ---
    ChaseShape(ChaseKind),
    /// Sweeps per second.
    ChaseSpeed(f32),
    /// Band width in degrees.
    ChaseWidth(f32),
    ChaseDir,
    ChaseSoft,
    ChaseAuto,
    /// Show the chase's shape on the 3D stage, where it can be moved.
    ChaseShow,
    ChaseGo,
}

/// What the Phaser page puts on one key. The bottom corners are reserved —
/// Tap and Clear sit in the same place on every page, and Select all needs
/// to be somewhere you can hit without looking.
pub enum PhaserKey {
    Pad(usize),
    SelectAll,
    Clear,
    Tap,
}

/// The Phaser page's key -> contents mapping, shared by the deck and by the
/// page editor so the grid on screen matches the grid under your hands.
pub fn phaser_page_key(k: usize, rows: usize, cols: usize) -> PhaserKey {
    let last = rows * cols - 1;
    let last_row_start = (rows - 1) * cols;
    if k == last {
        return PhaserKey::Tap;
    }
    if cols >= 3 && k + 1 == last {
        return PhaserKey::Clear;
    }
    if rows >= 2 && k == last_row_start {
        return PhaserKey::SelectAll;
    }
    // Slots stay contiguous by counting only the keys that hold a pad, so
    // reserving the corners doesn't renumber the rows above them.
    let mut slot = k;
    if rows >= 2 && k > last_row_start {
        slot -= 1;
    }
    PhaserKey::Pad(slot)
}

/// Speed steps offered on the Chases page, slowest first.
const CHASE_SPEEDS: [f32; 6] = [0.05, 0.1, 0.2, 0.4, 0.8, 1.5];
/// Band widths offered on the Chases page, in degrees.
const CHASE_WIDTHS: [f32; 4] = [15.0, 35.0, 60.0, 85.0];

/// Colour words a ShowBuddy preset name might carry, and the key colour
/// each one gets. Matched by prefix, so "golden" and "greenish" count.
const PRESET_HUES: [(&str, [u8; 3]); 11] = [
    ("red", [255, 60, 50]),
    ("blue", [60, 90, 255]),
    ("magenta", [240, 70, 220]),
    ("pink", [250, 120, 190]),
    ("green", [60, 220, 80]),
    ("cyan", [60, 220, 230]),
    ("teal", [50, 190, 180]),
    ("yellow", [245, 220, 60]),
    ("white", [235, 235, 235]),
    ("gold", [255, 180, 50]),
    ("purple", [160, 80, 240]),
];
/// A rainbow preset's key — neutral, so the arch carries the colour.
const PRESET_RAINBOW_KEY: [u8; 3] = [120, 120, 135];
/// The OFF preset's key: as near to black as still reads as a button.
const PRESET_OFF_KEY: [u8; 3] = [40, 40, 44];
/// A preset whose name says nothing about its colour.
const PRESET_PLAIN_KEY: [u8; 3] = [150, 150, 160];

/// The colour word in a preset name, if any.
fn preset_hue(name: &str) -> Option<&'static str> {
    name.to_lowercase()
        .split_whitespace()
        .find_map(|w| PRESET_HUES.iter().find(|(h, _)| w.starts_with(h)).map(|(h, _)| *h))
}

/// How a preset is drawn on a key, from nothing but its name: the colour
/// word picks the hue, a "50" dims it, "rainbow" gets the arch, and the
/// label is whatever the name says beyond the colour ("100", "FADE",
/// "OFF"; a rainbow's last word).
fn preset_key_look(name: &str) -> ([u8; 3], String, KeyIcon) {
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let hue = preset_hue(name);
    let rest: Vec<&str> = words
        .iter()
        .copied()
        .filter(|w| hue.is_none_or(|h| !w.starts_with(h)))
        .collect();
    let rainbow = lower.contains("rainbow");
    let level = if rest.iter().any(|w| *w == "50") { 0.6 } else { 1.0 };
    let scale = |c: [u8; 3]| c.map(|v| (v as f32 * level).round() as u8);
    let rgb = if rainbow {
        PRESET_RAINBOW_KEY
    } else if let Some(c) = hue.and_then(|h| PRESET_HUES.iter().find(|(k, _)| *k == h)).map(|(_, c)| *c) {
        scale(c)
    } else if lower.trim() == "off" {
        PRESET_OFF_KEY
    } else {
        PRESET_PLAIN_KEY
    };
    let label_src = if rainbow {
        rest.iter().rev().find(|w| **w != "rainbow").copied()
    } else {
        rest.first().copied()
    }
    .or_else(|| words.first().copied())
    .unwrap_or("");
    let label: String = label_src.to_uppercase().chars().take(5).collect();
    let icon = if rainbow { KeyIcon::Rainbow } else { KeyIcon::None };
    (rgb, label, icon)
}

/// The Presets page's rows of (bank, index); `None` is a deliberate gap.
type PresetRows = Vec<Vec<Option<(usize, usize)>>>;

/// The Presets page, from the ShowBuddy banks: one row per "General …"
/// bank (brightest first), later rows lined up under the first by colour so
/// each column is one hue, then a row of the OFF bank and every rainbow.
fn preset_page_rows(banks: &[PresetBank]) -> PresetRows {
    let lower = |s: &str| s.to_lowercase();
    let mut general: Vec<(i32, usize)> = banks
        .iter()
        .enumerate()
        .filter(|(_, b)| lower(&b.name).starts_with("general"))
        .map(|(i, b)| {
            let level = b.name.split_whitespace().find_map(|w| w.parse::<i32>().ok());
            (level.unwrap_or(0), i)
        })
        .collect();
    general.sort_by(|a, b| b.0.cmp(&a.0));

    let mut rows: PresetRows = Vec::new();
    let mut lead: Vec<Option<&'static str>> = Vec::new();
    for (_, bi) in general {
        let bank = &banks[bi];
        let mut items: Vec<(usize, usize)> = (0..bank.presets.len()).map(|pi| (bi, pi)).collect();
        if rows.is_empty() {
            lead = items.iter().map(|&(_, pi)| preset_hue(&bank.presets[pi].name)).collect();
            rows.push(items.into_iter().map(Some).collect());
            continue;
        }
        // Under each of the first row's hues, or a gap; leftovers trail.
        let mut row: Vec<Option<(usize, usize)>> = Vec::new();
        for h in &lead {
            let hit = h.and_then(|h| {
                items.iter().position(|&(_, pi)| preset_hue(&bank.presets[pi].name) == Some(h))
            });
            row.push(hit.map(|pos| items.remove(pos)));
        }
        while row.last() == Some(&None) {
            row.pop();
        }
        row.extend(items.into_iter().map(Some));
        rows.push(row);
    }

    let mut extras: Vec<Option<(usize, usize)>> = Vec::new();
    for (bi, b) in banks.iter().enumerate() {
        if lower(&b.name) == "off" {
            extras.extend((0..b.presets.len()).map(|pi| Some((bi, pi))));
        }
    }
    for (bi, b) in banks.iter().enumerate() {
        if lower(&b.name).contains("rainbow") {
            extras.extend(
                b.presets
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| lower(&p.name).contains("rainbow"))
                    .map(|(pi, _)| Some((bi, pi))),
            );
        }
    }
    if !extras.is_empty() {
        rows.push(extras);
    }
    rows
}

/// Lays the preset rows onto the grid: each row of presets takes a deck row
/// (cut to the width), with the last deck row kept for Clear and Tap. A
/// deck without room for rows lists what fits, in order, Clear and Tap last.
fn preset_layout(page: &PresetRows, rows: usize, cols: usize) -> Vec<(u8, DeckItem)> {
    let mut out = Vec::new();
    let n = rows * cols;
    if rows >= 4 && cols >= 7 {
        for (r, items) in page.iter().enumerate().take(rows - 1) {
            for (c, item) in items.iter().enumerate().take(cols) {
                if let Some((b, p)) = item {
                    out.push(((r * cols + c) as u8, DeckItem::Preset(*b, *p)));
                }
            }
        }
    } else {
        let flat = page.iter().flatten().flatten().copied();
        for (i, (b, p)) in flat.enumerate().take(n.saturating_sub(2)) {
            out.push((i as u8, DeckItem::Preset(b, p)));
        }
    }
    if n >= 3 {
        out.push(((n - 2) as u8, DeckItem::Clear));
        out.push(((n - 1) as u8, DeckItem::Tap));
    } else if n >= 1 {
        out.push(((n - 1) as u8, DeckItem::Tap));
    }
    out
}

/// Lays islands (each `sizes[i]` keys, header included) onto pages of a
/// `rows` × `cols` grid whose last `reserved` keys are spoken for. An
/// island starts a fresh row when it won't fit in what's left of the
/// current one, wraps across rows when longer than one, never straddles a
/// page, and is followed by one blank key. Returns, per page, each island's
/// index and its keys, header first.
fn island_pages(sizes: &[usize], rows: usize, cols: usize, reserved: usize) -> Vec<Vec<(usize, Vec<u8>)>> {
    let usable = (rows * cols).saturating_sub(reserved);
    let mut pages: Vec<Vec<(usize, Vec<u8>)>> = Vec::new();
    let mut page: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut next = 0usize;
    for (i, &size) in sizes.iter().enumerate() {
        if size == 0 || usable == 0 {
            continue;
        }
        let size = size.min(usable);
        let col = next % cols;
        if col != 0 && col + size > cols {
            next = (next / cols + 1) * cols;
        }
        if next + size > usable {
            if !page.is_empty() {
                pages.push(std::mem::take(&mut page));
            }
            next = 0;
        }
        page.push((i, (next..next + size).map(|k| k as u8).collect()));
        next += size + 1;
    }
    if !page.is_empty() {
        pages.push(page);
    }
    pages
}

fn control_items() -> Vec<DeckItem> {
    let mut v = vec![
        DeckItem::CycleGo,
        DeckItem::CycleFade,
        DeckItem::Pattern(SeqPattern::Wave),
        DeckItem::Pattern(SeqPattern::Wings),
        DeckItem::Pattern(SeqPattern::Random),
    ];
    v.extend(LEVELS.iter().map(|(lv, _)| DeckItem::Spacing(*lv)));
    v.extend(LEVELS.iter().map(|(lv, _)| DeckItem::Snap(*lv)));
    v.push(DeckItem::Tap);
    v
}

/// The Chases page. On a full grid it reads top to bottom: every shape, then
/// speed, then the size/feel row, with Go and Tap on the last row. On a deck
/// too small for that it falls back to a priority list — the shapes and Go
/// always make the cut, so even eight keys still drive a chase.
///
/// Depends only on the grid, not on app state, so it can be laid out and
/// previewed without a device attached.
fn chase_layout(rows: usize, cols: usize) -> Vec<(u8, DeckItem)> {
        let mut out = Vec::new();
        let shapes = ChaseKind::ALL;

        if cols >= 7 && rows >= 4 {
            let at = |r: usize, c: usize, item: DeckItem, out: &mut Vec<(u8, DeckItem)>| {
                if r < rows && c < cols {
                    out.push(((r * cols + c) as u8, item));
                }
            };
            // Shapes fill row 0 and spill onto row 1.
            for (i, k) in shapes.into_iter().enumerate() {
                at(i / cols, i % cols, DeckItem::ChaseShape(k), &mut out);
            }
            // Speeds take the rest of row 1, right-aligned after the spill.
            let spill = shapes.len() % cols;
            for (i, s) in CHASE_SPEEDS.into_iter().enumerate() {
                at(1, spill + i, DeckItem::ChaseSpeed(s), &mut out);
            }
            // Row 2: how big the band is, then how it behaves.
            for (i, w) in CHASE_WIDTHS.into_iter().enumerate() {
                at(2, i, DeckItem::ChaseWidth(w), &mut out);
            }
            at(2, 4, DeckItem::ChaseDir, &mut out);
            at(2, 5, DeckItem::ChaseSoft, &mut out);
            at(2, 6, DeckItem::ChaseAuto, &mut out);
            at(2, 7, DeckItem::ChaseShow, &mut out);
            // Row 3: run it, and keep Clear and Tap where they always are.
            at(rows - 1, 0, DeckItem::ChaseGo, &mut out);
            at(rows - 1, cols - 2, DeckItem::Clear, &mut out);
            at(rows - 1, cols - 1, DeckItem::Tap, &mut out);
            return out;
        }

        let mut items: Vec<DeckItem> = vec![DeckItem::ChaseGo];
        items.extend(shapes.into_iter().map(DeckItem::ChaseShape));
        items.extend(CHASE_SPEEDS.into_iter().map(DeckItem::ChaseSpeed));
        items.push(DeckItem::ChaseDir);
        let n = rows * cols;
        if items.len() > n && n > 0 {
            items.truncate(n - 1);
            items.push(DeckItem::Tap);
        }
        for (i, item) in items.into_iter().enumerate().take(n) {
            out.push((i as u8, item));
        }
    out
}

impl App {
    /// How many pages the rig's effect islands fill on this deck (or a
    /// Plus XL grid when none is attached), so the Page knob knows its cycle.
    fn fx_page_count(&self) -> usize {
        let (rows, cols) = self.deck.kind().map_or((4, 9), |k| (k.row_count() as usize, k.column_count() as usize));
        if !(rows >= 4 && cols >= 7) {
            return 1;
        }
        let islands = effect_islands(&self.patch.fixtures);
        let mut sizes: Vec<usize> = islands.iter().map(|i| 1 + i.slots.len()).collect();
        let stored = self.wheel_palettes(&islands).len();
        if stored > 0 {
            sizes.push(1 + stored);
        }
        island_pages(&sizes, rows, cols, 4).len().max(1)
    }

    /// The Gobos / Effects sub-pages. Gobos: every gobo slot, then any
    /// stored palette that sets a gobo, filling the grid from the top left.
    /// Effects: islands laid out by [`island_pages`], each under its header,
    /// stored effect palettes as a final "STORED" island. The last four
    /// keys are the cycle's Go and Fade, Clear and Tap.
    fn wheel_layout(&self, rows: usize, cols: usize) -> Vec<(u8, DeckItem)> {
        let n = rows * cols;
        let big = rows >= 4 && cols >= 7;
        let reserved = if big { 4 } else { 2.min(n) };
        let islands = self.wheel_islands(self.palette_sub);
        let stored = self.wheel_palettes(&islands);
        let mut out: Vec<(u8, DeckItem)> = Vec::new();
        match self.palette_sub {
            PaletteSub::Effects(page) if big => {
                let mut sizes: Vec<usize> = islands.iter().map(|i| 1 + i.slots.len()).collect();
                if !stored.is_empty() {
                    sizes.push(1 + stored.len());
                }
                let pages = island_pages(&sizes, rows, cols, reserved);
                if let Some(p) = pages.get(page.min(pages.len().saturating_sub(1))) {
                    for (isl, keys) in p {
                        out.push((keys[0], DeckItem::Island(*isl)));
                        for (j, &k) in keys[1..].iter().enumerate() {
                            let item = if *isl < islands.len() {
                                DeckItem::Wheel(*isl, j)
                            } else {
                                DeckItem::WheelPalette(stored[j])
                            };
                            out.push((k, item));
                        }
                    }
                }
            }
            _ => {
                let mut items: Vec<DeckItem> = islands
                    .iter()
                    .enumerate()
                    .flat_map(|(i, isl)| (0..isl.slots.len()).map(move |j| DeckItem::Wheel(i, j)))
                    .collect();
                items.extend(stored.into_iter().map(DeckItem::WheelPalette));
                items.truncate(n.saturating_sub(reserved));
                out.extend(items.into_iter().enumerate().map(|(i, it)| (i as u8, it)));
            }
        }
        if big {
            out.push(((n - 4) as u8, DeckItem::CycleGo));
            out.push(((n - 3) as u8, DeckItem::CycleFade));
        }
        if n >= 2 {
            out.push(((n - 2) as u8, DeckItem::Clear));
        }
        if n >= 1 {
            out.push(((n - 1) as u8, DeckItem::Tap));
        }
        out
    }

    /// Maps deck key index -> what's there right now.
    ///
    /// On the Phaser page: every key maps 1:1 to the matching `phaser_deck`
    /// slot (empty slots included — pressing one is just a no-op).
    ///
    /// On the Colors page, on a deck wide/tall enough (the Stream Deck XL
    /// and similar): colour palettes fill a fixed 3-column box on the left;
    /// the rest of the grid is a control box with one group per row —
    /// Pattern, then Spacing, then Snap — and Tap sits alone in the very
    /// last key (bottom-right), with the rest of that row left blank. On a
    /// smaller deck (e.g. the Stream Deck Plus) there isn't room for both
    /// boxes, so it shows only the control list in key order, Tap last; if
    /// that list doesn't fit, it's truncated and Tap is forced into the
    /// final key so there's always a way to tap.
    fn deck_layout(&self) -> Vec<(u8, DeckItem)> {
        let Some(kind) = self.deck.kind() else { return Vec::new() };
        let rows = kind.row_count() as usize;
        let cols = kind.column_count() as usize;
        let mut out = Vec::new();

        if self.deck_page == DeckPage::Presets {
            return preset_layout(&preset_page_rows(&self.banks), rows, cols);
        }

        if self.deck_page == DeckPage::Phasers {
            // The keys show the current page of pads; the Page knob scrolls.
            let base = self.deck_phaser_page * PHASER_DECK_SLOTS;
            for k in 0..rows * cols {
                match phaser_page_key(k, rows, cols) {
                    PhaserKey::Tap => out.push((k as u8, DeckItem::Tap)),
                    PhaserKey::SelectAll => out.push((k as u8, DeckItem::SelectAll)),
                    PhaserKey::Clear => out.push((k as u8, DeckItem::Clear)),
                    PhaserKey::Pad(slot) if base + slot < self.phaser_deck.len() => {
                        out.push((k as u8, DeckItem::PhaserPad(base + slot)))
                    }
                    PhaserKey::Pad(_) => {}
                }
            }
            return out;
        }

        if self.deck_page == DeckPage::Chases {
            return chase_layout(rows, cols);
        }

        // The Palettes page: its wheel sub-pages, or (below) the colours.
        if self.palette_sub.is_wheel() {
            return self.wheel_layout(rows, cols);
        }

        if cols >= 7 && rows >= 4 {
            let left_cols = 3;
            let right_start = left_cols;
            let colors: Vec<&Palette> =
                self.palettes.iter().filter(|p| p.feature == Feature::Color).collect();
            let mut idx = 0;
            'left: for r in 0..rows {
                for c in 0..left_cols {
                    let Some(p) = colors.get(idx) else { break 'left };
                    out.push(((r * cols + c) as u8, DeckItem::Color(p.id)));
                    idx += 1;
                }
            }
            // One control group per row, left-aligned in the right box;
            // Tap is isolated alone in the grid's very last key.
            let pattern_row = 0;
            let spacing_row = 1;
            let snap_row = 2;
            let tap_key = (rows - 1) * cols + (cols - 1);
            for (c, pat) in
                [SeqPattern::Wave, SeqPattern::Wings, SeqPattern::Random].into_iter().enumerate()
            {
                if right_start + c < cols {
                    out.push(((pattern_row * cols + right_start + c) as u8, DeckItem::Pattern(pat)));
                }
            }
            for (c, (lv, _)) in LEVELS.iter().enumerate() {
                if right_start + c < cols {
                    out.push(((spacing_row * cols + right_start + c) as u8, DeckItem::Spacing(*lv)));
                }
            }
            for (c, (lv, _)) in LEVELS.iter().enumerate() {
                if right_start + c < cols {
                    out.push(((snap_row * cols + right_start + c) as u8, DeckItem::Snap(*lv)));
                }
            }
            out.push((tap_key as u8, DeckItem::Tap));
            out.push(((tap_key - 1) as u8, DeckItem::Clear));
            // Cycle Go and Fade complete the bottom-right corner.
            out.push(((tap_key - 3) as u8, DeckItem::CycleGo));
            out.push(((tap_key - 2) as u8, DeckItem::CycleFade));
        } else {
            let n = rows * cols;
            let mut items = control_items();
            if items.len() > n && n > 0 {
                items.truncate(n - 1);
                items.push(DeckItem::Tap);
            }
            for (i, item) in items.into_iter().enumerate() {
                out.push((i as u8, item));
            }
        }
        out
    }

    /// Builds the full desired key-image frame from current app state.
    fn deck_key_specs(&self) -> Vec<KeySpec> {
        let Some(kind) = self.deck.kind() else { return Vec::new() };
        let mut specs: Vec<KeySpec> =
            (0..kind.key_count()).map(|i| KeySpec::plain(i, [8, 8, 10], None)).collect();
        let flash = self.beat_taps.last().is_some_and(|t| t.elapsed().as_secs_f32() < 0.15);
        // The wheel sub-page's islands, gathered once rather than per key.
        let islands = if self.palette_sub.is_wheel() { self.wheel_islands(self.palette_sub) } else { Vec::new() };
        let island_accent = |i: usize| match self.palette_sub {
            PaletteSub::Gobos => GOBO_ACCENT,
            _ => ISLAND_ACCENTS[i % ISLAND_ACCENTS.len()],
        };
        let cycle_ready = self.cycle_on || self.cycle_ids.len() >= 2;
        for (key, item) in self.deck_layout() {
            let Some(slot) = specs.get_mut(key as usize) else { continue };
            *slot = match item {
                DeckItem::Preset(b, i) => {
                    let Some(p) = self.banks.get(b).and_then(|bk| bk.presets.get(i)) else {
                        continue;
                    };
                    let (rgb, label, icon) = preset_key_look(&p.name);
                    let on = self.active_preset == Some((b, i));
                    KeySpec {
                        index: key,
                        rgb: if on { rgb } else { mute_rgb(rgb) },
                        label: Some(label),
                        icon,
                        wheel: None,
                    }
                }
                DeckItem::Color(id) => {
                    let Some(p) = self.palettes.iter().find(|p| p.id == id) else { continue };
                    let base = self.palette_swatch(p);
                    let base = [base.r(), base.g(), base.b()];
                    let on = self.cycle_ids.contains(&id);
                    KeySpec::plain(key, if on { base } else { mute_rgb(base) }, None)
                }
                DeckItem::PhaserPad(idx) => match self.phaser_deck.get(idx).cloned().flatten() {
                    Some(s) => {
                        let on = self.active_phasers.contains_key(&s.phaser);
                        KeySpec {
                            index: key,
                            rgb: if on { s.color } else { mute_rgb(s.color) },
                            label: if s.label.is_empty() { None } else { Some(s.label) },
                            icon: s.icon,
                            wheel: None,
                        }
                    }
                    None => KeySpec::plain(key, [14, 14, 16], None),
                },
                DeckItem::SelectAll => {
                    let all = self.stage.all_selected();
                    let mut spec = control_spec(key, "ALL", all, SELECT_ALL_ACCENT);
                    spec.icon = KeyIcon::StageLight;
                    spec
                }
                DeckItem::CycleGo => {
                    let running = self.cycle_on && !self.cycle_fading_out();
                    let label = if self.cycle_on { "STOP" } else { "CYCLE" };
                    let mut spec = control_spec(key, label, running, CHASE_LIVE);
                    if !cycle_ready {
                        spec.rgb = dim([90, 90, 95]);
                    }
                    spec.icon = KeyIcon::Bullseye;
                    spec
                }
                DeckItem::CycleFade => {
                    let going_out = self.cycle_on && !self.cycle_fading_out();
                    let label = if going_out { "F-OUT" } else { "F-IN" };
                    let mut spec = control_spec(key, label, self.cycle_fade.is_some(), ACCENT_SOFT);
                    if !cycle_ready {
                        spec.rgb = dim([90, 90, 95]);
                    }
                    spec.icon = KeyIcon::Wave;
                    spec
                }
                DeckItem::Wheel(i, j) => {
                    let Some(isl) = islands.get(i) else { continue };
                    let Some(slot) = isl.slots.get(j) else { continue };
                    let on = self
                        .slot_palette_id(&isl.name, &slot.label)
                        .is_some_and(|id| self.lane_has(&isl.name, id));
                    let accent = island_accent(i);
                    KeySpec {
                        index: key,
                        rgb: if on { accent } else { mute_rgb(accent) },
                        label: Some(wheel_label(&slot.label)),
                        icon: KeyIcon::None,
                        wheel: Some((isl.name.clone(), slot.label.clone())),
                    }
                }
                DeckItem::Island(i) => {
                    // The header: the island's name written out, on a darker
                    // shade of its slots' colour. The index past the last
                    // island is the stored-palettes island.
                    let (name, accent) = match islands.get(i) {
                        Some(isl) => (isl.name.as_str(), island_accent(i)),
                        None => ("Stored", WHEEL_PALETTE_ACCENT),
                    };
                    header_spec(key, name, accent)
                }
                DeckItem::WheelPalette(id) => {
                    let Some(p) = self.palettes.iter().find(|p| p.id == id) else { continue };
                    let on = match stored_lane(self.palette_sub) {
                        Some(lane) => self.lane_has(lane, id),
                        None => self.cycle_ids.contains(&id),
                    };
                    let mut spec = control_spec(key, &wheel_label(&p.name), on, WHEEL_PALETTE_ACCENT);
                    spec.icon = KeyIcon::Bullseye;
                    spec
                }
                DeckItem::Clear => {
                    // Coloured by what the next press clears: encoders
                    // (amber), effects (blue), or the full blackout (red).
                    let accent = match self.clear_stage_next() {
                        ClearStage::Encoders => SNAP_ACCENT,
                        ClearStage::Effects => CHASE_ACCENT,
                        ClearStage::Blackout => CLEAR_BLACKOUT,
                    };
                    control_spec(key, "CLR", true, accent)
                }
                DeckItem::Pattern(pat) => {
                    control_spec(key, pattern_label(pat), self.cycle_pattern == pat, ACCENT_SOFT)
                }
                DeckItem::Spacing(v) => control_spec(
                    key,
                    &format!("SP{}", level_num(v)),
                    (self.cycle_spread - v).abs() < 0.05,
                    ACCENT_SOFT,
                ),
                DeckItem::Snap(v) => control_spec(
                    key,
                    &format!("SN{}", level_num(v)),
                    (self.cycle_shape - v).abs() < 0.05,
                    SNAP_ACCENT,
                ),
                DeckItem::Tap => KeySpec::plain(
                    key,
                    if flash { [255, 255, 255] } else { ACCENT },
                    Some("TAP".to_string()),
                ),
                DeckItem::ChaseShape(k) => {
                    let on = self.chase.kind == k;
                    let mut spec = control_spec(key, k.tag(), on, CHASE_ACCENT);
                    // The selected shape goes brighter still once it's running,
                    // so the deck shows what the rig is actually doing.
                    if on && self.chase.enabled {
                        spec.rgb = CHASE_LIVE;
                    }
                    spec
                }
                DeckItem::ChaseSpeed(s) => {
                    let step = CHASE_SPEEDS.iter().position(|v| *v == s).unwrap_or(0) + 1;
                    let on = (self.chase.speed - s).abs() < 0.001;
                    control_spec(key, &step.to_string(), on, CHASE_ACCENT)
                }
                DeckItem::ChaseWidth(w) => {
                    let step = CHASE_WIDTHS.iter().position(|v| *v == w).unwrap_or(0) + 1;
                    let on = !self.chase.auto_width && (self.chase.band_deg - w).abs() < 0.5;
                    control_spec(key, &format!("W{step}"), on, SNAP_ACCENT)
                }
                DeckItem::ChaseDir => {
                    let fwd = self.chase.direction >= 0.0;
                    control_spec(key, if fwd { "FWD" } else { "REV" }, true, SNAP_ACCENT)
                }
                DeckItem::ChaseSoft => {
                    let soft = self.chase.soft;
                    control_spec(key, if soft { "SOFT" } else { "HARD" }, true, SNAP_ACCENT)
                }
                DeckItem::ChaseAuto => {
                    control_spec(key, "AUTO", self.chase.auto_width, SNAP_ACCENT)
                }
                DeckItem::ChaseShow => {
                    let mut spec =
                        control_spec(key, "SHOW", self.chase.expanded, CHASE_ACCENT);
                    spec.icon = KeyIcon::Ring;
                    spec
                }
                DeckItem::ChaseGo => {
                    let running = self.chase.enabled;
                    let mut spec = control_spec(
                        key,
                        if running { "STOP" } else { "GO" },
                        running,
                        CHASE_LIVE,
                    );
                    // No preset to inject means Go has nothing to run.
                    if self.chase.source.is_none() {
                        spec.rgb = dim([90, 90, 95]);
                    }
                    spec.icon = KeyIcon::Burst;
                    spec
                }
            };
        }
        specs
    }

    /// Dispatches a deck key press to whatever it currently means.
    pub(crate) fn handle_deck_press(&mut self, key: u8) {
        let Some((_, item)) = self.deck_layout().into_iter().find(|(k, _)| *k == key) else {
            return;
        };
        match item {
            DeckItem::Preset(b, i) => self.apply_preset(b, i),
            DeckItem::Color(id) => self.toggle_cycle_palette(id),
            DeckItem::Wheel(i, j) => {
                let islands = self.wheel_islands(self.palette_sub);
                if let Some(slot) = islands.get(i).and_then(|isl| isl.slots.get(j)) {
                    let id = self.ensure_slot_palette(&islands[i].name, slot);
                    self.toggle_lane_palette(&islands[i].name, id);
                }
            }
            DeckItem::Island(_) => {}
            DeckItem::WheelPalette(id) => match stored_lane(self.palette_sub) {
                Some(lane) => self.toggle_lane_palette(lane, id),
                None => self.toggle_cycle_palette(id),
            },
            DeckItem::CycleGo => self.toggle_cycle(0.0),
            DeckItem::CycleFade => self.toggle_cycle(CYCLE_FADE_S),
            DeckItem::Pattern(pat) => self.cycle_pattern = pat,
            DeckItem::Spacing(v) => self.cycle_spread = v,
            DeckItem::Snap(v) => self.cycle_shape = v,
            DeckItem::Tap => self.beat_tap(),
            DeckItem::PhaserPad(idx) => {
                let Some(slot) = self.phaser_deck.get(idx).cloned().flatten() else { return };
                if self.active_phasers.contains_key(&slot.phaser) {
                    self.stop_phaser(&slot.phaser);
                } else if let Some(ph) = self.phasers.iter().find(|p| p.name == slot.phaser).cloned() {
                    self.apply_phaser(ph, false);
                }
            }
            DeckItem::Clear => self.clear_stage(),
            DeckItem::SelectAll => {
                let on = self.stage.select_all_fixtures();
                self.sync_selection_units();
                self.log.push(if on {
                    format!("Selected all {} lights", self.patch.fixtures.len())
                } else {
                    "Selection cleared".into()
                });
            }
            DeckItem::ChaseShape(k) => {
                self.chase.kind = k;
                // Swapping shape mid-run restarts it on the new one rather
                // than making you stop and go again.
                if self.chase.enabled {
                    self.start_chase();
                }
            }
            DeckItem::ChaseSpeed(s) => self.chase.speed = s,
            DeckItem::ChaseWidth(w) => {
                self.chase.band_deg = w;
                // Picking a width by hand is a clear vote against auto-fit.
                self.chase.auto_width = false;
            }
            DeckItem::ChaseDir => self.chase.direction = -self.chase.direction,
            DeckItem::ChaseSoft => self.chase.soft = !self.chase.soft,
            DeckItem::ChaseAuto => self.chase.auto_width = !self.chase.auto_width,
            DeckItem::ChaseShow => self.chase.expanded = !self.chase.expanded,
            DeckItem::ChaseGo => {
                if self.chase.enabled {
                    self.stop_chase();
                } else if self.chase.source.is_some() {
                    self.start_chase();
                } else {
                    self.log
                        .push("Chase: pick a preset to inject in the Chases window first".into());
                }
            }
        }
    }

    /// Move the chase's inject preset `delta` places along the source list,
    /// clamped at the ends so spinning past the last one doesn't wrap round
    /// to the top mid-show. Restarts a running chase on the new look.
    fn step_chase_source(&mut self, delta: i32) {
        let sources = self.chase_sources();
        if sources.is_empty() {
            self.log.push("Chase: no presets stored yet to inject".into());
            return;
        }
        let current = self
            .chase
            .source
            .and_then(|s| sources.iter().position(|(cs, _)| *cs == s));
        let next = match current {
            Some(i) => (i as i32 + delta).clamp(0, sources.len() as i32 - 1) as usize,
            // Nothing picked yet: the first nudge lands on either end.
            None if delta < 0 => sources.len() - 1,
            None => 0,
        };
        if current == Some(next) {
            return;
        }
        let (src, name) = sources[next].clone();
        self.chase.source = Some(src);
        self.log.push(format!("Chase injects \"{name}\""));
        if self.chase.enabled {
            self.start_chase();
        }
    }

    /// Builds the touch-strip image (Stream Deck Plus XL only) — see
    /// [`paint_strip`] for what goes on each panel.
    fn render_lcd_strip(&self) -> RgbImage {
        // Driven straight off the tap machinery (real elapsed time since the
        // last tap, at the tapped BPM) rather than `self.live.beat_phase()`
        // — that clock only advances while an oscillator is actually
        // assigned to the programmer, so with none running it just sits
        // still and the strip looked frozen. This ticks live off the wall
        // clock instead: tap a few beats to set the tempo/anchor, and it
        // keeps flashing on its own from there, re-anchoring on every tap.
        let pulse = match self.beat_taps.last() {
            Some(last) => {
                let period = 60.0 / self.master_bpm.max(1.0);
                let frac = (last.elapsed().as_secs_f32() / period).rem_euclid(1.0);
                // Sharp attack, quick decay — a flash right on the beat.
                (1.0 - frac * 0.85).clamp(0.0, 1.0).powf(2.0)
            }
            None => 0.0,
        };
        paint_strip(&StripState {
            level: self.grand_master.clamp(0.0, 1.0),
            blackout: self.blackout,
            bpm: self.master_bpm.round() as i32,
            pulse,
            frozen: self.frozen,
            page: self.deck_page,
            // On the Chases page the fourth knob picks the look being
            // injected, so its panel names that look instead.
            inject: (self.deck_page == DeckPage::Chases).then(|| self.chase_source_name()),
            encoder: self.encoder_readout().map(|(name, value, _)| (name, value)),
            sub: match self.deck_page {
                DeckPage::Palettes => {
                    let (pos, count) = self.palette_sub.position(self.fx_page_count());
                    Some((self.palette_sub.name(), pos, count))
                }
                // On the Phasers page the knob scrolls the pages of pads.
                DeckPage::Phasers => Some((
                    "PHASERS",
                    self.deck_phaser_page + 1,
                    phaser_deck_pages(&self.phaser_deck),
                )),
                _ => None,
            },
        })
    }

    /// Scroll the Phaser page's pads by `d` pages, wrapping at either end.
    pub(crate) fn step_phaser_deck_page(&mut self, d: i32) {
        let count = phaser_deck_pages(&self.phaser_deck) as i32;
        self.deck_phaser_page = (self.deck_phaser_page as i32 + d).rem_euclid(count) as usize;
    }

    /// Called once per frame from `update()`. Off: clears the deck exactly
    /// once on the falling edge. On: drains key presses, drains encoder
    /// turns/presses (Dimmer/Beat/Mode and the programmer knob wired up;
    /// Page is graphics-only for now), then republishes the key images and
    /// touch-strip.
    pub(crate) fn sync_deck(&mut self) {
        if !self.send_to_deck {
            if self.deck_active {
                self.deck.clear();
                self.deck_active = false;
            }
            return;
        }
        self.deck_active = true;
        for key in self.deck.take_presses() {
            self.handle_deck_press(key);
        }
        let deltas = self.deck.take_encoder_deltas();
        // Encoder 0 = Master Dimmer: same multiplicative, floor-anchored-at-
        // black grand master already used everywhere else, so a wave's
        // shape rides straight down with it instead of getting clipped.
        if let Some(&d) = deltas.first() {
            if d != 0 {
                self.grand_master = (self.grand_master + d as f32 * 0.02).clamp(0.0, 1.0);
            }
        }
        // Encoder 1 = Master Beat: one whole BPM per detent, so — like the
        // Dimmer knob — every click moves the number the readout shows.
        // (A 0.1/detent step here used to leave the rounded display looking
        // unresponsive for nine clicks out of ten.)
        if let Some(&d) = deltas.get(1) {
            if d != 0 {
                self.master_bpm = (self.master_bpm + d as f32).clamp(30.0, 300.0);
            }
        }
        // Encoder 3 is the Page knob everywhere else; on the Chases page it
        // scrolls the look the chase injects, which is otherwise the one
        // thing you have to go back to the laptop for.
        if self.deck_page == DeckPage::Chases {
            if let Some(&d) = deltas.get(3) {
                if d != 0 {
                    self.step_chase_source(d);
                }
            }
        }
        // On the Palettes page the Page knob flips between its sub-pages:
        // colours, gobos, prisms.
        if self.deck_page == DeckPage::Palettes {
            if let Some(&d) = deltas.get(3) {
                if d != 0 {
                    self.palette_sub = self.palette_sub.step(d, self.fx_page_count());
                }
            }
        }
        // On the Phasers page it scrolls through the pages of pads.
        if self.deck_page == DeckPage::Phasers {
            if let Some(&d) = deltas.get(3) {
                if d != 0 {
                    self.step_phaser_deck_page(d);
                }
            }
        }
        // Encoder 5 = the programmer knob: nudges one channel type across
        // the selected lights (see `encoder.rs`).
        if let Some(&d) = deltas.get(5) {
            if d != 0 {
                self.encoder_nudge(d);
            }
        }
        for enc in self.deck.take_encoder_presses() {
            match enc {
                0 => self.blackout = !self.blackout,
                1 => self.set_frozen(!self.frozen),
                // Encoder 2 = Mode: cycles Presets → Colors → Phasers → Chases.
                2 => self.deck_page = self.deck_page.next(),
                // Encoder 5 = programmer knob: step to the next channel type.
                5 => self.encoder_next_channel(),
                // 3 = Page: graphics only for now, no action yet. 4 is idle.
                _ => {}
            }
        }
        self.deck.set_keys(self.deck_key_specs());
        if self.deck.kind() == Some(Kind::PlusXl) {
            let img = self.render_lcd_strip();
            let (w, h) = (img.width(), img.height());
            self.deck.set_lcd(w, h, img.into_raw());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon has to rasterise cleanly at every key size the supported
    /// devices use — the shape maths is all float-to-pixel, so an off-by-one
    /// would panic inside `put_pixel` rather than just look wrong.
    #[test]
    fn key_artwork_rasterises_at_every_key_size() {
        for size in [72, 80, 96, 120] {
            for icon in KeyIcon::ALL {
                for label in [None, Some("S1".to_string())] {
                    let spec = KeySpec { index: 0, rgb: [81, 219, 219], label, icon, wheel: None };
                    let img = render_key(&spec, size);
                    assert_eq!(img.width(), size);
                    assert_eq!(img.height(), size);
                }
            }
        }
    }

    /// A deck saved before paging existed is exactly page 1; anything
    /// longer rounds up to whole pages so every page is fully addressable.
    #[test]
    fn phaser_deck_rounds_up_to_whole_pages() {
        for (len, pages) in [(0, 1), (10, 1), (36, 1), (37, 2), (72, 2), (100, 3)] {
            let mut deck: Vec<Option<PhaserSlot>> = vec![None; len];
            assert_eq!(phaser_deck_pages(&deck), pages, "len {len}");
            pad_phaser_deck(&mut deck);
            assert_eq!(deck.len(), pages * PHASER_DECK_SLOTS, "len {len}");
            assert_eq!(phaser_deck_pages(&deck), pages, "padded from {len}");
        }
    }

    /// Reserving the bottom corners must not renumber the pads above them —
    /// there are already phasers placed in rows 1-3, and shifting their slot
    /// indices would silently scramble the page.
    #[test]
    fn phaser_page_reserves_corners_without_moving_pads() {
        let (rows, cols) = (4usize, 9usize);
        let mut pads = Vec::new();
        for k in 0..rows * cols {
            match phaser_page_key(k, rows, cols) {
                PhaserKey::Pad(slot) => pads.push((k, slot)),
                PhaserKey::SelectAll => assert_eq!(k, 27, "select-all moved"),
                PhaserKey::Clear => assert_eq!(k, 34, "clear is not beside tap"),
                PhaserKey::Tap => assert_eq!(k, 35, "tap is not bottom-right"),
            }
        }
        // Rows 1-3 are untouched: key index == slot index.
        for &(k, slot) in &pads {
            if k < 27 {
                assert_eq!(k, slot, "pad at key {k} renumbered to slot {slot}");
            }
        }
        // And the slots that remain are contiguous with no gaps or repeats.
        let mut slots: Vec<usize> = pads.iter().map(|(_, s)| *s).collect();
        slots.sort_unstable();
        assert_eq!(slots, (0..slots.len()).collect::<Vec<_>>());
    }

    /// Tap sits in the same corner on every page — muscle memory is the whole
    /// point of a control surface.
    #[test]
    fn tap_is_bottom_right_on_every_page() {
        let (rows, cols) = (4usize, 9usize);
        let last = (rows * cols - 1) as u8;
        assert!(matches!(phaser_page_key(last as usize, rows, cols), PhaserKey::Tap));
        let chase_tap = chase_layout(rows, cols)
            .into_iter()
            .find(|(_, item)| matches!(item, DeckItem::Tap));
        assert_eq!(chase_tap.map(|(k, _)| k), Some(last), "chases page tap moved");
    }

    /// Clear sits just left of Tap on every page, for the same reason.
    #[test]
    fn clear_is_beside_tap_on_every_page() {
        let (rows, cols) = (4usize, 9usize);
        let beside = (rows * cols - 2) as u8;
        let last = (rows * cols - 1) as u8;
        assert!(matches!(phaser_page_key(beside as usize, rows, cols), PhaserKey::Clear));
        let chase_clear = chase_layout(rows, cols)
            .into_iter()
            .find(|(_, item)| matches!(item, DeckItem::Clear));
        assert_eq!(chase_clear.map(|(k, _)| k), Some(beside), "chases page clear moved");
        let presets = preset_layout(&preset_page_rows(&sample_banks()), rows, cols);
        let find = |want: fn(&DeckItem) -> bool| presets.iter().find(|(_, i)| want(i)).map(|(k, _)| *k);
        assert_eq!(find(|i| matches!(i, DeckItem::Clear)), Some(beside), "presets page clear moved");
        assert_eq!(find(|i| matches!(i, DeckItem::Tap)), Some(last), "presets page tap moved");
    }

    /// The Page knob wraps both ways round the Palettes sub-pages, however
    /// many effect pages the rig needs.
    #[test]
    fn palette_sub_steps_round() {
        assert_eq!(PaletteSub::Colors.step(1, 2), PaletteSub::Gobos);
        assert_eq!(PaletteSub::Gobos.step(1, 2), PaletteSub::Effects(0));
        assert_eq!(PaletteSub::Effects(0).step(1, 2), PaletteSub::Effects(1));
        assert_eq!(PaletteSub::Effects(1).step(1, 2), PaletteSub::Colors);
        assert_eq!(PaletteSub::Colors.step(-1, 2), PaletteSub::Effects(1));
        assert_eq!(PaletteSub::Effects(0).position(1), (3, 3));
        // An empty rig still has one (empty) effects page.
        assert_eq!(PaletteSub::Gobos.step(1, 0), PaletteSub::Effects(0));
        assert_eq!(PaletteSub::Effects(0).step(1, 0), PaletteSub::Colors);
    }

    /// Islands start rows cleanly, wrap when longer than a row, never
    /// straddle a page, and leave a blank key between them.
    #[test]
    fn island_pages_keep_islands_whole() {
        let pages = island_pages(&[4, 6, 5, 13, 7], 4, 9, 4);
        assert_eq!(pages.len(), 2, "{pages:?}");
        let first = &pages[0];
        assert_eq!(first[0].1, vec![0, 1, 2, 3]);
        // 6 keys don't fit after the gap at key 5, so the next row.
        assert_eq!(first[1].1[0], 9);
        assert_eq!(first[2].1, vec![18, 19, 20, 21, 22]);
        // 13 keys: longer than a row, and no room left on this page.
        let second = &pages[1];
        assert_eq!(second[0].0, 3);
        assert_eq!(second[0].1.len(), 13);
        assert_eq!(second[0].1[0], 0);
        assert_eq!(second[1].1[0], 18);
        // Everything stays clear of the four reserved keys.
        for p in &pages {
            for (_, keys) in p {
                assert!(keys.iter().all(|&k| (k as usize) < 32));
            }
        }
    }

    /// Gobo names from this rig's fixture files, as they come.
    const RIG_GOBOS: [&str; 20] = [
        "Spot", "Small", "Flwr", "Vortex", "Daisy", "dotring", "invVortex", "Sun", "Sspot",
        "Ssmall", "Sflw", "Svort", "notes", "squiggle", "X", "star", "mercedes", "octagon",
        "auto s>f", "mystery",
    ];
    const RIG_PRISMS: [&str; 3] = ["off", "on", "spins>f"];

    fn wheel_spec(key: u8, island: &str, name: &str, on: bool, accent: [u8; 3]) -> KeySpec {
        KeySpec {
            index: key,
            rgb: if on { accent } else { mute_rgb(accent) },
            label: Some(wheel_label(name)),
            icon: KeyIcon::None,
            wheel: Some((island.to_string(), name.to_string())),
        }
    }

    /// Effect islands shaped like this rig's, for the sheets.
    const RIG_EFFECTS: [(&str, &[&str]); 7] = [
        ("Prism", &["off", "on", "spins>f"]),
        ("Spinner", &["off", "on s>f", "low", "mid", "full"]),
        ("Light ring", &["off", "low", "mid", "full"]),
        ("Macro", &["none", "aut1", "aut2", "aut3", "aut4", "s1", "s2", "s3", "s4", "colorselect", "color combo", "color jump"]),
        ("Auto", &["off", "on", "clrcmbo", "colorjmp", "gradient", "gradient2", "low", "mid", "full"]),
        ("Fog", &["none", "low", "medium", "high"]),
        ("Circle", &["off", "low", "mid", "full"]),
    ];

    /// Every wheel picture rasterises, for every name on this rig and at
    /// every key size — the shapes are float-to-pixel like the icons.
    #[test]
    fn wheel_art_rasterises_for_rig_names() {
        for size in [72, 80, 96, 120] {
            for n in RIG_GOBOS {
                let img = render_key(&wheel_spec(0, "Gobo", n, true, GOBO_ACCENT), size);
                assert_eq!(img.width(), size);
            }
            for (island, names) in RIG_EFFECTS {
                for n in names.iter() {
                    let img = render_key(&wheel_spec(0, island, n, false, ISLAND_ACCENTS[0]), size);
                    assert_eq!(img.width(), size);
                }
                let img = render_key(&header_spec(0, island, ISLAND_ACCENTS[1]), size);
                assert_eq!(img.width(), size);
            }
        }
    }

    /// Renders the Gobos sub-page for this rig, with the prisms squeezed
    /// into the spare bottom-row keys so both sets of pictures can be judged.
    #[test]
    fn dump_wheel_page_sheet() {
        const SIZE: u32 = 96;
        const GAP: u32 = 4;
        let (rows, cols) = (4usize, 9usize);
        let mut sheet = RgbImage::from_pixel(
            cols as u32 * (SIZE + GAP) + GAP,
            rows as u32 * (SIZE + GAP) + GAP,
            image::Rgb([24, 24, 28]),
        );
        let mut specs: Vec<KeySpec> = RIG_GOBOS
            .iter()
            .enumerate()
            .map(|(i, n)| wheel_spec(i as u8, "Gobo", n, i == 3 || i == 7, GOBO_ACCENT))
            .collect();
        specs.extend(RIG_PRISMS.iter().enumerate().map(|(i, n)| wheel_spec(27 + i as u8, "Prism", n, i == 1, ISLAND_ACCENTS[0])));
        let mut go = control_spec(32, "STOP", true, CHASE_LIVE);
        go.icon = KeyIcon::Bullseye;
        let mut fade = control_spec(33, "F-OUT", false, ACCENT_SOFT);
        fade.icon = KeyIcon::Wave;
        specs.push(go);
        specs.push(fade);
        specs.push(control_spec(34, "CLR", true, SNAP_ACCENT));
        specs.push(KeySpec::plain(35, ACCENT, Some("TAP".to_string())));
        for spec in specs {
            let img = render_key(&spec, SIZE).into_rgb8();
            let key = spec.index as u32;
            let (r, c) = (key / cols as u32, key % cols as u32);
            let (ox, oy) = (GAP + c * (SIZE + GAP), GAP + r * (SIZE + GAP));
            for y in 0..SIZE {
                for x in 0..SIZE {
                    sheet.put_pixel(ox + x, oy + y, *img.get_pixel(x, y));
                }
            }
        }
        let path = std::env::temp_dir().join("dmxpress_wheel_page.png");
        sheet.save(&path).expect("write wheel page sheet");
        println!("wheel page sheet: {}", path.display());
    }

    /// Renders the first Effects page as islands, each under its header,
    /// so the pictures and the clustering can be judged without a deck.
    #[test]
    fn dump_effects_page_sheet() {
        const SIZE: u32 = 96;
        const GAP: u32 = 4;
        let (rows, cols) = (4usize, 9usize);
        let sizes: Vec<usize> = RIG_EFFECTS.iter().map(|(_, n)| 1 + n.len()).collect();
        let pages = island_pages(&sizes, rows, cols, 4);
        assert!(pages.len() >= 2, "this rig's effects should need two pages");
        for (pi, page) in pages.iter().enumerate() {
            let mut sheet = RgbImage::from_pixel(
                cols as u32 * (SIZE + GAP) + GAP,
                rows as u32 * (SIZE + GAP) + GAP,
                image::Rgb([24, 24, 28]),
            );
            let mut specs: Vec<KeySpec> = Vec::new();
            for (isl, keys) in page {
                let (name, names) = RIG_EFFECTS[*isl];
                let accent = ISLAND_ACCENTS[isl % ISLAND_ACCENTS.len()];
                specs.push(header_spec(keys[0], name, accent));
                for (j, &k) in keys[1..].iter().enumerate() {
                    specs.push(wheel_spec(k, name, names[j], j == 1, accent));
                }
            }
            let mut go = control_spec(32, "CYCLE", false, CHASE_LIVE);
            go.icon = KeyIcon::Bullseye;
            let mut fade = control_spec(33, "F-IN", false, ACCENT_SOFT);
            fade.icon = KeyIcon::Wave;
            specs.push(go);
            specs.push(fade);
            specs.push(control_spec(34, "CLR", true, CHASE_ACCENT));
            specs.push(KeySpec::plain(35, ACCENT, Some("TAP".to_string())));
            for spec in specs {
                let img = render_key(&spec, SIZE).into_rgb8();
                let key = spec.index as u32;
                let (r, c) = (key / cols as u32, key % cols as u32);
                let (ox, oy) = (GAP + c * (SIZE + GAP), GAP + r * (SIZE + GAP));
                for y in 0..SIZE {
                    for x in 0..SIZE {
                        sheet.put_pixel(ox + x, oy + y, *img.get_pixel(x, y));
                    }
                }
            }
            let path = std::env::temp_dir().join(format!("dmxpress_effects_page_{}.png", pi + 1));
            sheet.save(&path).expect("write effects page sheet");
            println!("effects page sheet: {}", path.display());
        }
    }

    /// Banks shaped like the ones on this machine's ShowBuddy.
    fn sample_banks() -> Vec<PresetBank> {
        let bank = |name: &str, presets: &[&str]| PresetBank {
            name: name.into(),
            order: 0,
            presets: presets
                .iter()
                .map(|p| crate::showbuddy::PresetRef {
                    name: (*p).into(),
                    path: std::path::PathBuf::new(),
                    data: None,
                })
                .collect(),
        };
        vec![
            bank("General 100", &["Red 100", "Blue 100", "Magenta 100", "Green 100", "Cyan 100", "Yellow 100", "white 100", "white fade"]),
            bank("immersive party", &["magenta waves", "straight yellow"]),
            bank("OFF", &["OFF"]),
            bank("General 50", &["Green 50", "Yellow 50", "Red 50", "Magenta 50", "Blue 50", "Cyan 50"]),
            bank("Rainbow + crazy Stuff", &["rainbow moving slow", "Rainbow 2", "white strobe"]),
        ]
    }

    /// The 50s line up under the 100s by colour, 100s on top; OFF and the
    /// rainbows share the third row; nothing else from ShowBuddy leaks in.
    #[test]
    fn preset_rows_line_up_by_hue() {
        let banks = sample_banks();
        let rows = preset_page_rows(&banks);
        assert_eq!(rows.len(), 3);
        let name = |item: &Option<(usize, usize)>| {
            item.map(|(b, p)| banks[b].presets[p].name.as_str()).unwrap_or("-")
        };
        let row: Vec<&str> = rows[0].iter().map(name).collect();
        assert_eq!(row[..3], ["Red 100", "Blue 100", "Magenta 100"]);
        let row: Vec<&str> = rows[1].iter().map(name).collect();
        assert_eq!(row, ["Red 50", "Blue 50", "Magenta 50", "Green 50", "Cyan 50", "Yellow 50"]);
        let row: Vec<&str> = rows[2].iter().map(name).collect();
        assert_eq!(row, ["OFF", "rainbow moving slow", "Rainbow 2"]);
    }

    /// A gap opens where a colour is missing from a later row, so the
    /// columns still mean one hue each.
    #[test]
    fn preset_rows_keep_gaps_for_missing_hues() {
        let mut banks = sample_banks();
        banks[3].presets.retain(|p| p.name != "Blue 50");
        let rows = preset_page_rows(&banks);
        assert!(rows[1][1].is_none(), "expected a gap under Blue");
        assert_eq!(rows[1].len(), 6);
    }

    #[test]
    fn preset_key_look_reads_names() {
        let (rgb, label, icon) = preset_key_look("Red 100");
        assert_eq!((rgb, label.as_str(), icon), ([255, 60, 50], "100", KeyIcon::None));
        let (dim, label, _) = preset_key_look("Red 50");
        assert_eq!(label, "50");
        assert!(dim[0] < rgb[0], "50 should be dimmer than 100");
        assert_eq!(preset_key_look("white fade").1, "FADE");
        assert_eq!(preset_key_look("OFF"), (PRESET_OFF_KEY, "OFF".into(), KeyIcon::None));
        let (_, label, icon) = preset_key_look("rainbow moving slow");
        assert_eq!((label.as_str(), icon), ("SLOW", KeyIcon::Rainbow));
        assert_eq!(preset_key_look("Rainbow 2").1, "2");
    }

    /// Renders the Presets page as it lands on a Plus XL, Red 100 active.
    #[test]
    fn dump_preset_page_sheet() {
        const SIZE: u32 = 96;
        const GAP: u32 = 4;
        let (rows, cols) = (4usize, 9usize);
        let banks = sample_banks();
        let mut sheet = RgbImage::from_pixel(
            cols as u32 * (SIZE + GAP) + GAP,
            rows as u32 * (SIZE + GAP) + GAP,
            image::Rgb([24, 24, 28]),
        );
        for (key, item) in preset_layout(&preset_page_rows(&banks), rows, cols) {
            let spec = match item {
                DeckItem::Preset(b, p) => {
                    let (rgb, label, icon) = preset_key_look(&banks[b].presets[p].name);
                    let on = (b, p) == (0, 0);
                    KeySpec { index: key, rgb: if on { rgb } else { mute_rgb(rgb) }, label: Some(label), icon, wheel: None }
                }
                DeckItem::Clear => control_spec(key, "CLR", true, SNAP_ACCENT),
                DeckItem::Tap => KeySpec::plain(key, ACCENT, Some("TAP".to_string())),
                _ => continue,
            };
            let img = render_key(&spec, SIZE).into_rgb8();
            let (r, c) = (key as u32 / cols as u32, key as u32 % cols as u32);
            let (ox, oy) = (GAP + c * (SIZE + GAP), GAP + r * (SIZE + GAP));
            for y in 0..SIZE {
                for x in 0..SIZE {
                    sheet.put_pixel(ox + x, oy + y, *img.get_pixel(x, y));
                }
            }
        }
        let path = std::env::temp_dir().join("dmxpress_preset_page.png");
        sheet.save(&path).expect("write preset page sheet");
        println!("preset page sheet: {}", path.display());
    }

    /// Renders the Phaser page from the page actually saved on disk, so the
    /// reserved corners can be checked in context.
    #[test]
    fn dump_phaser_page_sheet() {
        const SIZE: u32 = 96;
        const GAP: u32 = 4;
        let (rows, cols) = (4usize, 9usize);
        let deck = load_phaser_deck();
        // Every page, stacked top to bottom, in the order the Page knob scrolls.
        let pages = phaser_deck_pages(&deck);
        let page_h = rows as u32 * (SIZE + GAP) + GAP;
        let mut sheet = RgbImage::from_pixel(
            cols as u32 * (SIZE + GAP) + GAP,
            pages as u32 * page_h,
            image::Rgb([24, 24, 28]),
        );
        for (page, k) in (0..pages).flat_map(|p| (0..rows * cols).map(move |k| (p, k))) {
            let key = k as u8;
            let spec = match phaser_page_key(k, rows, cols) {
                PhaserKey::Tap => KeySpec::plain(key, ACCENT, Some("TAP".to_string())),
                PhaserKey::SelectAll => {
                    let mut s = control_spec(key, "ALL", true, SELECT_ALL_ACCENT);
                    s.icon = KeyIcon::StageLight;
                    s
                }
                PhaserKey::Clear => control_spec(key, "CLR", true, SNAP_ACCENT),
                PhaserKey::Pad(slot) => match deck.get(page * PHASER_DECK_SLOTS + slot).cloned().flatten() {
                    Some(p) => KeySpec {
                        index: key,
                        rgb: mute_rgb(p.color),
                        label: (!p.label.is_empty()).then_some(p.label),
                        icon: p.icon,
                        wheel: None,
                    },
                    None => KeySpec::plain(key, [14, 14, 16], None),
                },
            };
            let img = render_key(&spec, SIZE).into_rgb8();
            let (r, c) = (k as u32 / cols as u32, k as u32 % cols as u32);
            let (ox, oy) = (GAP + c * (SIZE + GAP), page as u32 * page_h + GAP + r * (SIZE + GAP));
            for y in 0..SIZE {
                for x in 0..SIZE {
                    sheet.put_pixel(ox + x, oy + y, *img.get_pixel(x, y));
                }
            }
        }
        let path = std::env::temp_dir().join("dmxpress_phaser_page.png");
        sheet.save(&path).expect("write phaser page sheet");
        println!("phaser page sheet: {}", path.display());
    }

    /// Names have to stay readable on a 200px panel, including the awkward
    /// two-word ones the user actually has ("Magenta 100").
    #[test]
    fn inject_panel_names_split_sensibly() {
        assert_eq!(split_label("Blue"), ("BLUE".into(), None));
        assert_eq!(
            split_label("Magenta 100"),
            ("MAGENTA".into(), Some("100".into()))
        );
        // No space to break on: one line, truncated rather than squeezed.
        let (one, two) = split_label("Supercalifragilistic");
        assert!(two.is_none());
        assert!(one.chars().count() <= 9, "{one} too long to read");
    }

    /// Renders the strip in four states, stacked, so the artwork can be
    /// judged without a device: idle on Colors; blackout + frozen on Phasers;
    /// Chases with an inject preset; Chases with none picked.
    #[test]
    fn dump_strip_sheet() {
        let states = [
            StripState { level: 0.85, blackout: false, bpm: 128, pulse: 1.0, frozen: false, page: DeckPage::Presets, inject: None, encoder: None, sub: None },
            StripState { level: 0.3, blackout: false, bpm: 128, pulse: 0.1, frozen: false, page: DeckPage::Palettes, inject: None, encoder: None, sub: Some(("EFFECTS", 3, 4)) },
            StripState { level: 0.5, blackout: true, bpm: 96, pulse: 0.0, frozen: true, page: DeckPage::Phasers, inject: None, encoder: None, sub: Some(("PHASERS", 2, 3)) },
            StripState { level: 1.0, blackout: false, bpm: 140, pulse: 0.5, frozen: false, page: DeckPage::Chases, inject: Some(Some("Magenta 100".into())), encoder: Some(("Gobo".into(), "Gobo 3 shake".into())), sub: None },
            StripState { level: 0.0, blackout: false, bpm: 60, pulse: 0.0, frozen: false, page: DeckPage::Chases, inject: Some(None), encoder: Some(("Dimmer".into(), "128".into())), sub: None },
        ];
        let gap = 6u32;
        let mut sheet = RgbImage::from_pixel(LCD_W, states.len() as u32 * (LCD_H + gap), image::Rgb([30, 30, 34]));
        let cold = std::time::Instant::now();
        for (i, st) in states.iter().enumerate() {
            let strip = paint_strip(st);
            blit(&mut sheet, &strip, 0, i as u32 * (LCD_H + gap));
        }
        let cold = cold.elapsed() / states.len() as u32;
        // Second pass hits the panel cache — this is the steady-state cost.
        let warm = std::time::Instant::now();
        for st in &states {
            let _ = paint_strip(st);
        }
        let warm = warm.elapsed() / states.len() as u32;
        let path = std::env::temp_dir().join("dmxpress_strip.png");
        sheet.save(&path).expect("write strip sheet");
        println!("strip sheet: {} (cold {cold:?}, cached {warm:?} per frame)", path.display());
    }

    /// Renders the Chases page exactly as it would land on a Plus XL, so the
    /// layout can be judged without a device attached. The chase state is a
    /// plausible resting one: Sphere selected, medium speed, not running.
    #[test]
    fn dump_chase_page_sheet() {
        const SIZE: u32 = 96;
        const GAP: u32 = 4;
        let (rows, cols) = (4usize, 9usize);
        let mut sheet = RgbImage::from_pixel(
            cols as u32 * (SIZE + GAP) + GAP,
            rows as u32 * (SIZE + GAP) + GAP,
            image::Rgb([24, 24, 28]),
        );
        for (key, item) in chase_layout(rows, cols) {
            let spec = match item {
                DeckItem::ChaseShape(k) => {
                    control_spec(key, k.tag(), k == ChaseKind::Sphere, CHASE_ACCENT)
                }
                DeckItem::ChaseSpeed(s) => {
                    let step = CHASE_SPEEDS.iter().position(|v| *v == s).unwrap_or(0) + 1;
                    control_spec(key, &step.to_string(), (s - 0.2).abs() < 0.001, CHASE_ACCENT)
                }
                DeckItem::ChaseWidth(w) => {
                    let step = CHASE_WIDTHS.iter().position(|v| *v == w).unwrap_or(0) + 1;
                    control_spec(key, &format!("W{step}"), (w - 60.0).abs() < 0.5, SNAP_ACCENT)
                }
                DeckItem::ChaseDir => control_spec(key, "FWD", true, SNAP_ACCENT),
                DeckItem::ChaseSoft => control_spec(key, "SOFT", true, SNAP_ACCENT),
                DeckItem::ChaseAuto => control_spec(key, "AUTO", false, SNAP_ACCENT),
                DeckItem::ChaseShow => {
                    let mut s = control_spec(key, "SHOW", true, CHASE_ACCENT);
                    s.icon = KeyIcon::Ring;
                    s
                }
                DeckItem::ChaseGo => {
                    let mut s = control_spec(key, "GO", false, CHASE_LIVE);
                    s.icon = KeyIcon::Burst;
                    s
                }
                DeckItem::Tap => KeySpec::plain(key, ACCENT, Some("TAP".to_string())),
                DeckItem::Clear => control_spec(key, "CLR", true, SNAP_ACCENT),
                _ => continue,
            };
            let img = render_key(&spec, SIZE).into_rgb8();
            let (r, c) = (key as u32 / cols as u32, key as u32 % cols as u32);
            let (ox, oy) = (GAP + c * (SIZE + GAP), GAP + r * (SIZE + GAP));
            for y in 0..SIZE {
                for x in 0..SIZE {
                    sheet.put_pixel(ox + x, oy + y, *img.get_pixel(x, y));
                }
            }
        }
        let path = std::env::temp_dir().join("dmxpress_chase_page.png");
        sheet.save(&path).expect("write chase page sheet");
        println!("chase page sheet: {}", path.display());
    }

    /// Writes a contact sheet of the key artwork (lit on top, idle below) so
    /// the art can be eyeballed without a Stream Deck plugged in.
    #[test]
    fn dump_key_artwork_sheet() {
        const SIZE: u32 = 96;
        const GAP: u32 = 4;
        let icons = KeyIcon::ALL;
        let w = icons.len() as u32 * (SIZE + GAP) + GAP;
        let h = 2 * (SIZE + GAP) + GAP;
        let mut sheet = RgbImage::from_pixel(w, h, image::Rgb([24, 24, 28]));
        let lit = [81, 219, 219];
        for (col, icon) in icons.into_iter().enumerate() {
            for (row, rgb) in [lit, mute_rgb(lit)].into_iter().enumerate() {
                let spec = KeySpec { index: 0, rgb, label: Some("S1".to_string()), icon, wheel: None };
                let key = render_key(&spec, SIZE).into_rgb8();
                let ox = GAP + col as u32 * (SIZE + GAP);
                let oy = GAP + row as u32 * (SIZE + GAP);
                for y in 0..SIZE {
                    for x in 0..SIZE {
                        sheet.put_pixel(ox + x, oy + y, *key.get_pixel(x, y));
                    }
                }
            }
        }
        let path = std::env::temp_dir().join("dmxpress_key_artwork.png");
        sheet.save(&path).expect("write contact sheet");
        println!("key artwork sheet: {}", path.display());
    }
}
