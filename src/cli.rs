//! pterm-style command line. Also implements the options desktops use to
//! launch a default terminal (xdg-terminal-exec, Debian's x-terminal-emulator).

use std::path::PathBuf;

use crate::config::*;
use crate::{storage, validate};

pub enum Launch {
    Help,
    Version,
    /// Open the settings window first, then a terminal as for `Open`.
    Configure(Settings, Option<Vec<String>>, Option<PathBuf>),
    /// Open a terminal; `Some(argv)` runs that instead of the shell, and
    /// `Some(dir)` starts it in that directory.
    Open(Settings, Option<Vec<String>>, Option<PathBuf>),
}

/// What the command line asks for. Settings are only loaded once all of it
/// has been read, so the order of the options doesn't matter, and -help and
/// -version leave the configuration alone.
#[derive(Default)]
struct Options {
    load: Option<String>,
    configure: bool,
    title: Option<String>,
    geometry: Option<(u16, u16)>,
    dir: Option<PathBuf>,
    hold: bool,
    login_shell: bool,
    log_file: Option<String>,
    command: Option<Vec<String>>,
}

pub const USAGE: &str = "\
Usage: moriartty [options] [-e command [args...]]

Opens a terminal running your shell. Ctrl+Right-click inside it for the menu
(New Window, Settings, Find, Event Log, ...).

  -load NAME        use a saved session's settings instead of Default Settings
  -config           open the settings window first
  -title TEXT       window title (also -T)
  -geometry CxR     columns x rows, e.g. 120x40
  -dir DIR          start in directory DIR (also --working-directory DIR)
  -hold             keep the window open after the program exits
  -ls               run the shell as a login shell
  -log FILE         log all session output to FILE
  -e CMD [ARGS...]  run CMD instead of the shell (must be last)
  -help, -version
";

fn parse_geometry(g: &str) -> Result<(u16, u16), String> {
    let (c, r) = g
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("invalid geometry {g:?} (expected e.g. 120x40)"))?;
    let c: u16 = c.parse().map_err(|_| format!("invalid geometry {g:?}"))?;
    let r: u16 = r.parse().map_err(|_| format!("invalid geometry {g:?}"))?;
    if !(1..=1000).contains(&c) || !(1..=500).contains(&r) {
        return Err(format!("geometry {g:?} out of range"));
    }
    Ok((c, r))
}

/// `-log FILE`: a relative name is taken relative to the current directory,
/// like any other command-line file argument.
fn absolute_log_file(f: &str) -> Result<String, String> {
    validate::path(f)?;
    if validate::expand_tilde(f).is_absolute() {
        return Ok(f.to_string());
    }
    let cwd = std::env::current_dir().map_err(|e| format!("-log: {e}"))?;
    let cwd = cwd
        .to_str()
        .ok_or("-log: current directory is not valid UTF-8")?;
    // '&' starts a placeholder in log names; keep the directory literal.
    Ok(format!("{}/{f}", cwd.replace('&', "&&")))
}

fn parse_dir(d: &str) -> Result<PathBuf, String> {
    validate::path(d)?;
    let p = validate::expand_tilde(d);
    if !p.is_dir() {
        return Err(format!("{d:?} is not a directory"));
    }
    Ok(p)
}

