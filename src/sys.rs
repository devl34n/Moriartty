//! The only module allowed to use `unsafe`: thin wrappers around a handful
//! of libc calls (pty creation, terminal ioctls, process hardening).
#![allow(unsafe_code)]

use std::ffi::{CStr, OsStr};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

fn check(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

/// Make the process non-dumpable: no core files containing scrollback, and
/// other same-user processes cannot ptrace us to read terminal contents.
pub fn harden_process() -> io::Result<()> {
    // SAFETY: PR_SET_DUMPABLE takes plain integer arguments and touches no memory.
    check(unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong) })?;
    Ok(())
}

// From libvte, which vte4 already links; the Rust bindings don't wrap them.
extern "C" {
    fn vte_get_major_version() -> libc::c_uint;
    fn vte_get_minor_version() -> libc::c_uint;
    fn vte_get_micro_version() -> libc::c_uint;
}

/// The running libvte's version as VTE_VERSION spells it (0.78.2 -> 7802).
pub fn vte_version() -> u32 {
    // SAFETY: the three functions take no arguments and only return
    // constants compiled into libvte.
    let (major, minor, micro) =
        unsafe { (vte_get_major_version(), vte_get_minor_version(), vte_get_micro_version()) };
    major * 10_000 + minor * 100 + micro
}

pub fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl on a valid fd with integer arguments.
    unsafe {
        let flags = check(libc::fcntl(fd, libc::F_GETFL))?;
        check(libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK))?;
    }
    Ok(())
}

pub fn set_winsize(fd: RawFd, cols: u16, rows: u16) -> io::Result<()> {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCSWINSZ reads a `winsize` struct from the pointer, which is valid for the call.
    check(unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &ws as *const libc::winsize) })?;
    Ok(())
}

/// Process group currently in the foreground of the terminal.
pub fn foreground_pgrp(fd: RawFd) -> io::Result<i32> {
    // SAFETY: tcgetpgrp only reads from the given descriptor.
    let r = unsafe { libc::tcgetpgrp(fd) };
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

/// Spawn `cmd` as a session leader whose controlling terminal is a fresh pty.
/// Returns the (non-blocking, close-on-exec) master side and the child.
///
/// Both pty fds are created close-on-exec (not marked afterwards, as
/// openpty would need), so a fork+exec on another thread — GIO starts some —
/// can never inherit them.
pub fn spawn_on_pty(mut cmd: Command, cols: u16, rows: u16) -> io::Result<(File, Child)> {
    // SAFETY: posix_openpt takes only flags and returns a new fd or -1.
    let master = check(unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) })?;
    // SAFETY: posix_openpt succeeded, so the descriptor is open and owned only by us.
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    // SAFETY: grantpt and unlockpt take a pty master fd and touch no memory.
    check(unsafe { libc::grantpt(master.as_raw_fd()) })?;
    // SAFETY: as above.
    check(unsafe { libc::unlockpt(master.as_raw_fd()) })?;
    let mut name = [0u8; 128];
    // SAFETY: ptsname_r writes a NUL-terminated name of at most `name.len()`
    // bytes into the buffer, which is valid for the call.
    let r = unsafe { libc::ptsname_r(master.as_raw_fd(), name.as_mut_ptr().cast(), name.len()) };
    if r != 0 {
        return Err(io::Error::from_raw_os_error(r));
    }
    let name = CStr::from_bytes_until_nul(&name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "pty name is not terminated"))?;
    let slave: OwnedFd = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC)
        .open(OsStr::from_bytes(name.to_bytes()))?
        .into();

    set_winsize(master.as_raw_fd(), cols, rows)?;
    set_nonblocking(master.as_raw_fd())?;

    let out = slave.try_clone()?;
    let err = slave.try_clone()?;
    cmd.stdin(Stdio::from(slave))
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err));

    // SAFETY: the closure runs in the forked child before exec and only calls
    // async-signal-safe functions (setsid, ioctl).
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd.spawn()?;
    // `cmd` (holding the parent's copies of the slave fds) is dropped here.
    Ok((File::from(master), child))
}
