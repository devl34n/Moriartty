//! Text matching helpers: clickable-URL detection and find-in-scrollback.
//! VTE's regexes are PCRE2; the flag values below are PCRE2's compile options.

pub const PCRE2_CASELESS: u32 = 0x0000_0008;
pub const PCRE2_MULTILINE: u32 = 0x0000_0400;
pub const PCRE2_UTF: u32 = 0x0008_0000;

/// http(s) URLs. Trailing punctuation that usually ends a sentence or a
/// bracketed aside is not considered part of the URL.
pub const URL_REGEX: &str =
    r#"\bhttps?://[^\s<>"'`{}|\\^]*[^\s<>"'`{}|\\^.,;:!?)\]]"#;

/// Only plain web links may be opened: http(s), printable, bounded length.
/// Anything else (file:, javascript:, custom schemes, ...) is refused.
pub fn safe_url(raw: &str) -> Option<String> {
    let url = raw.trim();
    let lower = url.to_ascii_lowercase();
    let scheme_ok = lower.starts_with("http://") || lower.starts_with("https://");
    let host_part = url.split_once("://").map(|(_, rest)| rest).unwrap_or("");
    if !scheme_ok
        || host_part.is_empty()
        || url.len() > 2048
        || url.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return None;
    }
    Some(url.to_string())
}

/// Escape text so it is matched literally by a PCRE2 regex.
pub fn regex_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        if "\\^$.|?*+()[]{}/-#".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(safe_url("https://example.com/a?b=1").as_deref(), Some("https://example.com/a?b=1"));
        assert!(safe_url("HTTP://EXAMPLE.COM").is_some());
        assert!(safe_url("file:///etc/passwd").is_none());
        assert!(safe_url("javascript:alert(1)").is_none());
        assert!(safe_url("https://").is_none());
        assert!(safe_url("https://exa mple.com").is_none());
        assert!(safe_url("https://example.com/\x1b[31m").is_none());
        assert!(safe_url(&format!("https://e.com/{}", "a".repeat(3000))).is_none());
    }

    #[test]
    fn escaping() {
        assert_eq!(regex_escape("a.b*c"), r"a\.b\*c");
        assert_eq!(regex_escape("(x)[y]{z}"), r"\(x\)\[y\]\{z\}");
        assert_eq!(regex_escape(r"c:\path"), r"c:\\path");
        assert_eq!(regex_escape("plain"), "plain");
    }
}