pub fn parse(args: &[String]) -> Result<Launch, String> {
    let mut o = Options::default();
    let mut it = args.iter();

    while let Some(a) = it.next() {
        let mut val = |name: &str| -> Result<String, String> {
            it.next()
                .cloned()
                .ok_or_else(|| format!("option {name} requires an argument"))
        };
        match a.as_str() {
            "-h" | "-help" | "--help" => return Ok(Launch::Help),
            "-V" | "-version" | "--version" => return Ok(Launch::Version),
            "-load" => o.load = Some(val("-load")?),
            "-config" => o.configure = true,
            "-title" | "-T" => o.title = Some(val("-title")?),
            "-geometry" => o.geometry = Some(parse_geometry(&val("-geometry")?)?),
            "-dir" | "--working-directory" => o.dir = Some(parse_dir(&val("-dir")?)?),
            x if x.starts_with("--working-directory=") => {
                o.dir = Some(parse_dir(&x["--working-directory=".len()..])?)
            }
            "-hold" => o.hold = true,
            "-ls" => o.login_shell = true,
            "-log" => o.log_file = Some(absolute_log_file(&val("-log")?)?),
            "-e" => {
                let rest: Vec<String> = it.by_ref().cloned().collect();
                if rest.is_empty() {
                    return Err("-e requires a command".into());
                }
                o.command = Some(rest);
                break;
            }
            x => return Err(format!("unknown option {x}")),
        }
    }

    let mut s = match &o.load {
        Some(name) => storage::load_session(name)?,
        None => storage::load_default(),
    };
    s.cli_options = o.title.is_some()
        || o.geometry.is_some()
        || o.hold
        || o.login_shell
        || o.log_file.is_some();
    if let Some(title) = o.title {
        s.wintitle = title;
    }
    if let Some((columns, rows)) = o.geometry {
        s.columns = columns;
        s.rows = rows;
    }
    if o.hold {
        s.close_on_exit = CloseOnExit::Never;
    }
    if o.login_shell {
        s.login_shell = true;
    }
    if let Some(file) = o.log_file {
        s.log_type = LogType::All;
        s.log_file = file;
    }

    // No validate() here: values from saved files are repaired when the
    // terminal starts (Settings::repair), and the settings window checks
    // them on Open, so a bad saved file can't lock the user out.
    s.sanitize();
    if o.configure {
        return Ok(Launch::Configure(s, o.command, o.dir));
    }
    Ok(Launch::Open(s, o.command, o.dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn opens_terminal_by_default() {
        assert!(matches!(parse(&[]).unwrap(), Launch::Open(_, None, None)));
    }

    #[test]
    fn dash_e_takes_the_rest() {
        match parse(&args(&["-geometry", "120x40", "-e", "htop", "-d", "10"])).unwrap() {
            Launch::Open(s, Some(cmd), None) => {
                assert_eq!(s.columns, 120);
                assert_eq!(cmd, args(&["htop", "-d", "10"]));
            }
            _ => panic!("expected open with command"),
        }
    }

    #[test]
    fn desktop_launcher_options() {
        match parse(&args(&[
            "-dir", "/tmp", "-hold", "-T", "build", "-e", "make",
        ]))
        .unwrap()
        {
            Launch::Open(s, Some(cmd), Some(dir)) => {
                assert_eq!(dir, PathBuf::from("/tmp"));
                assert_eq!(s.close_on_exit, CloseOnExit::Never);
                assert_eq!(s.wintitle, "build");
                assert_eq!(cmd, args(&["make"]));
            }
            _ => panic!("expected open with dir and command"),
        }
        assert!(matches!(
            parse(&args(&["--working-directory=/tmp"])).unwrap(),
            Launch::Open(_, None, Some(_))
        ));
    }

    #[test]
    fn options_count_wherever_they_stand() {
        let saved = Settings {
            wintitle: "from the session".into(),
            ..Settings::default()
        };
        storage::save_session("Order test", &saved).unwrap();
        for v in [
            &["-title", "x", "-load", "Order test"][..],
            &["-load", "Order test", "-title", "x"][..],
        ] {
            match parse(&args(v)).unwrap() {
                Launch::Open(s, None, None) => {
                    assert_eq!(s.wintitle, "x");
                    assert_eq!(s.session.as_deref(), Some("Order test"));
                    assert!(s.cli_options);
                }
                _ => panic!("expected open"),
            }
        }
        match parse(&args(&["-load", "Order test"])).unwrap() {
            Launch::Open(s, None, None) => {
                assert_eq!(s.wintitle, "from the session");
                assert!(!s.cli_options);
            }
            _ => panic!("expected open"),
        }
        storage::delete_session("Order test").unwrap();
    }

    #[test]
    fn help_loads_nothing() {
        // This session doesn't exist, which only matters if it gets loaded.
        assert!(matches!(
            parse(&args(&["-load", "No such session", "-help"])).unwrap(),
            Launch::Help
        ));
    }

    #[test]
    fn config_keeps_the_command_and_directory() {
        match parse(&args(&["-config", "-dir", "/tmp", "-e", "make"])).unwrap() {
            Launch::Configure(_, Some(cmd), Some(dir)) => {
                assert_eq!(cmd, args(&["make"]));
                assert_eq!(dir, PathBuf::from("/tmp"));
            }
            _ => panic!("expected the settings window, with command and directory"),
        }
    }

    #[test]
    fn errors() {
        assert!(parse(&args(&["-e"])).is_err());
        assert!(parse(&args(&["-geometry", "0x0"])).is_err());
        assert!(parse(&args(&["host"])).is_err());
        assert!(parse(&args(&["-dir", "/nonexistent/dir"])).is_err());
        assert!(parse(&args(&["-dir", "/etc/passwd"])).is_err());
    }
}
