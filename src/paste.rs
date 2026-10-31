//! Paste sanitising.

/// Clean clipboard text before it is pasted:
///  * line endings become CR (what the Enter key sends);
///  * unless the user allowed it, C0/C1 control characters (ESC included)
///    are removed, so a web page cannot smuggle escape sequences or
///    "invisible" commands into the shell;
///  * the bracketed-paste terminator is always removed, so pasted text can
///    never break out of bracketed-paste mode.
pub fn sanitize_paste(text: &str, allow_controls: bool) -> String {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    let mut out: String = normalized
        .chars()
        .filter(|&c| match c {
            '\r' | '\t' => true,
            c if (c as u32) < 0x20 || c == '\x7f' => allow_controls,
            c if ('\u{80}'..='\u{9f}').contains(&c) => allow_controls,
            _ => true,
        })
        .collect();
    while out.contains("\x1b[201~") {
        out = out.replace("\x1b[201~", "");
    }
    out
}

/// Characters that are invisible or change the direction text is shown in,
/// so they can make text look different from what it is (zero-width spaces,
/// bidi overrides and isolates, tag characters, ...). `char::is_control`
/// misses these. Emoji variation selectors (U+FE00-FE0F) are left out: they
/// only choose how the character before them is drawn, and are common.
pub fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E007F}'
            | '\u{E0100}'..='\u{E01EF}'
    )
}

/// Text for the paste confirmation dialog: line breaks shown as new lines,
/// control and invisible characters spelled out as <U+XXXX> so they can't
/// disguise the paste, cut to `max` characters with a note of what is left.
pub fn paste_preview(clean: &str, max: usize) -> String {
    let mut out = String::new();
    for c in clean.chars().take(max) {
        match c {
            '\r' => out.push('\n'),
            '\t' => out.push(c),
            c if c.is_control() || is_invisible(c) => {
                out.push_str(&format!("<U+{:04X}>", c as u32))
            }
            c => out.push(c),
        }
    }
    let total = clean.chars().count();
    if total > max {
        out.push_str(&format!(
            "\n… and {} more characters not shown",
            total - max
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_escapes() {
        assert_eq!(
            sanitize_paste("ls\x1b[201~; rm -rf ~\n", false),
            "ls[201~; rm -rf ~\r"
        );
        assert_eq!(sanitize_paste("a\r\nb\nc", false), "a\rb\rc");
        assert_eq!(sanitize_paste("x\u{9b}31m", false), "x31m");
        assert_eq!(sanitize_paste("tab\there", false), "tab\there");
    }

    #[test]
    fn invisible_characters() {
        for c in [
            '\u{AD}',
            '\u{34F}',
            '\u{180E}',
            '\u{200B}',
            '\u{2028}',
            '\u{2029}',
            '\u{202E}',
            '\u{2066}',
            '\u{3164}',
            '\u{FEFF}',
            '\u{FFA0}',
            '\u{FFF9}',
            '\u{E0041}',
            '\u{E007F}',
        ] {
            assert!(is_invisible(c), "U+{:04X}", c as u32);
        }
        for c in ['a', 'é', 'я', '日', '⚠', '\u{FE0F}', '😀'] {
            assert!(!is_invisible(c), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn preview_shows_hidden_characters() {
        assert_eq!(paste_preview("ls\rpwd", 100), "ls\npwd");
        assert_eq!(
            paste_preview("a\u{202E}b\u{200B}", 100),
            "a<U+202E>b<U+200B>"
        );
        assert_eq!(paste_preview("x\x1by", 100), "x<U+001B>y");
        assert_eq!(
            paste_preview("abcdef", 4),
            "abcd\n… and 2 more characters not shown"
        );
    }

    #[test]
    fn never_ends_bracketed_mode() {
        let s = sanitize_paste("a\x1b[20\x1b[201~1~b", true);
        assert!(!s.contains("\x1b[201~"));
    }
}
