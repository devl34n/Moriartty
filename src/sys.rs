//! The only module allowed to use `unsafe`: thin wrappers around a handful
//! of libc calls (pty creation, terminal ioctls, process hardening) and two
//! C functions of libvte/libglib that the Rust bindings don't wrap.
#![allow(unsafe_code)]

use std::ffi::{CStr, OsStr};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use gtk::glib;
use gtk::glib::translate::IntoGlib;

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
    check(unsafe {
        libc::prctl(
            libc::PR_SET_DUMPABLE,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
        )
    })?;
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
    let (major, minor, micro) = unsafe {
        (
            vte_get_major_version(),
            vte_get_minor_version(),
            vte_get_micro_version(),
        )
    };
    major * 10_000 + minor * 100 + micro
}

/// What an fd watch calls: keep watching (`Continue`) or stop (`Break`).
type FdCallback = Box<dyn FnMut(RawFd, glib::IOCondition) -> glib::ControlFlow>;

// From libglib (glib-unix.h), which glib-sys already links. gtk-rs 0.22
// dropped its wrapper, glib::unix_fd_add_local.
extern "C" {
    fn g_unix_fd_add_full(
        priority: libc::c_int,
        fd: libc::c_int,
        condition: libc::c_uint,
        function: Option<
            unsafe extern "C" fn(libc::c_int, libc::c_uint, *mut libc::c_void) -> libc::c_int,
        >,
        user_data: *mut libc::c_void,
        notify: Option<unsafe extern "C" fn(*mut libc::c_void)>,
    ) -> libc::c_uint;
}

unsafe extern "C" fn fd_dispatch(
    fd: libc::c_int,
    condition: libc::c_uint,
    data: *mut libc::c_void,
) -> libc::c_int {
    // SAFETY: `data` is the FdCallback boxed by unix_fd_add_local. GLib keeps
    // it alive until fd_destroy, which never runs during a dispatch, and
    // never dispatches the same source recursively.
    let f = unsafe { &mut *data.cast::<FdCallback>() };
    match f(fd, glib::IOCondition::from_bits_truncate(condition)) {
        glib::ControlFlow::Continue => 1,
        glib::ControlFlow::Break => 0,
    }
}

unsafe extern "C" fn fd_destroy(data: *mut libc::c_void) {
    // SAFETY: called once by GLib when the source goes away; `data` came
    // from Box::into_raw in unix_fd_add_local.
    drop(unsafe { Box::from_raw(data.cast::<FdCallback>()) });
}

/// Call `func` from the default main loop whenever `fd` meets `condition`,
/// until it returns `Break` (what glib::unix_fd_add_local did). Main thread
/// only, like every other GTK call here. `None` if GLib refused the fd.
pub fn unix_fd_add_local<F>(
    fd: RawFd,
    condition: glib::IOCondition,
    priority: glib::Priority,
    func: F,
) -> Option<glib::SourceId>
where
    F: FnMut(RawFd, glib::IOCondition) -> glib::ControlFlow + 'static,
{
    let boxed: Box<FdCallback> = Box::new(Box::new(func));
    let data = Box::into_raw(boxed).cast::<libc::c_void>();
    // SAFETY: the two callbacks have the GUnixFDSourceFunc and GDestroyNotify
    // signatures; from here on GLib owns `data` and frees it via fd_destroy.
    let id = unsafe {
        g_unix_fd_add_full(
            priority.into_glib(),
            fd,
            condition.bits(),
            Some(fd_dispatch),
            data,
            Some(fd_destroy),
        )
    };
    if id == 0 {
        // Refused before taking ownership (invalid fd): free it ourselves.
        // SAFETY: GLib did not keep `data`, so this is its only owner.
        drop(unsafe { Box::from_raw(data.cast::<FdCallback>()) });
        return None;
    }
    // SAFETY: a non-zero id from g_unix_fd_add_full names a live source.
    Some(unsafe { glib::translate::from_glib(id) })
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

/// A descriptor that turns readable once process `pid` has ended, so nothing
/// has to poll for it. Needs Linux 5.3.
pub fn pidfd_open(pid: i32) -> io::Result<OwnedFd> {
    // SAFETY: pidfd_open takes a pid and flags and touches no memory.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0 as libc::c_uint) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success it returns a new close-on-exec descriptor owned only by us.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
}

/// What the terminal driver needs for programs that leave line editing to it
/// (`cat`, `read`, password prompts): which byte the Backspace key sends, and
/// that text is UTF-8, so it erases a whole character and not one byte of it.
fn set_line_editing(fd: RawFd, erase: u8) -> io::Result<()> {
    // SAFETY: termios is plain integers, so all zeroes is a valid value.
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: tcgetattr fills the struct, which is valid for the call.
    check(unsafe { libc::tcgetattr(fd, &mut t) })?;
    t.c_iflag |= libc::IUTF8;
    t.c_cc[libc::VERASE] = erase;
    // SAFETY: tcsetattr reads the struct, which is valid for the call.
    check(unsafe { libc::tcsetattr(fd, libc::TCSANOW, &t) })?;
    Ok(())
}

/// Linux's flag for close_range: mark the descriptors, don't close them.
const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;

/// Spawn `cmd` as a session leader whose controlling terminal is a fresh pty.
/// Returns the (non-blocking, close-on-exec) master side and the child.
/// `erase` is the byte the Backspace key sends.
///
/// Both pty fds are created close-on-exec (not marked afterwards, as
/// openpty would need), so a fork+exec on another thread — GIO starts some —
/// can never inherit them.
pub fn spawn_on_pty(
    mut cmd: Command,
    cols: u16,
    rows: u16,
    erase: u8,
) -> io::Result<(File, Child)> {
    // SAFETY: posix_openpt takes only flags and returns a new fd or -1.
    let master =
        check(unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) })?;
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
    set_line_editing(slave.as_raw_fd(), erase)?;

    let out = slave.try_clone()?;
    let err = slave.try_clone()?;
    cmd.stdin(Stdio::from(slave))
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err));

    // SAFETY: the closure runs in the forked child before exec and only calls
    // async-signal-safe functions (setsid, ioctl, the close_range system call).
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            // Anything the launcher left open must not reach the program.
            // Marked, not closed: std's pipe for reporting a failed exec is
            // among them. Best effort, as kernels before 5.11 can't do this.
            libc::syscall(
                libc::SYS_close_range,
                3 as libc::c_uint,
                libc::c_uint::MAX,
                CLOSE_RANGE_CLOEXEC,
            );
            Ok(())
        });
    }

    let child = cmd.spawn()?;
    // `cmd` (holding the parent's copies of the slave fds) is dropped here.
    Ok((File::from(master), child))
}
