//! Shell integration: finds the marks a shell puts around its prompts and
//! commands (OSC 133, sent by the scripts in `packaging/shell-integration`)
//! in the program's output. The output is only read, never changed: VTE
//! still gets every byte.
//!
//! Any program can print these marks, so they are only ever used to place
//! prompt jumps and to say that a command finished. Nothing in them is shown
//! except the exit status, as a number.

/// What a mark says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// A prompt starts here (`133;A`).
    Prompt,
    /// The command line was entered and the command starts (`133;C`).
    CommandStart,
    /// The command ended, with its exit status if the shell gave one
    /// (`133;D;status`).
    CommandEnd(Option<i32>),
}

/// Longest OSC body kept. Real marks are a few bytes; longer sequences
/// (titles, hyperlinks, ...) are skipped without being stored.
const MAX_BODY: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    /// After ESC.
    Esc,
    /// Inside an OSC (ESC ]).
    Osc,
    /// ESC inside an OSC: ESC \ ends it, anything else starts over.
    OscEsc,
}

/// Reads the output piece by piece; a mark may be split across pieces.
#[derive(Default)]
pub struct Scanner {
    state: State,
    body: Vec<u8>,
    /// The OSC body is longer than MAX_BODY: not a mark.
    long: bool,
}

impl Scanner {
    /// Scan the next piece of output. Returns the marks that end in it, in
    /// order, each with the offset just after its last byte.
    pub fn scan(&mut self, data: &[u8]) -> Vec<(usize, Mark)> {
        let mut found = Vec::new();
        let mut i = 0;
        while i < data.len() {
            if self.state == State::Ground {
                // Nothing to do until the next ESC.
                match data[i..].iter().position(|&b| b == 0x1b) {
                    Some(n) => {
                        i += n + 1;
                        self.state = State::Esc;
                        continue;
                    }
                    None => break,
                }
            }
            let b = data[i];
            i += 1;
            match (self.state, b) {
                (State::Esc, b']') | (State::OscEsc, b']') => self.start_osc(),
                (State::Esc, 0x1b) => {}
                (State::Esc, _) => self.state = State::Ground,
                (State::Osc, 0x07) | (State::OscEsc, b'\\') => self.finish(i, &mut found),
                (State::Osc, 0x1b) => self.state = State::OscEsc,
                // CAN and SUB cancel the sequence.
                (State::Osc, 0x18 | 0x1a) => self.state = State::Ground,
                (State::Osc, b) => {
                    if self.body.len() < MAX_BODY {
                        self.body.push(b);
                    } else {
                        self.long = true;
                    }
                }
                (State::OscEsc, 0x1b) => self.state = State::Esc,
                (State::OscEsc, _) => self.state = State::Ground,
                // Handled before the match.
                (State::Ground, _) => {}
            }
        }
        found
    }

    fn start_osc(&mut self) {
        self.state = State::Osc;
        self.body.clear();
        self.long = false;
    }

    fn finish(&mut self, end: usize, found: &mut Vec<(usize, Mark)>) {
        self.state = State::Ground;
        if !self.long {
            if let Some(m) = parse(&self.body) {
                found.push((end, m));
            }
        }
    }
}

fn parse(body: &[u8]) -> Option<Mark> {
    let rest = body.strip_prefix(b"133;")?;
    let (&kind, params) = rest.split_first()?;
    match kind {
        b'A' => Some(Mark::Prompt),
        b'C' => Some(Mark::CommandStart),
        b'D' => {
            let status = params.strip_prefix(b";").and_then(|p| {
                let p = p.split(|&b| b == b';').next()?;
                std::str::from_utf8(p).ok()?.parse().ok()
            });
            Some(Mark::CommandEnd(status))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_marks_with_their_end() {
        let data = b"out\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ ";
        let found = Scanner::default().scan(data);
        assert_eq!(
            found,
            vec![(15, Mark::CommandEnd(Some(0))), (23, Mark::Prompt)]
        );
        assert_eq!(&data[23..], b"$ ");
    }

    #[test]
    fn marks_split_across_reads() {
        let mut s = Scanner::default();
        assert!(s.scan(b"ls\r\n\x1b]13").is_empty());
        assert!(s.scan(b"3;C").is_empty());
        assert_eq!(s.scan(b"\x1b\\file"), vec![(2, Mark::CommandStart)]);
        assert_eq!(
            s.scan(b"\x1b]133;D\x1b\\"),
            vec![(9, Mark::CommandEnd(None))]
        );
        assert_eq!(
            s.scan(b"\x1b]133;D;130;aid=1\x07"),
            vec![(18, Mark::CommandEnd(Some(130)))]
        );
    }

    #[test]
    fn other_sequences_are_not_marks() {
        let mut s = Scanner::default();
        assert!(s.scan(b"\x1b]0;133;A title\x07").is_empty());
        assert!(s.scan(b"\x1b[1;31mred\x1b[0m \x1b]133;B\x07").is_empty());
        assert!(s.scan(b"\x1b]133;D;notanumber\x07").len() == 1);
        // Too long, cancelled, or interrupted by another sequence.
        let mut long = b"\x1b]133;A;".to_vec();
        long.extend(std::iter::repeat_n(b'x', MAX_BODY));
        long.push(0x07);
        assert!(s.scan(&long).is_empty());
        assert!(s.scan(b"\x1b]133;A\x18\x07").is_empty());
        assert!(s.scan(b"\x1b]133\x1b[m;A\x07").is_empty());
        // ...after which marks are found again.
        assert_eq!(s.scan(b"\x1b\x1b]133;A\x07"), vec![(9, Mark::Prompt)]);
    }
}
