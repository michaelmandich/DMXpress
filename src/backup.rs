//! Show safety: rolling autosaves, one-click restore, and a show as one file.
//!
//! The show lives in two dozen JSON files that the console rewrites as you
//! work, so a slip — a wrong Load, a crash mid-save, an experiment gone too
//! far — used to be final. Now every minute in which something changed
//! (and every exit) writes the whole show, as a [`Configuration`], into
//! `backups/`, keeping the newest [`KEEP`]. Restoring one, importing a show
//! file, and Ctrl+Shift+Z all go through the same restore path, and a
//! restore or import first backs up what is being replaced.
//!
//! Exports are the same file written into `exports/` with a friendly name,
//! and the folder is opened so it can be copied or sent. Importing is a
//! drop onto the window or a pasted path — no file dialogs, which keeps the
//! Linux build free of a GTK dependency.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config::Configuration;

pub const BACKUPS_DIR: &str = "backups";
pub const EXPORTS_DIR: &str = "exports";
/// Newest backups kept; older ones are deleted as new ones are written.
pub const KEEP: usize = 20;
/// Least time between two autosaves.
pub const INTERVAL: Duration = Duration::from_secs(60);

/// Tracks when the show last hit disk as a backup, so autosaves only run
/// when something actually changed.
pub struct Autosave {
    last_run: Instant,
    saved_hash: Option<u64>,
    /// The most recent write, for the status line.
    pub last_written: Option<(PathBuf, Instant)>,
    /// One warning per failure, not one per minute.
    failed: bool,
}

impl Default for Autosave {
    fn default() -> Self {
        Self::new()
    }
}

impl Autosave {
    pub fn new() -> Self {
        Self { last_run: Instant::now(), saved_hash: None, last_written: None, failed: false }
    }

    /// The state at launch counts as saved; it is what the files hold.
    pub fn baseline(&mut self, hash: Option<u64>) {
        if self.saved_hash.is_none() {
            self.saved_hash = hash;
        }
    }

    pub fn changed(&self, hash: Option<u64>) -> bool {
        hash.is_some() && hash != self.saved_hash
    }

    /// Time for another autosave: something changed and the interval is up.
    pub fn due(&self, hash: Option<u64>) -> bool {
        self.changed(hash) && self.last_run.elapsed() >= INTERVAL
    }

    pub fn mark(&mut self, hash: Option<u64>, path: PathBuf) {
        self.saved_hash = hash;
        self.last_run = Instant::now();
        self.last_written = Some((path, Instant::now()));
        self.failed = false;
    }

    /// A write failed: wait a full interval before trying again, and say
    /// so once.
    pub fn defer(&mut self) -> bool {
        self.last_run = Instant::now();
        let first = !self.failed;
        self.failed = true;
        first
    }
}

/// One file in the backups folder.
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub modified: SystemTime,
    pub bytes: u64,
}

/// Write `cfg` as `backups/<kind>-<stamp>.json` and prune to [`KEEP`].
pub fn write(kind: &str, cfg: &Configuration) -> std::io::Result<PathBuf> {
    let dir = crate::paths::data_path(BACKUPS_DIR);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{kind}-{}.json", stamp(SystemTime::now())));
    let bytes = serde_json::to_vec(cfg).map_err(std::io::Error::other)?;
    // Write beside, then rename: a crash mid-write never leaves a half file
    // under the name a restore would pick.
    let tmp = path.with_extension("json.part");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)?;
    prune(KEEP);
    Ok(path)
}

/// Every backup, newest first.
pub fn list() -> Vec<Entry> {
    let Ok(dir) = std::fs::read_dir(crate::paths::data_path(BACKUPS_DIR)) else { return Vec::new() };
    let mut out: Vec<Entry> = dir
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "json") {
                return None;
            }
            let meta = e.metadata().ok()?;
            Some(Entry {
                name: path.file_stem()?.to_string_lossy().into_owned(),
                modified: meta.modified().ok()?,
                bytes: meta.len(),
                path,
            })
        })
        .collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

/// Delete all but the newest `keep` backups.
pub fn prune(keep: usize) {
    for old in list().into_iter().skip(keep) {
        let _ = std::fs::remove_file(old.path);
    }
}

/// Read a show file — a backup, an export, or a saved configuration.
pub fn read(path: &Path) -> Result<Configuration, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("not a DMXpress show file ({e})"))
}

/// Write `cfg` as one readable file in `exports/`, named after the show.
pub fn export(name: &str, cfg: &Configuration) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(crate::paths::data_path(EXPORTS_DIR))?;
    let safe: String = name
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_()".contains(c) { c } else { '_' })
        .collect();
    let safe = if safe.trim().is_empty() { "show".to_string() } else { safe.trim().to_string() };
    let path = crate::paths::data_path(EXPORTS_DIR)
        .join(format!("{safe} {}.dmxpress.json", stamp(SystemTime::now())));
    let text = serde_json::to_string_pretty(cfg).map_err(std::io::Error::other)?;
    std::fs::write(&path, text)?;
    Ok(path)
}

