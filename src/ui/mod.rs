pub mod config_dialog;
pub mod form;
pub mod keys;
pub mod style;
pub mod terminal;

use gtk::prelude::*;

/// Modal question; returns the index of the chosen button (or `cancel`).
pub async fn ask(
    parent: &impl IsA<gtk::Window>,
    message: &str,
    detail: &str,
    buttons: &[&str],
    cancel: i32,
    default: i32,
) -> i32 {
    let d = gtk::AlertDialog::builder()
        .modal(true)
        .message(message)
        .detail(detail)
        .build();
    d.set_buttons(buttons);
    d.set_cancel_button(cancel);
    d.set_default_button(default);
    d.choose_future(Some(parent)).await.unwrap_or(cancel)
}

pub fn message(parent: &impl IsA<gtk::Window>, message: &str, detail: &str) {
    let d = gtk::AlertDialog::builder()
        .modal(true)
        .message(message)
        .detail(detail)
        .build();
    d.show(Some(parent));
}

/// Diagnostics on stderr, only when MORIARTTY_DEBUG is set.
pub fn debug(msg: impl AsRef<str>) {
    if std::env::var_os("MORIARTTY_DEBUG").is_some() {
        eprintln!("moriartty[debug]: {}", msg.as_ref());
    }
}

/// Remove control characters (escape sequences included) and invisible or
/// direction-changing ones, and cap length. Used for anything
/// remote-controlled that ends up in window chrome.
pub fn strip_controls(s: &str, max: usize) -> String {
    s.chars()
        .filter(|&c| !c.is_control() && !crate::paste::is_invisible(c))
        .take(max)
        .collect()
}
