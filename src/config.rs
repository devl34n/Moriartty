//! Terminal settings. The layout mirrors PuTTY/pterm's configuration tree
//! (Session / Terminal / Window) so a PuTTY user finds every option where
//! they expect it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::validate;

/// An enum shown in the UI as a drop-down list.
pub trait Choice: Copy + PartialEq + 'static {
    fn all() -> &'static [Self];
    fn label(self) -> &'static str;
}

macro_rules! choice_enum {
    ($name:ident default $def:ident { $($var:ident => $label:expr),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($var),+ }

        impl Default for $name {
            fn default() -> Self { $name::$def }
        }

        impl Choice for $name {
            fn all() -> &'static [Self] { &[$($name::$var),+] }
            fn label(self) -> &'static str { match self { $($name::$var => $label),+ } }
        }
    };
}

choice_enum!(CloseOnExit default Clean {
    Always => "Always",
    Never => "Never",
    Clean => "Only on clean exit",
});

choice_enum!(LogType default None {
    None => "None",
    Printable => "Printable output",
    All => "All session output",
});

choice_enum!(LogExists default Ask {
    Ask => "Ask the user every time",
    Overwrite => "Always overwrite it",
    Append => "Always append to the end of it",
});

choice_enum!(BackspaceKey default ControlQuestion {
    ControlH => "Control-H",
    ControlQuestion => "Control-? (127)",
});

choice_enum!(BellStyle default Audible {
    None => "None (bell disabled)",
    Audible => "Make default system alert sound",
    Visual => "Visual bell (flash window)",
});

choice_enum!(CursorType default Block {
    Block => "Block",
    Underline => "Underline",
    Vertical => "Vertical line",
});

choice_enum!(MouseAction default Compromise {
    Compromise => "Compromise (Right pastes, Middle pastes PRIMARY)",
    Windows => "Windows (Right brings up menu, Middle pastes PRIMARY)",
    Xterm => "xterm (Right goes to terminal, Middle pastes PRIMARY)",
});

pub type Rgb = [u8; 3];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Colours {
    pub foreground: Rgb,
    pub bold_foreground: Rgb,
    pub background: Rgb,
    pub cursor_text: Rgb,
    pub cursor: Rgb,
    /// 0–7 normal, 8–15 bright (VTE palette order).
    pub ansi: [Rgb; 16],
}

impl Default for Colours {
    /// Moriartty Dark.
    fn default() -> Self {
        SCHEMES[0].1.clone()
    }
}

/// The default look: muted, readable colours with the logo's red as the
/// accent. Used with its light twin when following the desktop's style.
pub const MORIARTTY_DARK: Colours = Colours {
    foreground: [214, 217, 224],
    bold_foreground: [255, 255, 255],
    background: [27, 29, 35],
    cursor_text: [27, 29, 35],
    cursor: [229, 72, 77],
    ansi: [
        [42, 45, 53],
        [229, 72, 77],
        [95, 179, 122],
        [224, 178, 77],
        [91, 155, 240],
        [184, 133, 232],
        [79, 191, 196],
        [197, 201, 211],
        [92, 98, 112],
        [255, 107, 111],
        [127, 211, 154],
        [242, 204, 107],
        [127, 180, 255],
        [209, 166, 255],
        [114, 219, 224],
        [242, 244, 248],
    ],
};

pub const MORIARTTY_LIGHT: Colours = Colours {
    foreground: [42, 45, 53],
    bold_foreground: [0, 0, 0],
    background: [250, 250, 250],
    cursor_text: [250, 250, 250],
    cursor: [196, 37, 43],
    ansi: [
        [42, 45, 53],
        [196, 37, 43],
        [46, 133, 64],
        [154, 107, 0],
        [31, 99, 198],
        [138, 63, 199],
        [18, 127, 135],
        [107, 114, 128],
        [75, 81, 96],
        [229, 72, 77],
        [63, 162, 87],
        [184, 134, 11],
        [59, 130, 246],
        [168, 85, 247],
        [14, 165, 176],
        [156, 163, 175],
    ],
};

