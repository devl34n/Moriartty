//! Moriartty — a fast, private-by-default terminal emulator for Linux,
//! inspired by PuTTY's pterm.
//!
//! All `unsafe` code is confined to `sys.rs`; everything else is safe Rust.
#![deny(unsafe_code)]

mod argv;
mod cli;
mod config;
mod logging;
mod marks;
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
    if let Err(e) = sys::harden_process() {
        eprintln!("moriartty: warning: cannot make the process non-dumpable: {e}");
    }

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

    // Only once. The application is reachable on the session bus even when
    // non-unique, and a later "activate" from there must not open a window or
    // run the -e command again.
    let activated = std::cell::Cell::new(false);
    app.connect_activate(move |app| {
        if activated.replace(true) {
            return;
        }
        // First line of every debug log: which build, on which libraries.
        ui::debug(format!(
            "moriartty {}, GTK {}.{}.{}, VTE {}",
            env!("CARGO_PKG_VERSION"),
            gtk::major_version(),
            gtk::minor_version(),
            gtk::micro_version(),
            sys::vte_version(),
        ));
        ui::style::init();
        match &launch {
            cli::Launch::Configure(s, cmd, dir) => {
                ui::config_dialog::show(
                    app,
                    s.clone(),
                    ui::config_dialog::Mode::New {
                        command: cmd.clone(),
                        dir: dir.clone(),
                    },
                );
            }
            cli::Launch::Open(s, cmd, dir) => {
                ui::terminal::open(app, s.clone(), cmd.clone(), dir.clone())
            }
            cli::Launch::Help | cli::Launch::Version => {}
        }
    });

    let status = app.run_with_args(&["moriartty"]);
    pty::wait_for_reapers();
    status
}
