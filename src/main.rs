//! Moriartty — a PuTTY-style (pterm) terminal emulator for Linux.
//!
//! All `unsafe` code is confined to `sys.rs`; everything else is safe Rust.
#![deny(unsafe_code)]

mod cli;
mod config;
mod logging;
mod matching;
mod paste;
mod pty;
mod storage;
mod sys;
mod ui;
mod validate;

use gtk::prelude::*;
use gtk::{gio, glib};

pub const APP_ID: &str = "io.github.moriartty.Moriartty";

fn main() -> glib::ExitCode {
    sys::harden_process();

    let args: Vec<String> = match std::env::args_os()
        .skip(1)
        .map(|a| a.into_string())
        .collect::<Result<_, _>>()
    {
        Ok(a) => a,
        Err(_) => {
            eprintln!("moriartty: command-line arguments must be valid UTF-8");
            return glib::ExitCode::FAILURE;
        }
    };

    let launch = match cli::parse(&args) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("moriartty: {e}");
            eprintln!("Try 'moriartty -help' for usage.");
            return glib::ExitCode::FAILURE;
        }
    };

    match launch {
        cli::Launch::Help => {
            print!("{}", cli::USAGE);
            return glib::ExitCode::SUCCESS;
        }
        cli::Launch::Version => {
            println!("moriartty {}", env!("CARGO_PKG_VERSION"));
            return glib::ExitCode::SUCCESS;
        }
        _ => {}
    }

    // NON_UNIQUE: every invocation is its own process, so command-line
    // arguments never travel over D-Bus to another instance.
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_activate(move |app| match &launch {
        cli::Launch::Configure(s) => {
            ui::config_dialog::show(app, s.clone(), ui::config_dialog::Mode::New)
        }
        cli::Launch::Open(s, cmd) => ui::terminal::open(app, s.clone(), cmd.clone(), None),
        cli::Launch::Help | cli::Launch::Version => {}
    });

    app.run_with_args(&["moriartty"])
}
