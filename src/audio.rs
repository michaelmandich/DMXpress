//! Audio in: capture any computer audio source, watch its spectrum, follow
//! its beat, and fire looks from frequency bands.
//!
//! Capture runs on its own thread (cpal streams are not `Send`); the UI only
//! ever reads a small snapshot behind a mutex. On Windows an *output* device
//! can be opened for loopback capture, which is how "play a YouTube video and
//! light it" works without any cabling. On macOS/Linux system audio needs a
//! virtual device (BlackHole, monitor sources); microphones work everywhere.
//!
//! Analysis per hop (~11 ms): Hann-windowed FFT folded into 64 log-spaced
//! bins (30 Hz..16 kHz, dB-mapped against a slowly decaying reference so the
//! display and triggers are level-independent), plus spectral flux feeding a
//! predict-and-correct beat tracker. Detected beats increment a counter the
//! app turns into taps on the existing beat machinery — the tracker is just a
//! very patient drummer pressing the TAP button.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::engine::{Blend, Layer};
use crate::net::Frame;

pub const AUDIO_FILE: &str = "audio.json";

/// Spectrum resolution shared by the analyser, the graph and the triggers.
pub const BINS: usize = 64;
/// Log-spaced bin edges.
pub const FREQ_LO: f32 = 30.0;
pub const FREQ_HI: f32 = 16_000.0;

const FFT_SIZE: usize = 2048;
const HOP: usize = 512;
/// Flux history for tempo estimation (~5.5 s at 48 kHz).
const ENV_LEN: usize = 512;

/// Frequency of the log-bin position `x` in 0..1.
pub fn x_to_hz(x: f32) -> f32 {
    FREQ_LO * (FREQ_HI / FREQ_LO).powf(x.clamp(0.0, 1.0))
}

/// Inverse of [`x_to_hz`].
pub fn hz_to_x(hz: f32) -> f32 {
    (hz.clamp(FREQ_LO, FREQ_HI) / FREQ_LO).ln() / (FREQ_HI / FREQ_LO).ln()
}

// ---------------------------------------------------------------- devices --

/// One selectable capture source.
#[derive(Clone, PartialEq, Eq)]
pub struct AudioSource {
    pub name: String,
    /// Open an output device for loopback (system audio) instead of an input.
    pub loopback: bool,
}

impl AudioSource {
    pub fn label(&self) -> String {
        if self.loopback {
            format!("System audio: {}", self.name)
        } else {
            format!("Input: {}", self.name)
        }
    }
}

/// Enumerate microphones/inputs plus (on Windows) every output device as a
/// loopback source.
pub fn list_sources() -> Vec<AudioSource> {
    let host = cpal::default_host();
    let mut out = Vec::new();
    if cfg!(target_os = "windows") {
        if let Ok(devs) = host.output_devices() {
            for d in devs {
                if let Ok(name) = d.name() {
                    out.push(AudioSource {
                        name,
                        loopback: true,
                    });
                }
            }
        }
    }
    if let Ok(devs) = host.input_devices() {
        for d in devs {
            if let Ok(name) = d.name() {
                out.push(AudioSource {
                    name,
                    loopback: false,
                });
            }
        }
    }
    out
}

// --------------------------------------------------------------- analysis --

/// What the UI and triggers read, refreshed every hop.
#[derive(Clone)]
pub struct Analysis {
    /// Log-spaced band levels, 0..1 relative to the recent programme peak.
    pub spectrum: [f32; BINS],
    /// Overall level 0..1.
    pub level: f32,
    /// Tracker tempo estimate.
    pub bpm: f32,
    /// 0..1 — how periodic the music actually is.
    pub confidence: f32,
}

impl Default for Analysis {
    fn default() -> Self {
        Self {
            spectrum: [0.0; BINS],
            level: 0.0,
            bpm: 0.0,
            confidence: 0.0,
        }
    }
}

struct Shared {
    running: AtomicBool,
    /// Total beats emitted since start; the app tap-syncs on increments.
    beats: AtomicU64,
    analysis: Mutex<Analysis>,
    error: Mutex<Option<String>>,
}

enum Ctrl {
    Start(AudioSource),
    Stop,
}