/// Show `path` in the operating system's file manager (its folder, with the
/// file selected where the platform can). Best effort; never blocks.
pub fn reveal(path: &Path) {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    // canonicalize on Windows yields a `\\?\` prefix Explorer refuses.
    let shown = abs.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    let result = if cfg!(target_os = "windows") {
        if abs.is_dir() {
            std::process::Command::new("explorer").arg(&shown).spawn()
        } else {
            std::process::Command::new("explorer").arg(format!("/select,{shown}")).spawn()
        }
    } else if cfg!(target_os = "macos") {
        if abs.is_dir() {
            std::process::Command::new("open").arg(&shown).spawn()
        } else {
            std::process::Command::new("open").arg("-R").arg(&shown).spawn()
        }
    } else {
        let dir = if abs.is_dir() { abs.clone() } else { abs.parent().map_or(abs.clone(), Path::to_path_buf) };
        std::process::Command::new("xdg-open").arg(dir).spawn()
    };
    let _ = result;
}

/// `20260914-210311`: a UTC stamp that sorts as it reads.
pub fn stamp(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (y, mo, d, h, mi, s) = civil(secs);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// "just now", "4 min ago", "yesterday" — for the backups list.
pub fn ago(t: SystemTime) -> String {
    let s = SystemTime::now().duration_since(t).map_or(0, |d| d.as_secs());
    match s {
        0..=9 => "just now".into(),
        10..=59 => format!("{s} s ago"),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86399 => format!("{} h ago", s / 3600),
        86400..=172799 => "yesterday".into(),
        _ => format!("{} days ago", s / 86400),
    }
}

/// Seconds since the epoch to (year, month, day, hour, minute, second) in
/// UTC. Howard Hinnant's days-to-civil arithmetic; no calendar crate needed.
fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d, h as u32, mi as u32, s as u32)
}

impl crate::app::App {
    /// Called every frame from the undo tick: write an autosave when the
    /// show has changed and a minute has passed since the last one.
    pub(crate) fn autosave_tick(&mut self) {
        let hash = self.undo.hash();
        self.autosave.baseline(hash);
        if !self.autosave.due(hash) {
            return;
        }
        match write("autosave", &self.snapshot_configuration()) {
            Ok(path) => self.autosave.mark(hash, path),
            Err(e) => {
                if self.autosave.defer() {
                    self.log.push(format!("Autosave failed: {e}"));
                }
            }
        }
    }

    /// A backup right now, if anything changed since the last one — used on
    /// exit and before anything that replaces the show.
    pub(crate) fn backup_now_if_changed(&mut self, kind: &str) -> Option<PathBuf> {
        let json = self.snapshot_json();
        let hash = Some(crate::undo::fingerprint(&json));
        if !self.autosave.changed(hash) && self.autosave.last_written.is_some() {
            return None;
        }
        match write(kind, &self.snapshot_configuration()) {
            Ok(path) => {
                self.autosave.mark(hash, path.clone());
                Some(path)
            }
            Err(e) => {
                self.log.push(format!("Backup failed: {e}"));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_read_as_utc_dates() {
        assert_eq!(stamp(UNIX_EPOCH), "19700101-000000");
        assert_eq!(stamp(UNIX_EPOCH + Duration::from_secs(1_700_000_000)), "20231114-221320");
        // A leap day, and the day after.
        assert_eq!(stamp(UNIX_EPOCH + Duration::from_secs(1_709_164_800)), "20240229-000000");
        assert_eq!(stamp(UNIX_EPOCH + Duration::from_secs(1_709_251_200)), "20240301-000000");
    }

    #[test]
    fn ago_reads_naturally() {
        let now = SystemTime::now();
        assert_eq!(ago(now), "just now");
        assert_eq!(ago(now - Duration::from_secs(45)), "45 s ago");
        assert_eq!(ago(now - Duration::from_secs(180)), "3 min ago");
        assert_eq!(ago(now - Duration::from_secs(7200)), "2 h ago");
        assert_eq!(ago(now - Duration::from_secs(100_000)), "yesterday");
    }

    #[test]
    fn autosave_waits_for_a_change() {
        let mut a = Autosave::new();
        a.baseline(Some(1));
        assert!(!a.changed(Some(1)));
        assert!(a.changed(Some(2)));
        // The interval gates it even when changed.
        assert!(!a.due(Some(2)));
        a.mark(Some(2), PathBuf::from("x"));
        assert!(!a.changed(Some(2)));
    }
}
