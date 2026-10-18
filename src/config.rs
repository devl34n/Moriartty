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
        }
    }
}

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
    pub hide_mouse: bool,
    pub window_border: u16,

    // Window > Behaviour
    pub wintitle: String,
    pub warn_on_close: bool,
    pub fullscreen_on_alt_enter: bool,

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

            columns: 80,
            rows: 24,
            scrollback: 10_000,
            scrollbar: true,
            scroll_on_key: true,
            scroll_on_output: false,

            cursor: CursorType::Block,
            cursor_blink: false,
            font: "Monospace 12".into(),
            hide_mouse: true,
            window_border: 1,

            wintitle: String::new(),
            warn_on_close: true,
            fullscreen_on_alt_enter: false,

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

impl Settings {
    /// Clamp numbers into sane ranges. Applied to every file loaded from disk.
    pub fn sanitize(&mut self) {
        self.columns = self.columns.clamp(1, 1000);
        self.rows = self.rows.clamp(1, 500);
        self.scrollback = self.scrollback.min(1_000_000);
        self.window_border = self.window_border.min(64);
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
            validate::path(&self.log_file).map_err(|e| format!("Log file: {e}"))?;
        }
        Ok(())
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
    fn old_files_still_load() {
        let s: Settings =
            toml::from_str("host = \"example.com\"\nport = 2222\nprotocol = \"ssh\"\ncolumns = 100\n")
                .unwrap();
        assert_eq!(s.columns, 100);
        assert!(s.copy_on_select);
        assert_eq!(s.mouse_action, MouseAction::Compromise);
    }
}