/// Handle owned by the App; the stream and DSP live on the engine thread.
pub struct AudioEngine {
    shared: Arc<Shared>,
    ctrl: crossbeam_channel::Sender<Ctrl>,
    pub source_label: Option<String>,
}

impl AudioEngine {
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            running: AtomicBool::new(false),
            beats: AtomicU64::new(0),
            analysis: Mutex::new(Analysis::default()),
            error: Mutex::new(None),
        });
        let (ctrl, ctrl_rx) = crossbeam_channel::unbounded();
        let thread_shared = shared.clone();
        std::thread::Builder::new()
            .name("audio-engine".into())
            .spawn(move || engine_thread(thread_shared, ctrl_rx))
            .expect("spawn audio engine");
        Self {
            shared,
            ctrl,
            source_label: None,
        }
    }

    pub fn start(&mut self, source: AudioSource) {
        self.source_label = Some(source.label());
        *self.shared.error.lock() = None;
        let _ = self.ctrl.send(Ctrl::Start(source));
    }

    pub fn stop(&mut self) {
        self.source_label = None;
        let _ = self.ctrl.send(Ctrl::Stop);
    }

    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Relaxed)
    }

    pub fn beats(&self) -> u64 {
        self.shared.beats.load(Ordering::Relaxed)
    }

    pub fn analysis(&self) -> Analysis {
        self.shared.analysis.lock().clone()
    }

    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().clone()
    }
}

