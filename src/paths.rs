//! Portable on-disk locations for Anvil's persistent state.
//!
//! Anvil is designed to be self-contained: everything it downloads or
//! persists lives in a single `anvil-data` directory beside the
//! executable, so deleting the program's folder removes the program and
//! all of its state. The only deliberate exception is transient ZIP
//! extraction, which stays in the OS temp directory and is auto-deleted.
//!
//! Resolution order for [`anvil_data_dir`]:
//!
//! 1. `ANVIL_DATA_DIR`, for tests, development and unusual deployments.
//! 2. `<folder of the canonical executable>/anvil-data`, when it can be
//!    created and written to. `current_exe` is canonicalised first so a
//!    symlink in a `bin` directory resolves to the real install folder.
//! 3. The legacy per-user cache root (`%LOCALAPPDATA%\anvil`,
//!    `~/Library/Caches/anvil`, `$XDG_CACHE_HOME/anvil` or
//!    `~/.cache/anvil`). This is a fallback for a read-only install
//!    folder, and a one-time notice reports it so the location is never a
//!    surprise.
//!
//! [`legacy_data_dir`] is exposed so a model downloaded before the
//! portable layout can still be read in place instead of re-downloaded.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Once;

/// Folder created next to the executable.
pub const DATA_DIR_NAME: &str = "anvil-data";

/// Why no writable data directory could be resolved.
#[derive(Debug)]
pub enum PathsError {
    /// Neither the executable's folder nor any fallback cache location
    /// could be created and written to.
    Unavailable,
}

impl fmt::Display for PathsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathsError::Unavailable => write!(
                f,
                "cannot create a writable Anvil data directory next to the program; set ANVIL_DATA_DIR"
            ),
        }
    }
}

impl std::error::Error for PathsError {}

static FALLBACK_NOTICE: Once = Once::new();

/// The directory that holds all of Anvil's persistent state.
pub fn anvil_data_dir() -> Result<PathBuf, PathsError> {
    let override_dir = std::env::var_os("ANVIL_DATA_DIR").map(PathBuf::from);
    let exe = std::env::current_exe().ok();
    resolve(override_dir, exe.as_deref())
}

/// The model cache root, `<data>/models`. Both optional Ask backends store
/// their downloaded files here.
pub fn model_root() -> Result<PathBuf, PathsError> {
    Ok(anvil_data_dir()?.join("models"))
}

/// The per-user cache root used before the portable layout. Kept only so an
/// already-downloaded model can be read without moving gigabytes. `None`
/// when the platform has no standard cache location.
pub fn legacy_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("anvil"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches/anvil"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(cache) = std::env::var_os("XDG_CACHE_HOME") {
            Some(PathBuf::from(cache).join("anvil"))
        } else {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache/anvil"))
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    {
        None
    }
}

/// Pure resolution so tests can supply the override and executable path
/// without mutating the process environment.
fn resolve(override_dir: Option<PathBuf>, exe: Option<&Path>) -> Result<PathBuf, PathsError> {
    if let Some(dir) = override_dir {
        create_and_probe(&dir).map_err(|_| PathsError::Unavailable)?;
        return Ok(dir);
    }
    if let Some(exe) = exe {
        if let Some(parent) = exe
            .canonicalize()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
        {
            let portable = parent.join(DATA_DIR_NAME);
            if create_and_probe(&portable).is_ok() {
                return Ok(portable);
            }
        }
    }
    if let Some(legacy) = legacy_data_dir() {
        if create_and_probe(&legacy).is_ok() {
            FALLBACK_NOTICE.call_once(|| {
                eprintln!(
                    "anvil: the program folder is not writable; using {} instead (set ANVIL_DATA_DIR to choose a location)",
                    legacy.display()
                );
            });
            return Ok(legacy);
        }
    }
    Err(PathsError::Unavailable)
}

/// Create `dir` if needed and prove it is writable with a throwaway file.
fn create_and_probe(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(".anvil-write-test");
    std::fs::write(&probe, b"")?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins_and_is_created() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("custom-data");
        let resolved = resolve(Some(target.clone()), None).expect("resolve");
        assert_eq!(resolved, target);
        assert!(target.is_dir());
    }

    #[test]
    fn portable_dir_sits_beside_the_executable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dir = dir.path().canonicalize().expect("canonicalize");
        let exe = dir.join("anvil");
        std::fs::write(&exe, b"").expect("exe");
        let resolved = resolve(None, Some(&exe)).expect("resolve");
        assert_eq!(resolved, dir.join(DATA_DIR_NAME));
        assert!(resolved.is_dir());
    }

    #[test]
    fn missing_executable_falls_back_or_errors() {
        // With no override and no executable path, resolution either finds
        // the legacy cache or reports Unavailable; it must never panic.
        match resolve(None, None) {
            Ok(dir) => assert!(!dir.as_os_str().is_empty()),
            Err(PathsError::Unavailable) => {}
        }
    }
}
