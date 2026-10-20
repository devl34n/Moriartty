//! Saved sessions (settings profiles): one TOML file per session in
//! `$XDG_CONFIG_HOME/moriartty/sessions/`, directory mode 0700, files 0600,
//! written atomically, symlinks refused.

use std::collections::hash_map::RandomState;
use std::fs::{self, DirBuilder, File, OpenOptions, Permissions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::config::Settings;
use crate::validate;

pub const DEFAULT_SESSION: &str = "Default Settings";
const MAX_FILE: u64 = 1 << 20;

/// Problems found while loading settings before any window exists. The
/// first terminal shows them (`take_notices`), so they aren't only on a
/// stderr nobody sees.
static NOTICES: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn notice(msg: String) {
    eprintln!("moriartty: {msg}");
    if let Ok(mut n) = NOTICES.lock() {
        n.push(msg);
    }
}

pub fn take_notices() -> Vec<String> {
    NOTICES.lock().map(|mut n| std::mem::take(&mut *n)).unwrap_or_default()
}

fn uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

pub fn home_dir() -> Result<PathBuf, String> {
    if let Some(h) = std::env::var_os("HOME") {
        let p = PathBuf::from(h);
        if p.is_absolute() {
            return Ok(p);
        }
    }
    match nix::unistd::User::from_uid(nix::unistd::getuid()) {
        Ok(Some(u)) => Ok(u.dir),
        _ => Err("Cannot determine home directory".into()),
    }
}

pub fn config_dir() -> Result<PathBuf, String> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(p) if p.is_absolute() => p,
        _ => home_dir()?.join(".config"),
    };
    Ok(base.join("moriartty"))
}

/// Create `p` (mode 0700) if needed and make sure it is a real directory,
/// owned by us, and not accessible to anyone else.
pub fn ensure_private_dir(p: &Path) -> Result<(), String> {
    let err = |e: io::Error| format!("{}: {e}", p.display());
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(err)?;
    }
    match DirBuilder::new().mode(0o700).create(p) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(err(e)),
    }
    let m = fs::symlink_metadata(p).map_err(err)?;
    if !m.is_dir() {
        return Err(format!("{} is not a directory (symlinks are refused)", p.display()));
    }
    if m.uid() != uid() {
        return Err(format!("{} is not owned by you", p.display()));
    }
    if m.mode() & 0o077 != 0 {
        fs::set_permissions(p, Permissions::from_mode(0o700)).map_err(err)?;
    }
    Ok(())
}

fn sessions_dir() -> Result<PathBuf, String> {
    let cfg = config_dir()?;
    ensure_private_dir(&cfg)?;
    let dir = cfg.join("sessions");
    ensure_private_dir(&dir)?;
    Ok(dir)
}

fn session_path(name: &str) -> Result<PathBuf, String> {
    validate::session_name(name)?;
    Ok(sessions_dir()?.join(format!("{name}.toml")))
}

/// Read a small file we own, refusing symlinks, non-regular files and
/// anything implausibly large.
pub fn read_private_file(path: &Path) -> Result<String, String> {
    let err = |e: io::Error| format!("{}: {e}", path.display());
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(err)?;
    let m = f.metadata().map_err(err)?;
    if !m.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if m.uid() != uid() {
        return Err(format!("{} is not owned by you", path.display()));
    }
    if m.len() > MAX_FILE {
        return Err(format!("{} is too large", path.display()));
    }
    if m.mode() & 0o077 != 0 {
        f.set_permissions(Permissions::from_mode(0o600)).map_err(err)?;
    }
    let mut s = String::new();
    f.take(MAX_FILE).read_to_string(&mut s).map_err(err)?;
    Ok(s)
}

/// Write via a 0600 temporary file in the same directory, fsync, rename,
/// fsync the directory.
fn write_private_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("invalid path")?;
    let fname = path
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or("invalid file name")?;
    // An unpredictable name (RandomState is seeded from the OS), so nothing
    // can be waiting at it.
    let nonce = RandomState::new().build_hasher().finish();
    let tmp = dir.join(format!(".{fname}.{nonce:016x}.tmp"));
    let result = (|| -> io::Result<()> {
        let mut f: File = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{}: {e}", path.display()));
    }
    // Make the rename itself survive a crash. Best effort: the new file is
    // already in place.
    if let Ok(d) = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(dir)
    {
        let _ = d.sync_all();
    }
    Ok(())
}

pub fn list_sessions() -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(dir) = sessions_dir() {
        if let Ok(rd) = fs::read_dir(dir) {
            for ent in rd.flatten() {
                let fname = ent.file_name();
                let Some(fname) = fname.to_str() else { continue };
                let Some(name) = fname.strip_suffix(".toml") else { continue };
                if validate::session_name(name).is_ok() && name != DEFAULT_SESSION {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort_by_key(|n| n.to_lowercase());
    names.insert(0, DEFAULT_SESSION.to_string());
    names
}

/// Why a saved session could not be loaded.
enum LoadError {
    /// The file could not be read (permissions, symlink, owner, ...).
    Read(String),
    /// It was read, but is not valid settings.
    Parse(PathBuf, String),
}

fn try_load(name: &str) -> Result<Settings, LoadError> {
    let path = session_path(name).map_err(LoadError::Read)?;
    if name == DEFAULT_SESSION && fs::symlink_metadata(&path).is_err() {
        return Ok(Settings::default());
    }
    let text = read_private_file(&path).map_err(LoadError::Read)?;
    let mut s: Settings = toml::from_str(&text)
        .map_err(|e| LoadError::Parse(path.clone(), format!("{}: {e}", path.display())))?;
    s.migrate();
    s.sanitize();
    Ok(s)
}

pub fn load_session(name: &str) -> Result<Settings, String> {
    try_load(name).map_err(|e| match e {
        LoadError::Read(m) | LoadError::Parse(_, m) => m,
    })
}

/// "Default Settings", or built-in defaults if that cannot be read.
///
/// The settings window saves to "Default Settings" by default, so a file
/// that doesn't parse is moved aside to `….toml.bad` first: it is kept for
/// the user to fix, never silently overwritten.
pub fn load_default() -> Settings {
    match try_load(DEFAULT_SESSION) {
        Ok(s) => s,
        Err(LoadError::Read(e)) => {
            notice(format!("Ignoring saved default settings: {e}"));
            Settings::default()
        }
        Err(LoadError::Parse(path, e)) => {
            let bad = path.with_extension("toml.bad");
            notice(match fs::rename(&path, &bad) {
                Ok(()) => format!(
                    "Saved default settings could not be read ({e}). They were moved to {} \
                     and built-in defaults are used.",
                    bad.display()
                ),
                Err(re) => format!(
                    "Saved default settings could not be read ({e}), nor moved aside ({re}). \
                     Built-in defaults are used."
                ),
            });
            Settings::default()
        }
    }
}

pub fn save_session(name: &str, s: &Settings) -> Result<(), String> {
    let path = session_path(name)?;
    let text = toml::to_string_pretty(s).map_err(|e| e.to_string())?;
    write_private_atomic(&path, text.as_bytes())
}

pub fn delete_session(name: &str) -> Result<(), String> {
    let path = session_path(name)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