/// Owns the cpal stream and the DSP state; everything stays on this thread.
fn engine_thread(shared: Arc<Shared>, ctrl: crossbeam_channel::Receiver<Ctrl>) {
    let mut stream: Option<cpal::Stream> = None;
    let (sample_tx, sample_rx) = crossbeam_channel::unbounded::<Vec<f32>>();
    let mut dsp = Dsp::new(48_000.0);

    loop {
        // Apply any pending control messages (non-blocking once running).
        let msg = if stream.is_some() {
            ctrl.try_recv().ok()
        } else {
            // Idle: block so the thread costs nothing.
            match ctrl.recv() {
                Ok(m) => Some(m),
                Err(_) => return,
            }
        };
        match msg {
            Some(Ctrl::Start(source)) => {
                stream = None; // drop any old stream first
                while sample_rx.try_recv().is_ok() {}
                match open_stream(&source, sample_tx.clone()) {
                    Ok((s, rate)) => {
                        dsp = Dsp::new(rate);
                        if s.play().is_ok() {
                            stream = Some(s);
                            shared.running.store(true, Ordering::Relaxed);
                        } else {
                            *shared.error.lock() = Some("could not start the stream".into());
                        }
                    }
                    Err(e) => {
                        *shared.error.lock() = Some(format!("{e:#}"));
                        shared.running.store(false, Ordering::Relaxed);
                    }
                }
            }
            Some(Ctrl::Stop) => {
                stream = None;
                shared.running.store(false, Ordering::Relaxed);
                *shared.analysis.lock() = Analysis::default();
            }
            None => {}
        }

        if stream.is_none() {
            continue;
        }

        // Drain whatever the callback delivered, then process full hops.
        match sample_rx.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(chunk) => {
                dsp.push(&chunk);
                while sample_rx.try_recv().map(|c| dsp.push(&c)).is_ok() {}
                let beats = dsp.process(&mut shared.analysis.lock());
                for _ in 0..beats {
                    shared.beats.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Open `source` for capture, downmixing to mono f32 chunks.
fn open_stream(
    source: &AudioSource,
    tx: crossbeam_channel::Sender<Vec<f32>>,
) -> anyhow::Result<(cpal::Stream, f32)> {
    let host = cpal::default_host();
    let devices: Vec<cpal::Device> = if source.loopback {
        host.output_devices()?.collect()
    } else {
        host.input_devices()?.collect()
    };
    let device = devices
        .into_iter()
        .find(|d| d.name().map(|n| n == source.name).unwrap_or(false))
        .ok_or_else(|| anyhow::anyhow!("device \"{}\" not found", source.name))?;
    // Loopback opens the device's *output* format as an input stream.
    let config = if source.loopback {
        device.default_output_config()?
    } else {
        device.default_input_config()?
    };
    let rate = config.sample_rate().0 as f32;
    let channels = config.channels() as usize;
    let err_fn = |e| eprintln!("audio stream error: {e}");

    macro_rules! build {
        ($t:ty, $to_f32:expr) => {
            device.build_input_stream(
                &config.into(),
                move |data: &[$t], _: &cpal::InputCallbackInfo| {
                    let mono: Vec<f32> = data
                        .chunks(channels.max(1))
                        .map(|frame| {
                            frame.iter().map(|&s| $to_f32(s)).sum::<f32>()
                                / frame.len().max(1) as f32
                        })
                        .collect();
                    let _ = tx.send(mono);
                },
                err_fn,
                None,
            )?
        };
    }

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build!(f32, |s: f32| s),
        cpal::SampleFormat::I16 => build!(i16, |s: i16| s as f32 / 32768.0),
        cpal::SampleFormat::U16 => build!(u16, |s: u16| (s as f32 - 32768.0) / 32768.0),
        other => anyhow::bail!("unsupported sample format {other:?}"),
    };
    Ok((stream, rate))
}

// -------------------------------------------------------------------- DSP --

struct Dsp {
    rate: f32,
    ring: VecDeque<f32>,
    window: Vec<f32>,
    fft: std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    scratch: Vec<f32>,
    out: Vec<realfft::num_complex::Complex<f32>>,
    prev_mag: Vec<f32>,
    /// Which log bin each FFT bin lands in (None below/above range).
    bin_map: Vec<Option<usize>>,
    /// Reference peak per display update, decayed slowly (auto gain).
    ref_db: f32,
    smoothed: [f32; BINS],
    /// Onset envelope (one flux value per hop).
    env: VecDeque<f32>,
    /// Sample-clock time in seconds.
    t: f64,
    hops: usize,
    bpm: f32,
    confidence: f32,
    next_beat: f64,
    last_peak: f64,
    prev_flux: [f32; 3],
}

impl Dsp {
    fn new(rate: f32) -> Self {
        let mut planner = realfft::RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let out = fft.make_output_vec();
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let x = i as f32 / (FFT_SIZE - 1) as f32;
                0.5 - 0.5 * (std::f32::consts::TAU * x).cos()
            })
            .collect();
        let bin_map: Vec<Option<usize>> = (0..out.len())
            .map(|i| {
                let hz = i as f32 * rate / FFT_SIZE as f32;
                (hz >= FREQ_LO && hz <= FREQ_HI)
                    .then(|| ((hz_to_x(hz) * BINS as f32) as usize).min(BINS - 1))
            })
            .collect();
        Self {
            rate,
            ring: VecDeque::with_capacity(FFT_SIZE * 4),
            window,
            fft,
            scratch: vec![0.0; FFT_SIZE],
            prev_mag: vec![0.0; out.len()],
            out,
            bin_map,
            ref_db: -30.0,
            smoothed: [0.0; BINS],
            env: VecDeque::with_capacity(ENV_LEN),
            t: 0.0,
            hops: 0,
            bpm: 0.0,
            confidence: 0.0,
            next_beat: f64::MAX,
            last_peak: -10.0,
            prev_flux: [0.0; 3],
        }
    }

    fn push(&mut self, samples: &[f32]) {
        self.ring.extend(samples.iter().copied());
        // Never let a stall grow the ring unboundedly.
        while self.ring.len() > FFT_SIZE * 8 {
            self.ring.pop_front();
        }
    }

    /// Process every complete hop; returns how many beats were emitted.
    fn process(&mut self, analysis: &mut Analysis) -> u32 {
        let mut beats = 0;
        while self.ring.len() >= FFT_SIZE {
            beats += self.hop();
            for _ in 0..HOP.min(self.ring.len()) {
                self.ring.pop_front();
            }
        }
        analysis.spectrum = self.smoothed;
        analysis.level = self.smoothed.iter().copied().fold(0.0f32, f32::max);
        analysis.bpm = self.bpm;
        analysis.confidence = self.confidence;
        beats
    }

    fn hop(&mut self) -> u32 {
        self.t += HOP as f64 / self.rate as f64;
        self.hops += 1;
        for (i, s) in self.scratch.iter_mut().enumerate() {
            *s = self.ring[i] * self.window[i];
        }
        if self.fft.process(&mut self.scratch, &mut self.out).is_err() {
            return 0;
        }

        // Fold magnitudes into log bins and accumulate spectral flux.
        let mut bins = [0.0f32; BINS];
        let mut counts = [0u16; BINS];
        let mut flux = 0.0;
        for (i, c) in self.out.iter().enumerate() {
            let mag = c.norm();
            flux += (mag - self.prev_mag[i]).max(0.0);
            self.prev_mag[i] = mag;
            if let Some(b) = self.bin_map[i] {
                bins[b] += mag;
                counts[b] += 1;
            }
        }

        // dB map against a slowly decaying reference: level-independent 0..1.
        let mut peak_db = -90.0f32;
        for (b, bin) in bins.iter_mut().enumerate() {
            let avg = *bin / counts[b].max(1) as f32;
            let db = 20.0 * (avg + 1e-7).log10();
            peak_db = peak_db.max(db);
            *bin = db;
        }
        self.ref_db = (self.ref_db - 0.02).max(peak_db).max(-60.0);
        for (b, bin) in bins.iter().enumerate() {
            let v = ((bin - (self.ref_db - 50.0)) / 50.0).clamp(0.0, 1.0);
            // Fast attack, gentle release, so hits read but do not strobe.
            let s = &mut self.smoothed[b];
            *s = if v > *s { v } else { *s * 0.72 + v * 0.28 };
        }

        // Onset envelope, normalised by its own recent scale.
        if self.env.len() >= ENV_LEN {
            self.env.pop_front();
        }
        self.env.push_back(flux);
        self.track_beat()
    }

    /// Predict-and-correct beat tracking over the flux envelope.
    fn track_beat(&mut self) -> u32 {
        let n = self.env.len();
        // A flux peak one hop back (needs a successor to be a local max).
        self.prev_flux.rotate_left(1);
        self.prev_flux[2] = *self.env.back().unwrap_or(&0.0);
        let (a, b, c) = (self.prev_flux[0], self.prev_flux[1], self.prev_flux[2]);
        let mean = self.env.iter().sum::<f32>() / n.max(1) as f32;
        let var = self.env.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n.max(1) as f32;
        let is_peak = b > a && b >= c && b > mean + 1.4 * var.sqrt();
        let hop_dt = HOP as f64 / self.rate as f64;
        if is_peak {
            self.last_peak = self.t - hop_dt;
        }

        // Re-estimate tempo every ~0.35 s once the window has substance.
        if n >= ENV_LEN / 2 && self.hops % 32 == 0 {
            self.estimate_tempo(mean);
        }
        if self.bpm <= 0.0 || self.confidence < 0.15 {
            self.next_beat = f64::MAX;
            return 0;
        }

        let interval = 60.0 / self.bpm as f64;
        if self.next_beat == f64::MAX {
            // Arm on the next strong peak so beat one lands on a hit.
            if is_peak {
                self.next_beat = self.t + interval;
                return 1;
            }
            return 0;
        }
        if self.t >= self.next_beat {
            self.next_beat += interval;
            // Pull the prediction toward the nearest actual hit.
            let err = self.last_peak - (self.next_beat - interval);
            if err.abs() < interval * 0.35 {
                self.next_beat += err * 0.4;
            }
            // Fell badly behind (silence, device stall): re-arm instead of
            // machine-gunning catch-up beats.
            if self.t - self.next_beat > interval {
                self.next_beat = f64::MAX;
            }
            return 1;
        }
        0
    }

    /// Autocorrelation of the onset envelope over 70..180 BPM.
    fn estimate_tempo(&mut self, mean: f32) {
        let env: Vec<f32> = self.env.iter().map(|v| v - mean).collect();
        let hop_dt = HOP as f32 / self.rate;
        let lag_lo = (60.0 / 180.0 / hop_dt) as usize;
        let lag_hi = ((60.0 / 70.0 / hop_dt) as usize).min(env.len() / 2);
        if lag_hi <= lag_lo {
            return;
        }
        let mut best = (0usize, 0.0f32);
        let mut sum = 0.0;
        for lag in lag_lo..=lag_hi {
            let mut acc = 0.0;
            for i in lag..env.len() {
                acc += env[i] * env[i - lag];
            }
            let mut score = acc / (env.len() - lag) as f32;
            // Mild stickiness: prefer staying near the current estimate.
            if self.bpm > 0.0 {
                let bpm = 60.0 / (lag as f32 * hop_dt);
                if (bpm / self.bpm - 1.0).abs() < 0.06 {
                    score *= 1.2;
                }
            }
            sum += score.max(0.0);
            if score > best.1 {
                best = (lag, score);
            }
        }
        if best.0 == 0 || best.1 <= 0.0 {
            self.confidence = 0.0;
            return;
        }
        let avg = sum / (lag_hi - lag_lo + 1) as f32;
        self.confidence = (best.1 / (avg + 1e-9) / 6.0).clamp(0.0, 1.0);
        self.bpm = 60.0 / (best.0 as f32 * hop_dt);
    }
}

// ----------------------------------------------------------------- triggers

/// How a trigger turns band energy into a layer weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TriggerMode {
    /// On while the band is above the threshold (attack/release shaped).
    #[default]
    Gate,
    /// Fire once each time the band crosses the threshold, then decay.
    Pulse,
    /// The weight continuously follows how far above the threshold the band is.
    Level,
}

