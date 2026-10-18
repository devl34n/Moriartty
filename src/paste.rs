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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_escapes() {
        assert_eq!(sanitize_paste("ls\x1b[201~; rm -rf ~\n", false), "ls[201~; rm -rf ~\r");
        assert_eq!(sanitize_paste("a\r\nb\nc", false), "a\rb\rc");
        assert_eq!(sanitize_paste("x\u{9b}31m", false), "x31m");
        assert_eq!(sanitize_paste("tab\there", false), "tab\there");
    }

    #[test]
    fn never_ends_bracketed_mode() {
        let s = sanitize_paste("a\x1b[20\x1b[201~1~b", true);
        assert!(!s.contains("\x1b[201~"));
    }
}
