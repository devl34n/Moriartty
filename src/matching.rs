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
/// Anything else (file:, javascript:, custom schemes, ...) is refused, and
/// so are links with a user name before the host, a classic way to make
/// `https://bank.example@evil.example/` look like it goes to the bank.
pub fn safe_url(raw: &str) -> Option<String> {
    let url = raw.trim();
    let lower = url.to_ascii_lowercase();
    let scheme_ok = lower.starts_with("http://") || lower.starts_with("https://");
    let host_part = url.split_once("://").map(|(_, rest)| rest).unwrap_or("");
    let authority = host_part.split(['/', '?', '#']).next().unwrap_or("");
    if !scheme_ok
        || authority.is_empty()
        || authority.contains('@')
        || url.len() > 2048
        || url.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return None;
    }
    Some(url.to_string())
}

/// The host part of an http(s) URL accepted by `safe_url`, without port.
fn url_host(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or("");
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.starts_with('[') {
        return authority; // IPv6 literal: ASCII anyway
    }
    authority.split(':').next().unwrap_or("")
}

/// For a link whose host has non-ASCII characters (an internationalised
/// domain, which can imitate another site: "аpple.com" with a Cyrillic а),
/// the host in its ASCII "xn--" form, which is what the browser really
/// visits and what makes the imitation visible. `None` for plain ASCII
/// hosts.
pub fn idn_ascii_host(url: &str) -> Option<String> {
    let host = url_host(url);
    if host.is_ascii() {
        return None;
    }
    let labels: Option<Vec<String>> = host
        .split('.')
        .map(|label| {
            if label.is_ascii() {
                Some(label.to_ascii_lowercase())
            } else {
                punycode_encode(&label.to_lowercase()).map(|p| format!("xn--{p}"))
            }
        })
        .collect();
    // If it can't be encoded, still flag it rather than hiding the warning.
    Some(labels.map(|l| l.join(".")).unwrap_or_else(|| "(cannot be shown)".into()))
}

/// RFC 3492 Punycode encoding of one label. Checked arithmetic throughout:
/// release builds abort on overflow, and the input comes from the screen.
fn punycode_encode(input: &str) -> Option<String> {
    const BASE: u32 = 36;
    const TMIN: u32 = 1;
    const TMAX: u32 = 26;
    const SKEW: u32 = 38;
    const DAMP: u32 = 700;

    fn adapt(mut delta: u32, numpoints: u32, first: bool) -> u32 {
        delta /= if first { DAMP } else { 2 };
        delta += delta / numpoints;
        let mut k = 0;
        while delta > ((BASE - TMIN) * TMAX) / 2 {
            delta /= BASE - TMIN;
            k += BASE;
        }
        k + ((BASE - TMIN + 1) * delta) / (delta + SKEW)
    }

    fn digit(d: u32) -> char {
        // 0..=25 -> a..=z, 26..=35 -> 0..=9
        let b = if d < 26 { b'a' + d as u8 } else { b'0' + (d - 26) as u8 };
        b as char
    }

    let cps: Vec<u32> = input.chars().map(u32::from).collect();
    let mut out: String = input.chars().filter(char::is_ascii).collect();
    let basic = u32::try_from(out.len()).ok()?;
    let total = u32::try_from(cps.len()).ok()?;
    if basic > 0 {
        out.push('-');
    }
    let (mut n, mut delta, mut bias, mut h) = (128u32, 0u32, 72u32, basic);
    while h < total {
        let m = cps.iter().copied().filter(|&c| c >= n).min()?;
        delta = delta.checked_add((m - n).checked_mul(h + 1)?)?;
        n = m;
        for &c in &cps {
            if c < n {
                delta = delta.checked_add(1)?;
            }
            if c == n {
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = if k <= bias {
                        TMIN
                    } else if k >= bias + TMAX {
                        TMAX
                    } else {
                        k - bias
                    };
                    if q < t {
                        break;
                    }
                    out.push(digit(t + (q - t) % (BASE - t)));
                    q = (q - t) / (BASE - t);
                    k = k.checked_add(BASE)?;
                }
                out.push(digit(q));
                bias = adapt(delta, h + 1, h == basic);
                delta = 0;
                h += 1;
            }
        }
        delta = delta.checked_add(1)?;
        n = n.checked_add(1)?;
    }
    Some(out)
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
        assert!(safe_url("https:///etc/passwd").is_none());
        assert!(safe_url("https://bank.example@evil.example/").is_none());
        assert!(safe_url("https://user:pw@example.com").is_none());
        assert!(safe_url("https://example.com/~me/@handle?a=b@c").is_some());
        assert!(safe_url("https://exa mple.com").is_none());
        assert!(safe_url("https://example.com/\x1b[31m").is_none());
        assert!(safe_url(&format!("https://e.com/{}", "a".repeat(3000))).is_none());
    }

    #[test]
    fn punycode() {
        // RFC 3492 / IDNA examples.
        assert_eq!(punycode_encode("bücher").as_deref(), Some("bcher-kva"));
        assert_eq!(punycode_encode("münchen").as_deref(), Some("mnchen-3ya"));
        assert_eq!(punycode_encode("пример").as_deref(), Some("e1afmkfd"));
    }

    #[test]
    fn international_hosts() {
        assert_eq!(idn_ascii_host("https://example.com/ü"), None);
        assert_eq!(
            idn_ascii_host("https://www.Bücher.example:8443/x").as_deref(),
            Some("www.xn--bcher-kva.example")
        );
        // "аpple.com" with a Cyrillic а
        assert_eq!(idn_ascii_host("https://\u{430}pple.com/").as_deref(), Some("xn--pple-43d.com"));
    }

    #[test]
    fn escaping() {
        assert_eq!(regex_escape("a.b*c"), r"a\.b\*c");
        assert_eq!(regex_escape("(x)[y]{z}"), r"\(x\)\[y\]\{z\}");
        assert_eq!(regex_escape(r"c:\path"), r"c:\\path");
        assert_eq!(regex_escape("plain"), "plain");
    }
}