impl TriggerMode {
    pub const ALL: [Self; 3] = [Self::Gate, Self::Pulse, Self::Level];

    pub fn label(self) -> &'static str {
        match self {
            Self::Gate => "Gate",
            Self::Pulse => "Pulse",
            Self::Level => "Level",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Gate => "Holds while the band stays above the line",
            Self::Pulse => "One hit per crossing, then it decays away",
            Self::Level => "Rides the band: louder = stronger",
        }
    }
}

/// What the trigger paints onto its target when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerSource {
    /// A palette, by stable id (colours, positions — any stored feature).
    Palette(u32),
    /// A native preset's base values, by pool index.
    Preset(usize),
}

/// One rule on the graph: when this band exceeds this level, paint this
/// look onto these lights.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioTrigger {
    pub name: String,
    pub enabled: bool,
    pub lo_hz: f32,
    pub hi_hz: f32,
    /// 0..1 against the normalised spectrum.
    pub threshold: f32,
    pub mode: TriggerMode,
    pub attack_s: f32,
    pub release_s: f32,
    /// Group-pool index; `None` = every light the source knows about.
    pub group: Option<usize>,
    pub source: Option<TriggerSource>,
    /// How the flash folds over the show (defaults to highest-takes).
    #[serde(default)]
    pub merge: crate::scene::MergeMode,
    #[serde(skip)]
    pub env: f32,
    #[serde(skip)]
    pub prev_energy: f32,
}

