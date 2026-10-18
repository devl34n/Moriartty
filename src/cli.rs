//! pterm-style command line.

use crate::config::*;
use crate::storage;

pub enum Launch {
    Help,
    Version,
    /// Open the settings window first.
    Configure(Settings),
    /// Open a terminal; `Some(argv)` runs that instead of the shell.
    Open(Settings, Option<Vec<String>>),
}

pub const USAGE: &str = "\
Usage: moriartty [options] [-e command [args...]]

Opens a terminal running your shell. Ctrl+Right-click inside it for the menu
(New Window, Change Settings, Event Log, ...).

  -load NAME        use a saved session's settings instead of Default Settings
  -config           open the settings window first
  -title TEXT       window title
  -geometry CxR     columns x rows, e.g. 120x40
  -ls               run the shell as a login shell
  -log FILE         log all session output to FILE
  -e CMD [ARGS...]  run CMD instead of the shell (must be last)
  -help, -version
";

fn parse_geometry(s: &mut Settings, g: &str) -> Result<(), String> {
    let (c, r) = g
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("invalid geometry {g:?} (expected e.g. 120x40)"))?;
    let c: u16 = c.parse().map_err(|_| format!("invalid geometry {g:?}"))?;
    let r: u16 = r.parse().map_err(|_| format!("invalid geometry {g:?}"))?;
    if !(1..=1000).contains(&c) || !(1..=500).contains(&r) {
        return Err(format!("geometry {g:?} out of range"));
    }
    s.columns = c;
    s.rows = r;
    Ok(())
}

pub fn parse(args: &[String]) -> Result<Launch, String> {
    let mut s = storage::load_default();
    let mut configure = false;
    let mut command = None;
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
            "-load" => s = storage::load_session(&val("-load")?)?,
            "-config" => configure = true,
            "-title" | "-T" => s.wintitle = val("-title")?,
            "-geometry" => parse_geometry(&mut s, &val("-geometry")?)?,
            "-ls" => s.login_shell = true,
            "-log" => {
                s.log_type = LogType::All;
                s.log_file = val("-log")?;
            }
            "-e" => {
                let rest: Vec<String> = it.by_ref().cloned().collect();
                if rest.is_empty() {
                    return Err("-e requires a command".into());
                }
                command = Some(rest);
                break;
            }
            x => return Err(format!("unknown option {x}")),
        }
    }

    s.sanitize();
    s.validate()?;
    if configure {
        return Ok(Launch::Configure(s));
    }
    Ok(Launch::Open(s, command))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn opens_terminal_by_default() {
        assert!(matches!(parse(&[]).unwrap(), Launch::Open(_, None)));
    }

    #[test]
    fn dash_e_takes_the_rest() {
        match parse(&args(&["-geometry", "120x40", "-e", "htop", "-d", "10"])).unwrap() {
            Launch::Open(s, Some(cmd)) => {
                assert_eq!(s.columns, 120);
                assert_eq!(cmd, args(&["htop", "-d", "10"]));
            }
            _ => panic!("expected open with command"),
        }
    }

    #[test]
    fn errors() {
        assert!(parse(&args(&["-e"])).is_err());
        assert!(parse(&args(&["-geometry", "0x0"])).is_err());
        assert!(parse(&args(&["host"])).is_err());
    }
}