/// PuTTY's default palette, the default before 0.6.
pub const PUTTY: Colours = Colours {
    foreground: [187, 187, 187],
    bold_foreground: [255, 255, 255],
    background: [0, 0, 0],
    cursor_text: [0, 0, 0],
    cursor: [0, 255, 0],
    ansi: [
        [0, 0, 0],
        [187, 0, 0],
        [0, 187, 0],
        [187, 187, 0],
        [0, 0, 187],
        [187, 0, 187],
        [0, 187, 187],
        [187, 187, 187],
        [85, 85, 85],
        [255, 85, 85],
        [85, 255, 85],
        [255, 255, 85],
        [85, 85, 255],
        [255, 85, 255],
        [85, 255, 255],
        [255, 255, 255],
    ],
};

const TANGO: [Rgb; 16] = [
    [46, 52, 54],
    [204, 0, 0],
    [78, 154, 6],
    [196, 160, 0],
    [52, 101, 164],
    [117, 80, 123],
    [6, 152, 154],
    [211, 215, 207],
    [85, 87, 83],
    [239, 41, 41],
    [138, 226, 52],
    [252, 233, 79],
    [114, 159, 207],
    [173, 127, 168],
    [52, 226, 226],
    [238, 238, 236],
];

const SOLARIZED: [Rgb; 16] = [
    [7, 54, 66],
    [220, 50, 47],
    [133, 153, 0],
    [181, 137, 0],
    [38, 139, 210],
    [211, 54, 130],
    [42, 161, 152],
    [238, 232, 213],
    [0, 43, 54],
    [203, 75, 22],
    [88, 110, 117],
    [101, 123, 131],
    [131, 148, 150],
    [108, 113, 196],
    [147, 161, 161],
    [253, 246, 227],
];

/// Colour scheme presets offered on the Colours page. The first entry is
/// the default.
pub const SCHEMES: &[(&str, Colours)] = &[
    ("Moriartty Dark", MORIARTTY_DARK),
    ("Moriartty Light", MORIARTTY_LIGHT),
    ("PuTTY (classic)", PUTTY),
    (
        "Tango Dark",
        Colours {
            foreground: [211, 215, 207],
            bold_foreground: [238, 238, 236],
            background: [46, 52, 54],
            cursor_text: [46, 52, 54],
            cursor: [211, 215, 207],
            ansi: TANGO,
        },
    ),
    (
        "Tango Light",
        Colours {
            foreground: [46, 52, 54],
            bold_foreground: [0, 0, 0],
            background: [255, 255, 255],
            cursor_text: [255, 255, 255],
            cursor: [46, 52, 54],
            ansi: TANGO,
        },
    ),
    (
        "Solarized Dark",
        Colours {
            foreground: [131, 148, 150],
            bold_foreground: [147, 161, 161],
            background: [0, 43, 54],
            cursor_text: [0, 43, 54],
            cursor: [147, 161, 161],
            ansi: SOLARIZED,
        },
    ),
    (
        "Solarized Light",
        Colours {
            foreground: [101, 123, 131],
            bold_foreground: [88, 110, 117],
            background: [253, 246, 227],
            cursor_text: [253, 246, 227],
            cursor: [88, 110, 117],
            ansi: SOLARIZED,
        },
    ),
    (
        "Dracula",
        Colours {
            foreground: [248, 248, 242],
            bold_foreground: [255, 255, 255],
            background: [40, 42, 54],
            cursor_text: [40, 42, 54],
            cursor: [248, 248, 242],
            ansi: [
                [33, 34, 44],
                [255, 85, 85],
                [80, 250, 123],
                [241, 250, 140],
                [189, 147, 249],
                [255, 121, 198],
                [139, 233, 253],
                [248, 248, 242],
                [98, 114, 164],
                [255, 110, 110],
                [105, 255, 148],
                [255, 255, 165],
                [214, 172, 255],
                [255, 146, 223],
                [164, 255, 255],
                [255, 255, 255],
            ],
        },
    ),
];

pub const ANSI_NAMES: [&str; 16] = [
    "ANSI Black",
    "ANSI Red",
    "ANSI Green",
    "ANSI Yellow",
    "ANSI Blue",
    "ANSI Magenta",
    "ANSI Cyan",
    "ANSI White",
    "ANSI Black Bold",
    "ANSI Red Bold",
    "ANSI Green Bold",
    "ANSI Yellow Bold",
    "ANSI Blue Bold",
    "ANSI Magenta Bold",
    "ANSI Cyan Bold",
    "ANSI White Bold",
];