impl AudioTrigger {
    pub fn new(n: usize) -> Self {
        Self {
            name: format!("Trigger {n}"),
            enabled: true,
            lo_hz: 60.0,
            hi_hz: 150.0,
            threshold: 0.7,
            mode: TriggerMode::Gate,
            attack_s: 0.02,
            release_s: 0.30,
            group: None,
            source: None,
            merge: crate::scene::MergeMode::Highest,
            env: 0.0,
            prev_energy: 0.0,
        }
    }

    /// Mean level of the spectrum bins inside this trigger's band.
    pub fn energy(&self, spectrum: &[f32; BINS]) -> f32 {
        let lo = ((hz_to_x(self.lo_hz) * BINS as f32) as usize).min(BINS - 1);
        let hi = ((hz_to_x(self.hi_hz) * BINS as f32).ceil() as usize).clamp(lo + 1, BINS);
        let slice = &spectrum[lo..hi];
        slice.iter().sum::<f32>() / slice.len().max(1) as f32
    }

    /// Advance the envelope by `dt` given the band `energy`; returns weight.
    pub fn advance(&mut self, energy: f32, dt: f32) -> f32 {
        let crossed = self.prev_energy < self.threshold && energy >= self.threshold;
        self.prev_energy = energy;
        let target = match self.mode {
            TriggerMode::Gate => {
                // Hysteresis: releases a little under the line, so it does
                // not chatter right at the threshold.
                if energy >= self.threshold || (self.env > 0.5 && energy >= self.threshold * 0.85)
                {
                    1.0
                } else {
                    0.0
                }
            }
            TriggerMode::Pulse => {
                if crossed {
                    // The hit lands at full; decay starts next frame.
                    self.env = 1.0;
                    return self.env;
                }
                0.0
            }
            TriggerMode::Level => ((energy - self.threshold)
                / (1.0 - self.threshold).max(0.05))
            .clamp(0.0, 1.0),
        };
        let tau = if target > self.env {
            self.attack_s.max(0.005)
        } else {
            self.release_s.max(0.02)
        };
        let k = 1.0 - (-dt / tau).exp();
        self.env += (target - self.env) * k;
        if self.env < 0.002 {
            self.env = 0.0;
        }
        self.env
    }

