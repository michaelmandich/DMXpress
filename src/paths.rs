//! Where the show's files live.
//!
//! Every pool the console persists — `presets.json`, `settings.json`, the
//! `backups/` and `setups/` folders and the rest — is named by a bare
//! relative name, and [`data_path`] is the one place that turns such a name
//! into a path to open. By default it resolves against the process
//! directory, exactly as a bare name always did, so a build launched inside
//! a show folder goes on writing there.
//!
//! Two things move it. `DMXPRESS_DATA_DIR` names the directory outright,
//! which is how a tool works on a real show without being launched from
//! inside it. And under `cargo test` the default becomes a scratch directory
//! belonging to that one test process: the directory a test run inherits is
//! the repository, ~300 tests build an `App`, and `App::new` re-saves every
//! pool it loads — so without this the suite would quietly overwrite the
//! operator's show, down to their chosen settings, every time it ran.
//!
//! The bundled assets under `fixtures/` — the fixture library and the gobo
//! pack — are not show data. The app only ever reads them, so they stay
//! where they ship, beside the process directory.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Names the data directory, overriding both defaults below.
const DATA_DIR_VAR: &str = "DMXPRESS_DATA_DIR";

/// The directory show data is read from and written to. Empty when it is
/// simply the process directory, so that the paths reaching the filesystem
/// stay the bare names they have always been.
pub fn data_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| match std::env::var_os(DATA_DIR_VAR) {
        Some(dir) if !dir.is_empty() => {
            let dir = PathBuf::from(dir);
            let _ = std::fs::create_dir_all(&dir);
            dir
        }
        _ => default_dir(),
    })
}

/// The path to open for the show file or folder called `name`.
pub fn data_path(name: impl AsRef<Path>) -> PathBuf {
    data_dir().join(name)
}

/// The process directory, which `resolve_data_dir` may already have moved to
/// the per-user data folder.
#[cfg(not(test))]
fn default_dir() -> PathBuf {
    PathBuf::new()
}

/// A scratch directory of this test process's own, holding a copy of the
/// show to work on. Emptied first: a pid comes round again eventually, and
/// a run must not inherit an older one's pools.
#[cfg(test)]
fn default_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dmxpress-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    seed(&dir);
    dir
}

/// Copy the show in the process directory into `dir`.
///
/// The suite is not run against an empty console: the repository carries a
/// show of its own — a patched rig, palettes, phasers, saved setups — and
/// tests assert against it, down to how many gobo wheels the patch has. So
/// the scratch directory starts as a copy of that show rather than as
/// nothing, and every test reads exactly what it always read while its
/// writes land on the copy.
///
/// Only the show is taken: the pools at the top level and the folders the
/// app saves into. What the app merely writes out — `backups/`, `exports/`,
/// `screenshots/` — is left behind, and so are the bundled `fixtures/`,
/// which are read where they ship.
#[cfg(test)]
fn seed(dir: &Path) {
    let Ok(here) = std::fs::read_dir(".") else { return };
    for entry in here.flatten() {
        let from = entry.path();
        let Some(name) = from.file_name() else { continue };
        let keep = match from.extension().and_then(|e| e.to_str()) {
            Some("json") => from.is_file(),
            _ => from.is_dir() && matches!(name.to_str(), Some("configs" | "setups" | "plugins")),
        };
        if !keep {
            continue;
        }
        let to = dir.join(name);
        if from.is_dir() {
            copy_dir(&from, &to);
        } else {
            let _ = std::fs::copy(&from, &to);
        }
    }
}

/// Copy `from` into `to`, recursively, best-effort.
#[cfg(test)]
fn copy_dir(from: &Path, to: &Path) {
    let _ = std::fs::create_dir_all(to);
    let Ok(entries) = std::fs::read_dir(from) else { return };
    for entry in entries.flatten() {
        let src = entry.path();
        let Some(name) = src.file_name() else { continue };
        let dst = to.join(name);
        if src.is_dir() {
            copy_dir(&src, &dst);
        } else {
            let _ = std::fs::copy(&src, &dst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The suite must never resolve a pool to a bare name: that would be
    /// the repository the tests run in, holding the operator's show.
    #[test]
    fn show_files_land_outside_the_working_directory() {
        assert!(!data_dir().as_os_str().is_empty(), "the show is the working directory");
        let path = data_path("presets.json");
        assert_ne!(path, Path::new("presets.json"));
        assert_eq!(path.parent(), Some(data_dir()));
    }

    /// A folder resolves the same way a file does, so `backups/` and
    /// `setups/` follow the pools rather than staying behind.
    #[test]
    fn folders_resolve_against_the_same_directory() {
        assert_eq!(data_path("backups"), data_dir().join("backups"));
    }

    /// The copy is what the suite reads: the tests that assert against the
    /// repository's own show need it there. Its contents are not compared
    /// with the original — by the time this runs, other tests have saved
    /// over the copy, which is the whole point of there being one.
    #[test]
    fn the_show_is_copied_into_the_scratch_directory() {
        if std::env::var_os(DATA_DIR_VAR).is_some_and(|d| !d.is_empty()) {
            // A run pointed at a directory of its own gets that directory
            // as it found it; nothing is copied over what is already there.
            return;
        }
        for pool in ["settings.json", "presets.json", "showbuddy_cache.json"] {
            let copied = data_path(pool);
            assert!(copied.is_file(), "{} was not seeded", copied.display());
        }
        assert!(data_path("setups").is_dir(), "setups/ was not seeded");
    }
}