/// Format version written into saved files. Files from before 0.5 have
/// none, which reads as 0; `Settings::migrate` brings them up to date.
pub const CONFIG_VERSION: u32 = 2;

// Unknown keys in saved files are ignored by serde, so files written by other
// versions still load.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Missing in old files, so it defaults to 0 there, not to the
    /// current version.
    #[serde(default)]
    pub config_version: u32,

    // Session
    /// Program to run; empty means the user's login shell.
    pub shell: String,
    pub login_shell: bool,
    /// Program and arguments to run instead of the shell, e.g. `["ssh",
    /// "prod"]`; empty runs the shell. Looked up on $PATH and started
    /// directly, never through a shell. Every tab and window opened with
    /// these settings runs it.
    pub command: Vec<String>,
    pub close_on_exit: CloseOnExit,

    // Session > Logging
    pub log_type: LogType,
    pub log_file: String,
    pub log_exists: LogExists,
    pub log_flush: bool,
    pub log_header: bool,

    // Terminal
    pub term_type: String,
    pub blink_text: bool,
    pub cjk_ambig_wide: bool,

    // Terminal > Keyboard
    pub backspace: BackspaceKey,

    // Terminal > Bell
    pub bell: BellStyle,
    pub bell_overload: bool,
    pub bell_overload_n: u32,
    pub bell_overload_t: u32,
    pub bell_overload_s: u32,

    // Terminal > Features
    pub no_remote_wintitle: bool,
    pub allow_hyperlinks: bool,
    pub clickable_urls: bool,
    /// With shell integration: a command that ran at least this many
    /// seconds and ends in a background tab marks the tab and shows a
    /// desktop notification. 0 = never.
    pub notify_command_secs: u32,

    // Window
    pub columns: u16,
    pub rows: u16,
    pub scrollback: u32,
    pub scrollbar: bool,
    pub scroll_on_key: bool,
    pub scroll_on_output: bool,

    // Window > Appearance
    pub cursor: CursorType,
    pub cursor_blink: bool,
    pub font: String,
    /// Font zoom in percent (Ctrl+= / Ctrl+- / Ctrl+0), 100 = normal.
    pub font_zoom: u32,
    pub hide_mouse: bool,
    pub window_border: u16,
    /// Terminal background opacity in percent, 30–100. Below 100 the
    /// desktop shows through (needs a compositor).
    pub background_opacity: u32,

    // Window > Behaviour
    pub wintitle: String,
    pub warn_on_close: bool,
    pub fullscreen_on_alt_enter: bool,
    /// Settings (gear) and menu buttons in the title bar.
    pub title_buttons: bool,

    // Window > Selection
    pub mouse_action: MouseAction,
    /// Selecting text always fills PRIMARY (middle-click paste); this also
    /// copies it to the clipboard, where clipboard managers keep it.
    /// Replaces 0.4's `copy_on_select`, which did that by default.
    pub copy_to_clipboard: bool,
    pub word_chars: String,
    pub paste_controls: bool,
    pub confirm_multiline_paste: bool,

    // Window > Colours
    pub bold_as_colour: bool,
    /// Moriartty Dark or Light, following the desktop's style; `colours`
    /// is used when this is off.
    pub auto_colours: bool,
    pub colours: Colours,

    // Terminal > Keyboard (a table, so it comes after every plain value)
    /// Shortcuts the user changed: action name (see `ui::keys::ACTIONS`) →
    /// keys such as "Ctrl+Shift+T"; an empty list turns the shortcut off.
    /// Actions not listed keep their default keys.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, Vec<String>>,

    /// The saved session these settings were loaded from or last saved
    /// to, so the settings window can highlight it. Never written to a file.
    #[serde(skip)]
    pub session: Option<String>,

    /// Command-line options changed these settings, for this window only: the
    /// settings window doesn't save them unless a session is highlighted.
    /// Never written to a file.
    #[serde(skip)]
    pub cli_options: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            config_version: CONFIG_VERSION,

            shell: String::new(),
            login_shell: false,
            command: Vec::new(),
            close_on_exit: CloseOnExit::Clean,

            log_type: LogType::None,
            log_file: "~/moriartty-&Y&M&D-&T.log".into(),
            log_exists: LogExists::Ask,
            log_flush: true,
            log_header: true,

            term_type: "xterm-256color".into(),
            blink_text: false,
            cjk_ambig_wide: false,

            backspace: BackspaceKey::ControlQuestion,

            bell: BellStyle::Audible,
            bell_overload: true,
            bell_overload_n: 5,
            bell_overload_t: 2,
            bell_overload_s: 5,

            no_remote_wintitle: false,
            allow_hyperlinks: false,
            clickable_urls: true,
            notify_command_secs: 10,

            columns: 80,
            rows: 24,
            scrollback: 10_000,
            scrollbar: true,
            scroll_on_key: true,
            scroll_on_output: false,

            cursor: CursorType::Block,
            cursor_blink: false,
            font: "Monospace 12".into(),
            font_zoom: 100,
            hide_mouse: true,
            window_border: 6,
            background_opacity: 100,

            wintitle: String::new(),
            warn_on_close: true,
            fullscreen_on_alt_enter: false,
            title_buttons: true,

            mouse_action: MouseAction::Compromise,
            copy_to_clipboard: false,
            word_chars: "-,./?%&#:_=+@~".into(),
            paste_controls: false,
            confirm_multiline_paste: true,

            bold_as_colour: true,
            auto_colours: true,
            colours: Colours::default(),

            keys: BTreeMap::new(),

            session: None,
            cli_options: false,
        }
    }
}

