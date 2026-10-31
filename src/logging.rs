//! Session logging. Log files are created 0600, never through a symlink,
//! and only if they belong to the current user.

use std::fs::{File, OpenOptions, Permissions};
use std::io::{self, BufWriter, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use gtk::glib;

use crate::config::LogType;
use crate::validate;

fn now(fmt: &str) -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format(fmt).ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Expand PuTTY's log-name placeholders: &Y year, &M month, &D day,
/// &T time, && literal '&'. (&H expands to "local", &P to the process id.)
pub fn expand_path(pattern: &str) -> Result<PathBuf, String> {
    validate::log_path(pattern)?;
    let mut out = String::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('H') => out.push_str("local"),
            Some('P') => out.push_str(&std::process::id().to_string()),
            Some('Y') => out.push_str(&now("%Y")),
            Some('M') => out.push_str(&now("%m")),
            Some('D') => out.push_str(&now("%d")),
            Some('T') => out.push_str(&now("%H%M%S")),
            Some('&') => out.push('&'),
            Some(o) => {
                out.push('&');
                out.push(o);
            }
            None => out.push('&'),
        }
    }
    Ok(validate::expand_tilde(&out))
}

/// O_NONBLOCK: opening a FIFO for writing would otherwise block (on the GTK
/// main thread) until something reads it; it has no effect on the regular
/// files we accept. O_NOCTTY: a tty device must never become our
/// controlling terminal. There is no O_TRUNC: the file is only emptied once
/// it has passed the checks below.
pub fn open_log(path: &Path, append: bool) -> io::Result<File> {
    let f = OpenOptions::new()
        .write(true)
        .append(append)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NOCTTY | libc::O_NONBLOCK)
        .open(path)?;
    let m = f.metadata()?;
    if !m.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    if m.uid() != nix::unistd::getuid().as_raw() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "file is owned by another user",
        ));
    }
    if m.mode() & 0o077 != 0 {
        f.set_permissions(Permissions::from_mode(0o600))?;
    }
    if !append {
        f.set_len(0)?;
    }
    Ok(f)
}

/// Strips escape sequences for "printable output" logs.
#[derive(Default)]
struct Stripper {
    state: StripState,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum StripState {
    #[default]
    Normal,
    /// After 0xC2, which starts both ordinary characters and, in UTF-8, the
    /// C1 control characters.
    C2,
    Esc,
    /// In a sequence like ESC ( B, until its final byte.
    EscIntermediate,
    Csi,
    Str,
    StrEsc,
    /// After 0xC2 in a string: the UTF-8 form of ST (0xC2 0x9C) ends it.
    StrC2,
}

impl StripState {
    /// A byte outside any sequence.
    fn plain(b: u8, out: &mut Vec<u8>) -> Self {
        match b {
            0x1b => StripState::Esc,
            0xc2 => StripState::C2,
            b'\n' | b'\r' | b'\t' => {
                out.push(b);
                StripState::Normal
            }
            0x00..=0x1f | 0x7f => StripState::Normal,
            _ => {
                out.push(b);
                StripState::Normal
            }
        }
    }

    /// The byte after an ESC.
    fn after_esc(b: u8) -> Self {
        match b {
            b'[' => StripState::Csi,
            b']' | b'P' | b'X' | b'^' | b'_' => StripState::Str,
            0x1b => StripState::Esc,
            0x20..=0x2f => StripState::EscIntermediate,
            _ => StripState::Normal,
        }
    }
}

impl Stripper {
    fn filter(&mut self, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len());
        for &b in data {
            self.state = match self.state {
                StripState::Normal => StripState::plain(b, &mut out),
                StripState::C2 => match b {
                    0x9b => StripState::Csi,
                    0x90 | 0x98 | 0x9d | 0x9e | 0x9f => StripState::Str,
                    0x80..=0x9f => StripState::Normal,
                    _ => {
                        out.push(0xc2);
                        StripState::plain(b, &mut out)
                    }
                },
                StripState::Esc => StripState::after_esc(b),
                StripState::EscIntermediate => match b {
                    0x1b => StripState::Esc,
                    0x20..=0x2f => StripState::EscIntermediate,
                    _ => StripState::Normal,
                },
                StripState::Csi => match b {
                    0x1b => StripState::Esc,
                    0x18 | 0x1a | 0x40..=0x7e => StripState::Normal,
                    _ => StripState::Csi,
                },
                StripState::Str => match b {
                    0x07 | 0x18 | 0x1a => StripState::Normal,
                    0x1b => StripState::StrEsc,
                    0xc2 => StripState::StrC2,
                    _ => StripState::Str,
                },
                // As in a terminal, an ESC ends the string and may start the
                // next sequence.
                StripState::StrEsc => {
                    if b == b'\\' {
                        StripState::Normal
                    } else {
                        StripState::after_esc(b)
                    }
                }
                StripState::StrC2 => {
                    if b == 0x9c {
                        StripState::Normal
                    } else {
                        StripState::Str
                    }
                }
            };
        }
        out
    }
}

