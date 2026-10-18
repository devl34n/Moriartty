//! The shell (or `-e` command) running on a pseudo-terminal.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

use crate::config::Settings;
use crate::{storage, sys, validate};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadStatus {
    More,
    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitState {
    Running,
    Exited(i32),
    Signaled(i32),
    Gone,
}

pub struct Pty {
    master: File,
    child: Option<Child>,
    /// True for the shell, false for an `-e` program.
    is_shell: bool,
}

fn login_shell() -> PathBuf {
    if let Some(sh) = std::env::var_os("SHELL").map(PathBuf::from) {
        if sh.is_absolute() && sh.exists() {
            return sh;
        }
    }
    if let Ok(Some(u)) = nix::unistd::User::from_uid(nix::unistd::getuid()) {
        if u.shell.is_absolute() && u.shell.exists() {
            return u.shell;
        }
    }
    PathBuf::from("/bin/sh")
}

/// Start the shell from settings, or `command` (argv, exec'd directly).
/// Returns the pty plus a description for the event log.
pub fn spawn(
    s: &Settings,
    command: Option<&[String]>,
    cwd: Option<&Path>,
    cols: u16,
    rows: u16,
) -> Result<(Pty, String), String> {
    let is_shell = !matches!(command, Some([_, ..]));
    let (mut cmd, desc) = match command {
        Some([prog, args @ ..]) => {
            let mut c = Command::new(prog);
            c.args(args);
            (c, format!("Running {prog:?} with {} argument(s)", args.len()))
        }
        _ => {
            let shell = if s.shell.is_empty() {
                login_shell()
            } else {
                validate::program(&s.shell)?;
                validate::expand_tilde(&s.shell)
            };
            let mut c = Command::new(&shell);
            if s.login_shell {
                let name = shell.file_name().and_then(|n| n.to_str()).unwrap_or("sh");
                c.arg0(format!("-{name}"));
            }
            (c, format!("Started {}{}", shell.display(), if s.login_shell { " (login shell)" } else { "" }))
        }
    };

    cmd.env("TERM", &s.term_type)
        .env("COLORTERM", "truecolor")
        .env_remove("COLUMNS")
        .env_remove("LINES");
    match cwd {
        Some(d) if d.is_dir() => {
            cmd.current_dir(d);
        }
        _ => {
            // Launched from a desktop menu the cwd is usually "/"; use $HOME then.
            if std::env::current_dir().map(|d| d == Path::new("/")).unwrap_or(true) {
                if let Ok(home) = storage::home_dir() {
                    cmd.current_dir(home);
                }
            }
        }
    }

    let (master, child) = sys::spawn_on_pty(cmd, cols.max(1), rows.max(1))
        .map_err(|e| format!("Cannot start program: {e}"))?;
    Ok((Pty { master, child: Some(child), is_shell }, desc))
}

impl Pty {
    pub fn raw_fd(&self) -> RawFd {
        self.master.as_raw_fd()
    }

    /// Read one chunk. EIO (what a pty master returns once the child side
    /// is closed) counts as end-of-file.
    pub fn read(&mut self, out: &mut Vec<u8>) -> io::Result<ReadStatus> {
        let mut buf = [0u8; 65536];
        loop {
            match self.master.read(&mut buf) {
                Ok(0) => return Ok(ReadStatus::Eof),
                Ok(n) => {
                    out.extend_from_slice(&buf[..n]);
                    return Ok(ReadStatus::More);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(ReadStatus::More),
                Err(e) if e.raw_os_error() == Some(libc::EIO) => return Ok(ReadStatus::Eof),
                Err(e) => return Err(e),
            }
        }
    }

    pub fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.master.write(buf)
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let _ = sys::set_winsize(self.master.as_raw_fd(), cols.max(1), rows.max(1));
    }

    fn pid(&self) -> Option<i32> {
        self.child.as_ref().and_then(|c| i32::try_from(c.id()).ok())
    }

    /// True when a program is running that closing would kill — used for
    /// "warn before closing". For the shell that means something other than
    /// the shell owns the terminal (vim, a build, ssh); an `-e` program is
    /// itself the thing the user is running, so it always counts while alive.
    pub fn has_foreground_job(&self) -> bool {
        if !self.is_shell {
            return self.child.is_some();
        }
        match (self.pid(), sys::foreground_pgrp(self.master.as_raw_fd())) {
            (Some(pid), Ok(pgrp)) => pgrp != pid,
            _ => true,
        }
    }

    /// The shell's current directory, for "New Window" in the same place.
    pub fn cwd(&self) -> Option<PathBuf> {
        let pid = self.pid()?;
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }

    pub fn exit_state(&mut self) -> ExitState {
        let Some(child) = self.child.as_mut() else {
            return ExitState::Gone;
        };
        match child.try_wait() {
            Ok(Some(st)) => match st.code() {
                Some(c) => ExitState::Exited(c),
                None => ExitState::Signaled(st.signal().unwrap_or(0)),
            },
            Ok(None) => ExitState::Running,
            Err(_) => ExitState::Gone,
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if let Ok(None) = child.try_wait() {
                if let Ok(pid) = i32::try_from(child.id()) {
                    let _ = kill(Pid::from_raw(pid), Signal::SIGHUP);
                }
            }
            // Reap in the background so no zombie is left behind.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}
