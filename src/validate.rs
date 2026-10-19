//! Input validation for the few values that reach the filesystem or exec.

use std::path::{Path, PathBuf};

use crate::storage;

fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

pub fn term_type(t: &str) -> Result<(), String> {
    if t.is_empty()
        || t.len() > 64
        || !t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.+".contains(c))
    {
        return Err(format!("Invalid terminal type: {t:?}"));
    }
    Ok(())
}

pub fn path(p: &str) -> Result<(), String> {
    if p.is_empty() {
        return Err("Path is empty".into());
    }
    if p.len() > 4096 || has_control(p) {
        return Err(format!("Invalid path: {p:?}"));
    }
    Ok(())
}

/// A log file name: a valid path that is absolute (or starts with `~/`), so
/// the log never lands in whatever directory Moriartty happened to start in.
pub fn log_path(p: &str) -> Result<(), String> {
    path(p)?;
    if !expand_tilde(p).is_absolute() {
        return Err(format!("{p:?} must be an absolute path (e.g. ~/term.log)"));
    }
    Ok(())
}

/// The "program to run" setting: an absolute path to an executable file.
/// It is exec'd directly (no shell), with no arguments.
pub fn program(p: &str) -> Result<(), String> {
    path(p)?;
    let expanded = expand_tilde(p);
    if !expanded.is_absolute() {
        return Err(format!("Program {p:?} must be an absolute path (e.g. /usr/bin/zsh)"));
    }
    if !is_executable(&expanded) {
        return Err(format!("{p:?} is not an executable file"));
    }
    Ok(())
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

pub fn session_name(n: &str) -> Result<(), String> {
    if n.is_empty() || n.len() > 64 {
        return Err("Session names must be 1-64 characters long".into());
    }
    if n.starts_with('.') || n.starts_with(' ') || n.ends_with(' ') {
        return Err("Session names must not start with '.' or start/end with a space".into());
    }
    if !n
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || " ._-@+()".contains(c))
    {
        return Err(format!(
            "Invalid session name {n:?} (letters, digits, space and . _ - @ + ( ) only)"
        ));
    }
    Ok(())
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if p == "~" || p.starts_with("~/") {
        if let Ok(home) = storage::home_dir() {
            return if p == "~" { home } else { home.join(&p[2..]) };
        }
    }
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs() {
        assert!(program("/bin/sh").is_ok());
        assert!(program("sh").is_err());
        assert!(program("/etc/passwd").is_err());
        assert!(program("/bin/sh\n").is_err());
    }

    #[test]
    fn log_paths() {
        assert!(log_path("/var/tmp/x.log").is_ok());
        assert!(log_path("~/moriartty-&Y&M&D-&T.log").is_ok());
        assert!(log_path("x.log").is_err());
        assert!(log_path("").is_err());
    }

    #[test]
    fn session_names() {
        assert!(session_name("Default Settings").is_ok());
        assert!(session_name("big font (dark)").is_ok());
        assert!(session_name("../x").is_err());
        assert!(session_name("a/b").is_err());
        assert!(session_name(".hidden").is_err());
    }
}
