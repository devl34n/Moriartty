//! Terminal settings. The layout mirrors PuTTY/pterm's configuration tree
//! (Session / Terminal / Window) so a PuTTY user finds every option where
//! they expect it.

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
    /// PuTTY's default palette.
    fn default() -> Self {
        SCHEMES[0].1.clone()
    }
}

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
    (
        "PuTTY (classic)",
        Colours {
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
        },
    ),
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

// Unknown keys in saved files (e.g. from the earlier SSH-client draft) are
// ignored by serde, so old saved sessions still load.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // Session
    /// Program to run; empty means the user's login shell.
    pub shell: String,
    pub login_shell: bool,
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

    // Window > Behaviour
    pub wintitle: String,
    pub warn_on_close: bool,
    pub fullscreen_on_alt_enter: bool,
    /// Settings (gear) and menu buttons in the title bar.
    pub title_buttons: bool,

    // Window > Selection
    pub mouse_action: MouseAction,
    pub copy_on_select: bool,
    pub word_chars: String,
    pub paste_controls: bool,
    pub confirm_multiline_paste: bool,

    // Window > Colours
    pub bold_as_colour: bool,
    pub colours: Colours,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            shell: String::new(),
            login_shell: false,
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
            window_border: 1,

            wintitle: String::new(),
            warn_on_close: true,
            fullscreen_on_alt_enter: false,
            title_buttons: true,

            mouse_action: MouseAction::Compromise,
            copy_on_select: true,
            word_chars: "-,./?%&#:_=+@~".into(),
            paste_controls: false,
            confirm_multiline_paste: true,

            bold_as_colour: true,
            colours: Colours::default(),
        }
    }
}

/// Zoom levels in percent, like a web browser's.
pub const ZOOM_STEPS: &[u32] = &[30, 50, 67, 80, 90, 100, 110, 120, 133, 150, 170, 200, 240, 300, 400];

/// Next zoom level up (`step > 0`), down (`step < 0`) or back to 100 %.
pub fn next_zoom(current: u32, step: i32) -> u32 {
    match step {
        0 => 100,
        s if s > 0 => ZOOM_STEPS.iter().copied().find(|&z| z > current).unwrap_or(current),
        _ => ZOOM_STEPS.iter().rev().copied().find(|&z| z < current).unwrap_or(current),
    }
}

impl Settings {
    /// Clamp numbers into sane ranges. Applied to every file loaded from disk.
    pub fn sanitize(&mut self) {
        self.columns = self.columns.clamp(1, 1000);
        self.rows = self.rows.clamp(1, 500);
        self.scrollback = self.scrollback.min(1_000_000);
        self.window_border = self.window_border.min(64);
        self.font_zoom = self.font_zoom.clamp(ZOOM_STEPS[0], ZOOM_STEPS[ZOOM_STEPS.len() - 1]);
        self.bell_overload_n = self.bell_overload_n.clamp(1, 1000);
        self.bell_overload_t = self.bell_overload_t.clamp(1, 3600);
        self.bell_overload_s = self.bell_overload_s.clamp(1, 3600);
        self.shell = self.shell.trim().to_string();
    }

    pub fn validate(&self) -> Result<(), String> {
        validate::term_type(&self.term_type)?;
        if !self.shell.is_empty() {
            validate::program(&self.shell)?;
        }
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
            toml::from_str("host = \"example.com\"\nport = 2222\nprotocol = \"ssh\"\ncolumns = 100\n")
                .unwrap();
        assert_eq!(s.columns, 100);
        assert!(s.copy_on_select);
        assert_eq!(s.mouse_action, MouseAction::Compromise);
    }
}
