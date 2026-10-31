//! Command lines for the `command` setting, typed as one line of text in the
//! settings window. Split into words like a shell does for plain words,
//! quotes and backslashes, and nothing more: no variables, globs, pipes or
//! `;`. The words are exec'd directly, never through a shell.

/// Split a command line into its words. An unclosed quote runs to the end
/// of the line, so every line has a meaning and nothing typed is lost.
pub fn split(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    cur.push(c);
                }
            }
            '"' => {
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        // As in a shell, only these lose their backslash.
                        '\\' => match chars.next() {
                            Some(n @ ('"' | '\\' | '$' | '`')) => cur.push(n),
                            Some(n) => {
                                cur.push('\\');
                                cur.push(n);
                            }
                            None => cur.push('\\'),
                        },
                        c => cur.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    words
}

/// The words as one line that `split` turns back into the same words.
pub fn join(words: &[String]) -> String {
    words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
}

fn quote(w: &str) -> String {
    let plain = !w.is_empty()
        && w.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,~^".contains(c));
    if plain {
        w.to_string()
    } else {
        format!("'{}'", w.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn splits_like_a_shell() {
        assert_eq!(
            split("ssh  admin@prod.example.com"),
            words(&["ssh", "admin@prod.example.com"])
        );
        assert_eq!(
            split(r#"a 'b c' "d \"e\" \n" f\ g"#),
            words(&["a", "b c", r#"d "e" \n"#, "f g"])
        );
        assert_eq!(
            split("echo $HOME; ls | wc"),
            words(&["echo", "$HOME;", "ls", "|", "wc"])
        );
        assert!(split("   ").is_empty());
        assert_eq!(split("''"), words(&[""]));
        assert_eq!(split("ssh 'my host"), words(&["ssh", "my host"]));
    }

    #[test]
    fn join_round_trips() {
        for case in [
            words(&["ssh", "prod"]),
            words(&["sh", "-c", "echo it's $HOME"]),
            words(&["", "a b", "\"q\"", "back\\slash"]),
            Vec::new(),
        ] {
            assert_eq!(split(&join(&case)), case);
        }
        assert_eq!(join(&words(&["ssh", "admin@prod"])), "ssh admin@prod");
    }
}