pub struct SessionLog {
    out: BufWriter<File>,
    printable: bool,
    strip: Stripper,
    flush: bool,
}

impl SessionLog {
    pub fn new(file: File, kind: LogType, flush: bool, header: bool) -> io::Result<Self> {
        let mut out = BufWriter::new(file);
        if header {
            writeln!(
                out,
                "=~=~=~=~=~=~=~=~=~=~=~= Moriartty log {} =~=~=~=~=~=~=~=~=~=~=~=",
                now("%Y.%m.%d %H:%M:%S")
            )?;
        }
        Ok(SessionLog {
            out,
            printable: kind == LogType::Printable,
            strip: Stripper::default(),
            flush,
        })
    }

    pub fn output(&mut self, data: &[u8]) -> io::Result<()> {
        if self.printable {
            let v = self.strip.filter(data);
            self.out.write_all(&v)?;
        } else {
            self.out.write_all(data)?;
        }
        if self.flush {
            self.out.flush()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::test_support::{fresh_dir, mode};
    use std::fs;

    #[test]
    fn strips_sequences() {
        let mut s = Stripper::default();
        let out = s.filter(b"\x1b[1;31mred\x1b[0m \x1b]0;title\x07ok\x1b]2;t\x1b\\!\r\n");
        assert_eq!(out, b"red ok!\r\n");
    }

    #[test]
    fn split_sequence() {
        let mut s = Stripper::default();
        let mut out = s.filter(b"a\x1b[3");
        out.extend(s.filter(b"1mb"));
        assert_eq!(out, b"ab");
    }

    #[test]
    fn strips_sequences_with_intermediates() {
        let mut s = Stripper::default();
        assert_eq!(s.filter(b"\x1b(Bok\x1b)0!\x1b#8.\x1bc?"), b"ok!.?");
    }

    #[test]
    fn strips_c1_controls_in_utf8() {
        let mut s = Stripper::default();
        assert_eq!(
            s.filter(b"\xc2\x9b31mred\xc2\x9d0;title\x07 ok\xc2\x9d0;t\xc2\x9c!\xc2\x85."),
            b"red ok!."
        );
        // Other characters that start with the same byte stay.
        assert_eq!(s.filter("© 2026".as_bytes()), "© 2026".as_bytes());
    }

    #[test]
    fn split_escape_and_c1() {
        let mut s = Stripper::default();
        let mut out = s.filter(b"a\x1b(");
        out.extend(s.filter(b"Bb\xc2"));
        out.extend(s.filter(b"\x9b3"));
        out.extend(s.filter(b"1mc\xc2"));
        out.extend(s.filter(b"\xa9"));
        assert_eq!(out, "abc©".as_bytes());
    }

    #[test]
    fn a_new_sequence_ends_an_unfinished_one() {
        let mut s = Stripper::default();
        assert_eq!(
            s.filter(b"\x1b]0;title\x1b[31mshown\x1b[1\x1b[0m!"),
            b"shown!"
        );
    }

    #[test]
    fn new_logs_are_private() {
        let d = fresh_dir("log-new");
        let p = d.join("term.log");
        drop(open_log(&p, false).unwrap());
        assert_eq!(mode(&p), 0o600);
    }

    #[test]
    fn log_symlinks_are_refused() {
        let d = fresh_dir("log-symlink");
        let target = d.join("target");
        fs::write(&target, "keep").unwrap();
        let link = d.join("term.log");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(open_log(&link, false).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
    }

    #[test]
    fn devices_are_refused() {
        assert!(open_log(Path::new("/dev/null"), false).is_err());
    }

    #[test]
    fn fifos_do_not_block() {
        let d = fresh_dir("log-fifo");
        let p = d.join("fifo");
        // No unsafe code outside sys.rs, so the FIFO comes from mkfifo(1).
        let made = std::process::Command::new("mkfifo").arg(&p).status();
        if !made.is_ok_and(|s| s.success()) {
            return;
        }
        // Without a reader, opening for writing fails at once (ENXIO)
        // instead of hanging.
        assert!(open_log(&p, false).is_err());
    }

    #[test]
    fn emptied_only_when_not_appending() {
        let d = fresh_dir("log-append");
        let p = d.join("term.log");
        fs::write(&p, "old").unwrap();
        let mut f = open_log(&p, true).unwrap();
        f.write_all(b"+new").unwrap();
        drop(f);
        assert_eq!(fs::read_to_string(&p).unwrap(), "old+new");
        assert_eq!(mode(&p), 0o600);
        drop(open_log(&p, false).unwrap());
        assert_eq!(fs::read_to_string(&p).unwrap(), "");
    }
}
