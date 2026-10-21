//! The "evil bytes" corpus: output a hostile program could print, fed into
//! the same VTE widget Moriartty uses.
//!
//! Some of Moriartty's protections exist because VTE does not implement a
//! feature: clipboard writes and reads (OSC 52), title reports, answerback.
//! This locks that in. Anything VTE answers reaches the shell as if it had
//! been typed, so no answer may contain text the attacker chose, and the
//! clipboard and PRIMARY selection must never change.
//!
//! Needs a display (CI runs it under xvfb-run). Without one it is skipped,
//! as in the RPM and .deb builds, unless MORIARTTY_REQUIRE_DISPLAY is set.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib};
use vte::prelude::*;

const MARK: &str = "EVILMARK";
/// "EVILMARK" in base64, as an OSC 52 write carries it.
const MARK_B64: &str = "RVZJTE1BUks=";
const SENTINEL: &str = "clipboard-sentinel";
/// SENTINEL in base64, which an answered OSC 52 read would send back.
const SENTINEL_B64: &str = "Y2xpcGJvYXJkLXNlbnRpbmVs";

const CORPUS: &[(&str, &[u8])] = &[
    ("OSC 52 clipboard write", b"\x1b]52;c;RVZJTE1BUks=\x07"),
    ("OSC 52 primary write", b"\x1b]52;p;RVZJTE1BUks=\x1b\\"),
    ("OSC 52 clipboard read", b"\x1b]52;c;?\x07"),
    ("OSC 52 primary read", b"\x1b]52;p;?\x1b\\"),
    ("window title report", b"\x1b]2;EVILMARK\x07\x1b[21t"),
    ("icon title report", b"\x1b]1;EVILMARK\x07\x1b[20t"),
    ("DECRQSS with a bogus request", b"\x1bP$qEVILMARK\x1b\\"),
    ("DECRQSS with line breaks", b"\x1bP$q\rEVILMARK\r\x1b\\"),
    (
        "XTGETTCAP of an unknown name",
        b"\x1bP+q45564C4D41524B\x1b\\",
    ),
    ("font query", b"\x1b]50;?\x07"),
    (
        "bracketed-paste markers in output",
        b"\x1b[?2004h\x1b[200~EVILMARK\x1b[201~",
    ),
    ("answerback (ENQ)", b"\x05"),
];

/// Run the main loop for `d`, so VTE processes what it was fed.
fn pump(d: Duration) {
    let ctx = glib::MainContext::default();
    let end = Instant::now() + d;
    while Instant::now() < end {
        while ctx.iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The text in `clip`, or None if it can't be read within two seconds.
fn read_text(clip: &gdk::Clipboard) -> Option<String> {
    let slot: Rc<RefCell<Option<Option<String>>>> = Rc::default();
    let s = slot.clone();
    let fut = clip.read_text_future();
    // Kept in a variable until the end: the task runs on the main loop that
    // pump() turns, and its result arrives through `slot`.
    let _task = glib::MainContext::default().spawn_local(async move {
        let text = fut.await.ok().flatten().map(|t| t.to_string());
        *s.borrow_mut() = Some(text);
    });
    let end = Instant::now() + Duration::from_secs(2);
    while slot.borrow().is_none() && Instant::now() < end {
        pump(Duration::from_millis(20));
    }
    // A local, so the RefMut is dropped before `slot` (edition 2021 keeps
    // temporaries in a block's last expression alive past its locals).
    let text = slot.borrow_mut().take().flatten();
    text
}

fn main() {
    if gtk::init().is_err() {
        if std::env::var_os("MORIARTTY_REQUIRE_DISPLAY").is_some() {
            eprintln!("evil_bytes: no display, but MORIARTTY_REQUIRE_DISPLAY is set");
            std::process::exit(1);
        }
        eprintln!("evil_bytes: skipped (no display)");
        return;
    }

    let term = vte::Terminal::new();
    let window = gtk::Window::new();
    window.set_child(Some(&term));
    window.present();

    // Everything VTE would send to the shell: keys, pastes, and answers.
    let replies: Rc<RefCell<Vec<u8>>> = Rc::default();
    {
        let r = replies.clone();
        term.connect_commit(move |_, text, _| r.borrow_mut().extend_from_slice(text.as_bytes()));
    }
    pump(Duration::from_millis(300));
    term.clipboard().set_text(SENTINEL);
    term.primary_clipboard().set_text(SENTINEL);
    pump(Duration::from_millis(100));

    let mut failures = Vec::new();
    for (name, bytes) in CORPUS {
        replies.borrow_mut().clear();
        term.feed(bytes);
        pump(Duration::from_millis(200));
        let reply = String::from_utf8_lossy(&replies.borrow()).into_owned();
        let leaked = [MARK, MARK_B64, SENTINEL, SENTINEL_B64]
            .iter()
            .any(|m| reply.contains(m));
        if leaked {
            failures.push(format!("{name}: answer contains chosen text: {reply:?}"));
        } else if name.starts_with("answerback") && !reply.is_empty() {
            failures.push(format!("{name}: answered {reply:?}"));
        } else {
            println!("ok   {name} (answer: {reply:?})");
        }
    }

    for (which, clip) in [
        ("clipboard", term.clipboard()),
        ("PRIMARY", term.primary_clipboard()),
    ] {
        match read_text(&clip) {
            Some(t) if t == SENTINEL => println!("ok   {which} unchanged"),
            other => failures.push(format!("{which} changed or unreadable: {other:?}")),
        }
    }

    window.destroy();
    if failures.is_empty() {
        println!("evil_bytes: all {} checks passed", CORPUS.len() + 2);
    } else {
        for f in &failures {
            eprintln!("FAIL {f}");
        }
        std::process::exit(1);
    }
}