/// Zoom levels in percent, like a web browser's.
pub const ZOOM_STEPS: &[u32] = &[
    30, 50, 67, 80, 90, 100, 110, 120, 133, 150, 170, 200, 240, 300, 400,
];

/// Next zoom level up (`step > 0`), down (`step < 0`) or back to 100 %.
pub fn next_zoom(current: u32, step: i32) -> u32 {
    match step {
        0 => 100,
        s if s > 0 => ZOOM_STEPS
            .iter()
            .copied()
            .find(|&z| z > current)
            .unwrap_or(current),
        _ => ZOOM_STEPS
            .iter()
            .rev()
            .copied()
            .find(|&z| z < current)
            .unwrap_or(current),
    }
}

impl Settings {
    /// Bring a file saved by an older Moriartty up to the current format.
    /// Applied to every file loaded from disk, before `sanitize`.
    ///
    /// Format 1 (0.5): `copy_on_select`, which also copied selections to
    /// the clipboard, became `copy_to_clipboard`, off by default. serde
    /// ignores the old key, so old files get the new, private default.
    ///
    /// Format 2 (0.6): new look. Everyone follows the desktop's light or
    /// dark style; the colours saved so far stay in `colours`, used again
    /// when that is turned off. A border still at the old default (the
    /// user never changed it) moves to the new one.
    pub fn migrate(&mut self) {
        if self.config_version < 2 {
            self.auto_colours = true;
            if self.window_border == 1 {
                self.window_border = Settings::default().window_border;
            }
        }
        // Never lower it: a file from a newer Moriartty keeps its version, so
        // saving it here doesn't make the newer one migrate it again.
        self.config_version = self.config_version.max(CONFIG_VERSION);
    }

    /// The palette in use: Moriartty Dark or Light when following the
    /// desktop (`dark` is its current style), else the configured one.
    pub fn active_colours(&self, dark: bool) -> &Colours {
        match (self.auto_colours, dark) {
            (true, true) => &MORIARTTY_DARK,
            (true, false) => &MORIARTTY_LIGHT,
            (false, _) => &self.colours,
        }
    }

    /// Clamp numbers into sane ranges. Applied to every file loaded from disk.
    pub fn sanitize(&mut self) {
        self.columns = self.columns.clamp(1, 1000);
        self.rows = self.rows.clamp(1, 500);
        self.scrollback = self.scrollback.min(1_000_000);
        self.window_border = self.window_border.min(64);
        self.background_opacity = self.background_opacity.clamp(30, 100);
        self.font_zoom = self
            .font_zoom
            .clamp(ZOOM_STEPS[0], ZOOM_STEPS[ZOOM_STEPS.len() - 1]);
        self.bell_overload_n = self.bell_overload_n.clamp(1, 1000);
        self.bell_overload_t = self.bell_overload_t.clamp(1, 3600);
        self.bell_overload_s = self.bell_overload_s.clamp(1, 3600);
        self.notify_command_secs = self.notify_command_secs.min(86_400);
        self.shell = self.shell.trim().to_string();
    }

