//! Portable data-root resolution.
//!
//! Unko stores its configuration, ROM library, saves, states, cheats, movies,
//! screenshots and recordings under one root. Keeping every path relative to
//! that root lets the whole directory move between machines unchanged.
//!
//! Resolution order:
//! 1. `--data-dir <path>`;
//! 2. `UNKO_HOME` (or `UNKO_DATA_DIR`);
//! 3. `--portable`, which selects the executable's folder;
//! 4. an `unko.conf` beside the executable (a previously created portable
//!    install);
//! 5. an `unko.conf` in the current directory (the legacy layout);
//! 6. the executable's folder when it is writable;
//! 7. the platform data directory (`ProjectDirs`) for read-only installs.
//!
//! Mobile always uses the platform data directory; the portable override is
//! available through `UNKO_HOME` so a user can point at an exported folder.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Initializes the process-wide data root. Later calls are ignored.
pub(crate) fn init(explicit: Option<PathBuf>, portable: bool) -> PathBuf {
    let root = choose(explicit, portable);
    if let Err(err) = fs::create_dir_all(&root) {
        eprintln!("unko: cannot create data folder {}: {err}", root.display());
    }
    migrate_legacy_config(&root);
    ROOT.get_or_init(|| root).clone()
}

/// The process-wide data root.
pub(crate) fn data_root() -> PathBuf {
    ROOT.get_or_init(|| choose(None, false)).clone()
}

/// Portable configuration file.
pub(crate) fn config_path() -> PathBuf {
    data_root().join("unko.conf")
}

/// Default ROM library folder.
pub(crate) fn roms_dir() -> PathBuf {
    data_root().join("roms")
}

/// Default save/state folder.
pub(crate) fn saves_dir() -> PathBuf {
    data_root().join("saves")
}

/// Default location for the user-supplied FDS BIOS.
pub(crate) fn fds_bios_path() -> PathBuf {
    data_root().join("disksys.rom")
}

/// Resolves a user-supplied path against the data root.
pub(crate) fn resolve(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        data_root().join(path)
    }
}

/// Formats a path for `unko.conf`: relative when it lives under `base`.
pub(crate) fn store_relative(path: &Path, base: &Path) -> String {
    match path.strip_prefix(base) {
        Ok(relative) if !relative.as_os_str().is_empty() => relative.to_string_lossy().into_owned(),
        _ => path.display().to_string(),
    }
}

/// Resolves a path from a configuration file against that file's directory.
pub(crate) fn resolve_from(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// True when the active root is not the platform data directory.
pub(crate) fn is_portable() -> bool {
    platform_root()
        .map(|platform| platform != data_root())
        .unwrap_or(true)
}

fn choose(explicit: Option<PathBuf>, portable: bool) -> PathBuf {
    if let Some(path) = explicit {
        return absolute(path);
    }
    if let Some(path) = env_path("UNKO_HOME").or_else(|| env_path("UNKO_DATA_DIR")) {
        return path;
    }
    if cfg!(any(target_os = "ios", target_os = "android")) {
        return platform_root().unwrap_or_else(current_dir);
    }
    if portable {
        return executable_root()
            .filter(|dir| writable(dir))
            .unwrap_or_else(current_dir);
    }
    if let Some(dir) = executable_root() {
        if dir.join("unko.conf").is_file() {
            return dir;
        }
    }
    let cwd = current_dir();
    if cwd.join("unko.conf").is_file() {
        return cwd;
    }
    if let Some(dir) = executable_root() {
        if writable(&dir) {
            return dir;
        }
    }
    platform_root().unwrap_or(cwd)
}

fn absolute(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        current_dir().join(path)
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    let value = env::var_os(name)?;
    if value.is_empty() {
        return None;
    }
    Some(absolute(PathBuf::from(value)))
}

fn current_dir() -> PathBuf {
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Folder that contains the executable (or the AppImage file).
fn executable_root() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    // The AppImage runtime exports APPIMAGE to every child process, so only
    // trust it when this executable actually lives in the mounted AppDir.
    if let (Some(appimage), Some(appdir)) = (env::var_os("APPIMAGE"), env::var_os("APPDIR")) {
        if exe.starts_with(Path::new(&appdir)) {
            let path = absolute(PathBuf::from(appimage));
            if let Some(parent) = path.parent() {
                return Some(parent.to_path_buf());
            }
        }
    }
    let dir = exe.parent()?.to_path_buf();
    // A macOS .app keeps the binary at Foo.app/Contents/MacOS/Foo. Data
    // belongs beside the bundle, not inside its signature.
    #[cfg(target_os = "macos")]
    if dir.file_name().is_some_and(|name| name == "MacOS")
        && dir
            .parent()
            .and_then(|path| path.file_name())
            .is_some_and(|name| name == "Contents")
    {
        if let Some(bundle) = dir.parent().and_then(|contents| contents.parent()) {
            if let Some(parent) = bundle.parent() {
                return Some(parent.to_path_buf());
            }
        }
    }
    Some(dir)
}

fn platform_root() -> Option<PathBuf> {
    ProjectDirs::from("", "", "Unko").map(|dirs| dirs.data_dir().to_path_buf())
}

/// Mobile builds before the portable root kept `unko.conf` in the platform
/// config directory. Copy it into the single data root on first launch.
fn migrate_legacy_config(root: &Path) {
    let target = root.join("unko.conf");
    if target.exists() {
        return;
    }
    if let Some(dirs) = ProjectDirs::from("", "", "Unko") {
        let legacy = dirs.config_dir().join("unko.conf");
        if legacy != target && legacy.is_file() {
            let _ = fs::copy(legacy, target);
        }
    }
}

fn writable(dir: &Path) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let probe = dir.join(format!(".unko-write-{}-{stamp}", std::process::id()));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_resolve_against_the_configuration_folder() {
        let base = Path::new("/portable/unko");
        assert_eq!(
            resolve_from(base, Path::new("saves")),
            PathBuf::from("/portable/unko/saves")
        );
        assert_eq!(
            resolve_from(base, Path::new("/absolute/saves")),
            PathBuf::from("/absolute/saves")
        );
    }

    #[test]
    fn explicit_data_dir_is_used_verbatim() {
        let path = PathBuf::from("/tmp/unko-portable-test");
        assert_eq!(choose(Some(path.clone()), false), path);
    }

    #[test]
    fn paths_under_the_root_are_stored_relative() {
        let base = Path::new("/portable/unko");
        assert_eq!(
            store_relative(Path::new("/portable/unko/saves"), base),
            "saves"
        );
        assert_eq!(
            store_relative(Path::new("/elsewhere/saves"), base),
            "/elsewhere/saves"
        );
    }
}
