//! The only module allowed to use `unsafe`: thin wrappers around a handful
//! of libc calls (pty creation, terminal ioctls, process hardening).
#![allow(unsafe_code)]

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
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
pub fn harden_process() {
    // SAFETY: PR_SET_DUMPABLE takes plain integer arguments and touches no memory.
    let _ = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong) };
}

pub fn set_cloexec(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl on a valid fd with integer arguments.
    unsafe {
        let flags = check(libc::fcntl(fd, libc::F_GETFD))?;
        check(libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC))?;
    }
    Ok(())
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
pub fn spawn_on_pty(mut cmd: Command, cols: u16, rows: u16) -> io::Result<(File, Child)> {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let mut master: libc::c_int = -1;
    let mut slave: libc::c_int = -1;
    // SAFETY: openpty writes two fds into the out-parameters; name and termios
    // may be null, and `ws` is a valid winsize for the duration of the call.
    check(unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            &ws,
        )
    })?;
    // SAFETY: openpty succeeded, so both descriptors are open and owned only by us.
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    // SAFETY: as above.
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };

    set_cloexec(master.as_raw_fd())?;
    set_cloexec(slave.as_raw_fd())?;
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