    /// Whether a tab can move between windows with these settings. Everything
    /// counts except what only says how a window happens to be sized or where
    /// it came from: its zoom, the size it started at, and its session's name.
    pub fn shares_tabs_with(&self, other: &Settings) -> bool {
        *self
            == Settings {
                font_zoom: self.font_zoom,
                columns: self.columns,
                rows: self.rows,
                session: self.session.clone(),
                cli_options: self.cli_options,
                ..other.clone()
            }
    }

    /// The command to run instead of the shell, if there is one, with the
    /// program's full path: `~` expanded, or found on $PATH. Left to exec, a
    /// bare name could also match a relative $PATH entry such as ".".
    pub fn session_command(&self) -> Option<Vec<String>> {
        let (prog, args) = self.command.split_first()?;
        let path = if prog.contains('/') {
            validate::expand_tilde(prog)
        } else {
            validate::find_in_path(prog).unwrap_or_else(|| prog.into())
        };
        let mut argv = vec![path.to_string_lossy().into_owned()];
        argv.extend(args.iter().cloned());
        Some(argv)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate::term_type(&self.term_type)?;
        if !self.shell.is_empty() {
            validate::program(&self.shell)?;
        }
        validate::command(&self.command)?;
        if self.log_type != LogType::None {
            validate::log_path(&self.log_file).map_err(|e| format!("Log file: {e}"))?;
        }
        Ok(())
    }