    /// Build this trigger's mixer layer from resolved `values` at `weight`.
    pub fn layer(values: &[(usize, u8)], weight: f32, merge: crate::scene::MergeMode) -> Option<Layer> {
        if values.is_empty() || weight <= 0.0 {
            return None;
        }
        let mut frame = Frame::black();
        let mut weights = Vec::with_capacity(values.len());
        for &(a, v) in values {
            if a < frame.len() {
                frame[a] = v;
                weights.push((a, weight));
            }
        }
        let blend = match merge {
            crate::scene::MergeMode::Override => Blend::Mix,
            crate::scene::MergeMode::Highest => Blend::Max,
            crate::scene::MergeMode::Add => Blend::Add,
        };
        Some(Layer::overlay(frame, weights).with_blend(blend))
    }
}

// ------------------------------------------------------------- persistence

/// Machine-local audio state: device names differ per machine, so this stays
/// out of shared show configurations on purpose.
#[derive(Default, Serialize, Deserialize)]
pub struct AudioFile {
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub loopback: bool,
    #[serde(default)]
    pub follow_beat: bool,
    #[serde(default)]
    pub triggers: Vec<AudioTrigger>,
}

pub fn load_audio() -> AudioFile {
    std::fs::read_to_string(AUDIO_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_audio(file: &AudioFile) {
    if let Ok(json) = serde_json::to_string_pretty(file) {
        let _ = std::fs::write(AUDIO_FILE, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequency_mapping_roundtrips() {
        for hz in [30.0, 100.0, 440.0, 1000.0, 8000.0, 16000.0] {
            let back = x_to_hz(hz_to_x(hz));
            assert!((back / hz - 1.0).abs() < 1e-3, "{hz} Hz came back as {back}");
        }
        assert_eq!(hz_to_x(FREQ_LO), 0.0);
        assert!((hz_to_x(FREQ_HI) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn gate_holds_and_pulse_decays() {
        let mut gate = AudioTrigger::new(1);
        gate.mode = TriggerMode::Gate;
        gate.threshold = 0.5;
        gate.attack_s = 0.01;
        gate.release_s = 0.1;
        // Above the line: rises toward 1.
        for _ in 0..40 {
            gate.advance(0.8, 0.02);
        }
        assert!(gate.env > 0.9);
        // Hysteresis: just under the line still holds.
        gate.advance(0.45, 0.02);
        assert!(gate.env > 0.8);
        // Well under: releases.
        for _ in 0..200 {
            gate.advance(0.1, 0.02);
        }
        assert!(gate.env < 0.05);

        let mut pulse = AudioTrigger::new(2);
        pulse.mode = TriggerMode::Pulse;
        pulse.threshold = 0.5;
        pulse.release_s = 0.05;
        pulse.advance(0.2, 0.02);
        pulse.advance(0.9, 0.02); // crossing fires it
        assert!(pulse.env > 0.9);
        // Staying above the line does not re-fire; it decays away.
        for _ in 0..200 {
            pulse.advance(0.9, 0.02);
        }
        assert!(pulse.env < 0.05);
    }
}