    /// Replace values that `validate` would reject with safe fallbacks, so a
    /// saved file that went bad (say, its shell was uninstalled) can never
    /// stop a terminal from starting. Returns one message per change.
    pub fn repair(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        if let Err(e) = validate::term_type(&self.term_type) {
            let fallback = Settings::default().term_type;
            notes.push(format!("{e}; using {fallback}"));
            self.term_type = fallback;
        }
        if !self.shell.is_empty() {
            if let Err(e) = validate::program(&self.shell) {
                notes.push(format!("{e}; using your login shell instead"));
                self.shell.clear();
            }
        }
        if let Err(e) = validate::command(&self.command) {
            notes.push(format!("{e}; running the shell instead"));
            self.command.clear();
        }
        if self.log_type != LogType::None {
            if let Err(e) = validate::log_path(&self.log_file) {
                notes.push(format!("Log file: {e}; logging disabled"));
                self.log_type = LogType::None;
            }
        }
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_toml() {
        let s = Settings::default();
        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn roundtrip_command_and_keys() {
        let mut s = Settings {
            command: vec!["ssh".into(), "admin@prod".into()],
            notify_command_secs: 0,
            ..Settings::default()
        };
        s.keys.insert("new-tab".into(), vec!["Ctrl+T".into()]);
        s.keys.insert("find".into(), Vec::new());
        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(s, back);
        // Untouched shortcuts are not written out at all.
        let text = toml::to_string_pretty(&Settings::default()).unwrap();
        assert!(!text.contains("keys"), "{text}");
    }

    #[test]
    fn session_command() {
        assert_eq!(Settings::default().session_command(), None);
        let s = Settings {
            command: vec!["sh".into(), "~/not-expanded".into()],
            ..Settings::default()
        };
        let argv = s.session_command().unwrap();
        assert!(
            argv[0].starts_with('/') && argv[0].ends_with("/sh"),
            "{argv:?}"
        );
        assert_eq!(argv[1], "~/not-expanded");
        let mut s = Settings {
            command: vec!["no-such-program-moriartty".into()],
            ..Settings::default()
        };
        assert!(s.validate().is_err());
        assert_eq!(s.repair().len(), 1);
        assert!(s.command.is_empty());
    }

    #[test]
    fn tabs_move_only_between_matching_settings() {
        let plain = Settings::default();
        let resized = Settings {
            font_zoom: 150,
            columns: 120,
            rows: 40,
            session: Some("big".into()),
            cli_options: true,
            ..Settings::default()
        };
        assert!(plain.shares_tabs_with(&resized));
        assert!(resized.shares_tabs_with(&plain));

        // A production session keeps its command and colours.
        let prod = Settings {
            command: vec!["ssh".into(), "prod".into()],
            ..Settings::default()
        };
        assert!(!plain.shares_tabs_with(&prod));
        let red = Settings {
            auto_colours: false,
            colours: PUTTY,
            ..Settings::default()
        };
        assert!(!red.shares_tabs_with(&plain));
    }

    #[test]
    fn zoom_steps() {
        assert_eq!(next_zoom(100, 1), 110);
        assert_eq!(next_zoom(100, -1), 90);
        assert_eq!(next_zoom(105, 1), 110);
        assert_eq!(next_zoom(105, -1), 100);
        assert_eq!(next_zoom(400, 1), 400);
        assert_eq!(next_zoom(30, -1), 30);
        assert_eq!(next_zoom(240, 0), 100);
    }

    #[test]
    fn default_colours_are_the_first_scheme() {
        assert_eq!(Colours::default(), SCHEMES[0].1);
        let mut names: Vec<_> = SCHEMES.iter().map(|(n, _)| *n).collect();
        names.dedup();
        assert_eq!(names.len(), SCHEMES.len());
    }

    #[test]
    fn repair_falls_back_instead_of_failing() {
        let mut s = Settings {
            shell: "/nonexistent/zsh".into(),
            term_type: "bad\nterm".into(),
            log_type: LogType::All,
            log_file: "relative.log".into(),
            ..Settings::default()
        };
        assert_eq!(s.repair().len(), 3);
        assert!(s.shell.is_empty());
        assert_eq!(s.term_type, "xterm-256color");
        assert_eq!(s.log_type, LogType::None);
        assert!(s.validate().is_ok());
        assert!(s.repair().is_empty());
    }

    #[test]
    fn old_files_still_load() {
        let s: Settings =
            toml::from_str("retired_key = \"x\"\nretired_number = 2222\ncolumns = 100\n").unwrap();
        assert_eq!(s.columns, 100);
        assert_eq!(s.mouse_action, MouseAction::Compromise);
    }

    #[test]
    fn old_files_are_migrated() {
        let mut s: Settings = toml::from_str("copy_on_select = true\ncolumns = 100\n").unwrap();
        assert_eq!(s.config_version, 0);
        assert!(!s.copy_to_clipboard);
        s.migrate();
        assert_eq!(s.config_version, CONFIG_VERSION);
        assert_eq!(s.columns, 100);
    }

    #[test]
    fn newer_files_keep_their_version() {
        let mut s: Settings =
            toml::from_str("config_version = 7\nsome_future_key = \"x\"\ncolumns = 90\n").unwrap();
        s.migrate();
        assert_eq!(s.config_version, 7);
        assert_eq!(s.columns, 90);
    }

    #[test]
    fn untouched_old_defaults_get_the_new_look() {
        let mut s = Settings {
            config_version: 1,
            colours: PUTTY,
            window_border: 1,
            ..Settings::default()
        };
        s.migrate();
        assert!(s.auto_colours);
        assert_eq!(s.window_border, 6);

        let dracula = SCHEMES
            .iter()
            .find(|(n, _)| *n == "Dracula")
            .unwrap()
            .1
            .clone();
        let mut s = Settings {
            config_version: 1,
            auto_colours: false,
            colours: dracula.clone(),
            window_border: 0,
            ..Settings::default()
        };
        s.migrate();
        assert!(s.auto_colours);
        assert_eq!(s.colours, dracula);
        assert_eq!(s.window_border, 0);

        // Current files are left alone.
        let mut s = Settings {
            auto_colours: false,
            colours: PUTTY,
            ..Settings::default()
        };
        s.migrate();
        assert!(!s.auto_colours);
    }

    #[test]
    fn auto_colours_follow_the_desktop() {
        let s = Settings::default();
        assert_eq!(*s.active_colours(true), MORIARTTY_DARK);
        assert_eq!(*s.active_colours(false), MORIARTTY_LIGHT);
        let s = Settings {
            auto_colours: false,
            colours: PUTTY,
            ..Settings::default()
        };
        assert_eq!(*s.active_colours(false), PUTTY);
    }
}
