//! Keyboard shortcuts: Moriartty's actions, their default keys, and the
//! keys the user chose instead (the `keys` setting).
//!
//! Keys are written like "Ctrl+Shift+T", "Alt+1", "Ctrl+PageUp" or
//! "Ctrl+plus": modifiers (Ctrl, Shift, Alt, Super) and one key, either a
//! single character or a GDK key name. "none" turns a shortcut off.

use std::collections::BTreeMap;

use gtk::gdk;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Copy,
    Paste,
    Find,
    Settings,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Fullscreen,
    PreviousPrompt,
    NextPrompt,
    NewTab,
    CloseTab,
    NewWindow,
    Sessions,
    PreviousTab,
    NextTab,
    MoveTabLeft,
    MoveTabRight,
    DetachTab,
    /// Tab 1-9, counted from 0.
    GoToTab(usize),
}

pub struct Def {
    pub action: Action,
    /// The action's name in the `keys` setting.
    pub name: &'static str,
    /// What the settings window calls it.
    pub label: &'static str,
    pub keys: &'static [&'static str],
}

/// Every action, in the order the settings window lists them.
pub const ACTIONS: &[Def] = &[
    Def {
        action: Action::Copy,
        name: "copy",
        label: "Copy",
        keys: &["Ctrl+Shift+C", "Ctrl+Insert"],
    },
    Def {
        action: Action::Paste,
        name: "paste",
        label: "Paste",
        keys: &["Ctrl+Shift+V", "Shift+Insert"],
    },
    Def {
        action: Action::Find,
        name: "find",
        label: "Find in scrollback",
        keys: &["Ctrl+Shift+F"],
    },
    Def {
        action: Action::Settings,
        name: "settings",
        label: "Settings",
        keys: &["Ctrl+Shift+P"],
    },
    Def {
        action: Action::ZoomIn,
        name: "zoom-in",
        label: "Zoom in",
        keys: &["Ctrl+equal", "Ctrl+plus", "Ctrl+Shift+plus", "Ctrl+KP_Add"],
    },
    Def {
        action: Action::ZoomOut,
        name: "zoom-out",
        label: "Zoom out",
        keys: &["Ctrl+minus", "Ctrl+KP_Subtract"],
    },
    Def {
        action: Action::ZoomReset,
        name: "zoom-reset",
        label: "Normal size",
        keys: &["Ctrl+0", "Ctrl+KP_0"],
    },
    Def {
        action: Action::Fullscreen,
        name: "fullscreen",
        label: "Full screen",
        keys: &["Alt+Return", "Alt+KP_Enter"],
    },
    Def {
        action: Action::PreviousPrompt,
        name: "previous-prompt",
        label: "Jump to the previous prompt",
        keys: &["Ctrl+Shift+Up"],
    },
    Def {
        action: Action::NextPrompt,
        name: "next-prompt",
        label: "Jump to the next prompt",
        keys: &["Ctrl+Shift+Down"],
    },
    Def {
        action: Action::NewTab,
        name: "new-tab",
        label: "New tab",
        keys: &["Ctrl+Shift+T"],
    },
    Def {
        action: Action::CloseTab,
        name: "close-tab",
        label: "Close tab",
        keys: &["Ctrl+Shift+W"],
    },
    Def {
        action: Action::NewWindow,
        name: "new-window",
        label: "New window",
        keys: &["Ctrl+Shift+N"],
    },
    Def {
        action: Action::Sessions,
        name: "sessions",
        label: "Saved sessions menu",
        keys: &["Ctrl+Shift+S"],
    },
    Def {
        action: Action::PreviousTab,
        name: "previous-tab",
        label: "Previous tab",
        keys: &["Ctrl+Page_Up", "Ctrl+Shift+Tab"],
    },
    Def {
        action: Action::NextTab,
        name: "next-tab",
        label: "Next tab",
        keys: &["Ctrl+Page_Down", "Ctrl+Tab"],
    },
    Def {
        action: Action::MoveTabLeft,
        name: "move-tab-left",
        label: "Move tab left",
        keys: &["Ctrl+Shift+Page_Up"],
    },
    Def {
        action: Action::MoveTabRight,
        name: "move-tab-right",
        label: "Move tab right",
        keys: &["Ctrl+Shift+Page_Down"],
    },
    Def {
        action: Action::DetachTab,
        name: "detach-tab",
        label: "Detach tab into a new window",
        keys: &["Ctrl+Shift+D"],
    },
    Def {
        action: Action::GoToTab(0),
        name: "tab-1",
        label: "Go to tab 1",
        keys: &["Alt+1"],
    },
    Def {
        action: Action::GoToTab(1),
        name: "tab-2",
        label: "Go to tab 2",
        keys: &["Alt+2"],
    },
    Def {
        action: Action::GoToTab(2),
        name: "tab-3",
        label: "Go to tab 3",
        keys: &["Alt+3"],
    },
    Def {
        action: Action::GoToTab(3),
        name: "tab-4",
        label: "Go to tab 4",
        keys: &["Alt+4"],
    },
    Def {
        action: Action::GoToTab(4),
        name: "tab-5",
        label: "Go to tab 5",
        keys: &["Alt+5"],
    },
    Def {
        action: Action::GoToTab(5),
        name: "tab-6",
        label: "Go to tab 6",
        keys: &["Alt+6"],
    },
    Def {
        action: Action::GoToTab(6),
        name: "tab-7",
        label: "Go to tab 7",
        keys: &["Alt+7"],
    },
    Def {
        action: Action::GoToTab(7),
        name: "tab-8",
        label: "Go to tab 8",
        keys: &["Alt+8"],
    },
    Def {
        action: Action::GoToTab(8),
        name: "tab-9",
        label: "Go to tab 9",
        keys: &["Alt+9"],
    },
];

/// The modifiers a shortcut can use; Caps Lock, Num Lock and the like are
/// ignored.
fn modifiers(state: gdk::ModifierType) -> gdk::ModifierType {
    state
        & (gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::SHIFT_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK)
}

/// With Shift held GDK reports T instead of t, and ISO_Left_Tab instead of
/// Tab; shortcuts compare the plain key, with Shift as a modifier.
fn plain(key: gdk::Key) -> gdk::Key {
    let key = key.to_lower();
    if key == gdk::Key::ISO_Left_Tab {
        gdk::Key::Tab
    } else {
        key
    }
}

/// A key that types a character outside ASCII: a letter on a Greek or Cyrillic
/// layout, say, where no shortcut can match it as it is.
pub fn is_non_ascii(key: gdk::Key) -> bool {
    key.to_unicode().is_some_and(|c| !c.is_ascii())
}

/// What the user's Latin layout has on a physical key, given what all the
/// layouts have on it, as (level, key): the first ASCII key without Shift.
/// None if no layout has one.
pub fn latin_equivalent(levels: impl IntoIterator<Item = (i32, gdk::Key)>) -> Option<gdk::Key> {
    levels
        .into_iter()
        .filter(|&(level, _)| level == 0)
        .map(|(_, key)| key)
        .find(|key| key.to_unicode().is_some_and(|c| c.is_ascii_graphic()))
}

/// VTE's own clipboard keys: Shift+Insert and Ctrl+Shift+Insert paste, and
/// Ctrl+Insert copies. They must never reach it. Its paste goes past the filter
/// and the question, and a shortcut that was turned off has to stay off.
pub fn is_vte_clipboard_key(key: gdk::Key, state: gdk::ModifierType) -> bool {
    (key == gdk::Key::Insert || key == gdk::Key::KP_Insert)
        && state.intersects(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK)
}

/// The GDK key name of a punctuation character, which is what
/// `Key::from_name` knows it by ("," is "comma"). Letters and digits are
/// their own names.
fn punctuation_name(c: char) -> Option<&'static str> {
    Some(match c {
        ' ' => "space",
        '!' => "exclam",
        '"' => "quotedbl",
        '#' => "numbersign",
        '$' => "dollar",
        '%' => "percent",
        '&' => "ampersand",
        '\'' => "apostrophe",
        '(' => "parenleft",
        ')' => "parenright",
        '*' => "asterisk",
        '+' => "plus",
        ',' => "comma",
        '-' => "minus",
        '.' => "period",
        '/' => "slash",
        ':' => "colon",
        ';' => "semicolon",
        '<' => "less",
        '=' => "equal",
        '>' => "greater",
        '?' => "question",
        '@' => "at",
        '[' => "bracketleft",
        '\\' => "backslash",
        ']' => "bracketright",
        '^' => "asciicircum",
        '_' => "underscore",
        '`' => "grave",
        '{' => "braceleft",
        '|' => "bar",
        '}' => "braceright",
        '~' => "asciitilde",
        _ => return None,
    })
}

fn key_named(name: &str) -> Option<gdk::Key> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return match punctuation_name(c) {
            Some(n) => gdk::Key::from_name(n),
            None => gdk::Key::from_name(name),
        };
    }
    let alias = match name.to_ascii_lowercase().as_str() {
        "pgup" | "pageup" | "page_up" | "prior" => "Page_Up",
        "pgdn" | "pagedown" | "page_down" | "next" => "Page_Down",
        "enter" | "return" => "Return",
        "esc" | "escape" => "Escape",
        "ins" | "insert" => "Insert",
        "del" | "delete" => "Delete",
        "backspace" => "BackSpace",
        "tab" => "Tab",
        "space" => "space",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "home" => "Home",
        "end" => "End",
        _ => "",
    };
    if !alias.is_empty() {
        return gdk::Key::from_name(alias);
    }
    gdk::Key::from_name(name).or_else(|| {
        // f11 → F11
        let mut c = name.chars();
        let first = c.next()?.to_ascii_uppercase();
        gdk::Key::from_name(format!("{first}{}", c.as_str()))
    })
}

/// Parse one shortcut, e.g. "Ctrl+Shift+T".
pub fn parse(spec: &str) -> Result<(gdk::Key, gdk::ModifierType), String> {
    let spec = spec.trim();
    // "Ctrl++" is Ctrl and the + key.
    let (mods, key) = if spec == "+" {
        ("", "plus")
    } else if let Some(m) = spec.strip_suffix("++") {
        (m, "plus")
    } else {
        spec.rsplit_once('+').unwrap_or(("", spec))
    };
    let key = key.trim();
    if key.is_empty() {
        return Err(format!("{spec:?} has no key"));
    }
    let mut state = gdk::ModifierType::empty();
    for m in mods.split('+').map(str::trim).filter(|m| !m.is_empty()) {
        state |= match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => gdk::ModifierType::CONTROL_MASK,
            "shift" => gdk::ModifierType::SHIFT_MASK,
            "alt" => gdk::ModifierType::ALT_MASK,
            "super" => gdk::ModifierType::SUPER_MASK,
            _ => return Err(format!("unknown modifier {m:?} in {spec:?}")),
        };
    }
    let k = key_named(key).ok_or_else(|| format!("unknown key {key:?} in {spec:?}"))?;
    Ok((plain(k), state))
}

/// The `keys` setting for one action, as the settings window shows it:
/// empty for the default, "none" for no keys.
pub fn setting_text(keys: Option<&[String]>) -> String {
    match keys {
        None => String::new(),
        Some([]) => "none".into(),
        Some(k) => k.join(", "),
    }
}

/// Store what was typed in the settings window for one action.
pub fn set_from_text(keys: &mut BTreeMap<String, Vec<String>>, action: &str, text: &str) {
    let specs: Vec<String> = text
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if specs.is_empty() {
        keys.remove(action);
    } else if specs.len() == 1 && specs[0].eq_ignore_ascii_case("none") {
        keys.insert(action.into(), Vec::new());
    } else {
        keys.insert(action.into(), specs);
    }
}

/// Check the `keys` setting; the error lists every problem.
pub fn check(keys: &BTreeMap<String, Vec<String>>) -> Result<(), String> {
    let (_, problems) = Keymap::new(keys, true);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

#[derive(Default)]
pub struct Keymap {
    /// The first match wins: the user's own shortcuts come first.
    bindings: Vec<(gdk::Key, gdk::ModifierType, Action)>,
}

impl Keymap {
    /// The shortcuts from the `keys` setting, and the defaults for every
    /// action it doesn't mention. Full screen's default (Alt+Enter) only
    /// applies when that is turned on (Behaviour page). Returns the
    /// problems found too: bad entries are skipped, never fatal.
    pub fn new(
        keys: &BTreeMap<String, Vec<String>>,
        fullscreen_on_alt_enter: bool,
    ) -> (Keymap, Vec<String>) {
        let mut map = Keymap::default();
        let mut problems = Vec::new();
        for def in ACTIONS {
            if let Some(specs) = keys.get(def.name) {
                map.add(def, specs.iter().map(String::as_str), &mut problems);
            }
        }
        for def in ACTIONS {
            let off = def.action == Action::Fullscreen && !fullscreen_on_alt_enter;
            if !off && !keys.contains_key(def.name) {
                map.add(def, def.keys.iter().copied(), &mut problems);
            }
        }
        for name in keys.keys() {
            if !ACTIONS.iter().any(|d| d.name == name.as_str()) {
                problems.push(format!("Unknown shortcut {name:?} ignored"));
            }
        }
        (map, problems)
    }

    fn add<'a>(
        &mut self,
        def: &Def,
        specs: impl Iterator<Item = &'a str>,
        problems: &mut Vec<String>,
    ) {
        for spec in specs {
            if spec.trim().eq_ignore_ascii_case("none") {
                continue;
            }
            match parse(spec) {
                Ok((key, mods)) => self.bindings.push((key, mods, def.action)),
                Err(e) => problems.push(format!("Shortcut for {}: {e}", def.label)),
            }
        }
    }

    /// The action for a key press, if any.
    pub fn lookup(&self, key: gdk::Key, state: gdk::ModifierType) -> Option<Action> {
        let (key, mods) = (plain(key), modifiers(state));
        self.bindings
            .iter()
            .find(|(k, m, _)| *k == key && *m == mods)
            .map(|b| b.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: gdk::ModifierType = gdk::ModifierType::CONTROL_MASK;
    const SHIFT: gdk::ModifierType = gdk::ModifierType::SHIFT_MASK;
    const ALT: gdk::ModifierType = gdk::ModifierType::ALT_MASK;

    #[test]
    fn parses_shortcuts() {
        assert_eq!(parse("Ctrl+Shift+T").unwrap(), (gdk::Key::t, CTRL | SHIFT));
        assert_eq!(
            parse("ctrl + shift + t").unwrap(),
            (gdk::Key::t, CTRL | SHIFT)
        );
        assert_eq!(parse("Alt+1").unwrap(), (gdk::Key::_1, ALT));
        assert_eq!(parse("Ctrl+PgUp").unwrap(), (gdk::Key::Page_Up, CTRL));
        assert_eq!(
            parse("Ctrl+Shift+Tab").unwrap(),
            (gdk::Key::Tab, CTRL | SHIFT)
        );
        assert_eq!(parse("Ctrl++").unwrap(), (gdk::Key::plus, CTRL));
        assert_eq!(parse("Ctrl+comma").unwrap(), (gdk::Key::comma, CTRL));
        assert_eq!(parse("Ctrl+,").unwrap(), (gdk::Key::comma, CTRL));
        assert_eq!(parse("Ctrl+=").unwrap(), (gdk::Key::equal, CTRL));
        assert_eq!(parse("Ctrl+0").unwrap(), (gdk::Key::_0, CTRL));
        assert_eq!(
            parse("f11").unwrap(),
            (gdk::Key::F11, gdk::ModifierType::empty())
        );
        assert!(parse("Hyper+x").is_err());
        assert!(parse("Ctrl+").is_err());
        assert!(parse("Ctrl+NoSuchKey").is_err());
    }

    #[test]
    fn defaults_are_valid() {
        let (map, problems) = Keymap::new(&BTreeMap::new(), true);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(map.lookup(gdk::Key::T, CTRL | SHIFT), Some(Action::NewTab));
        assert_eq!(
            map.lookup(gdk::Key::ISO_Left_Tab, CTRL | SHIFT),
            Some(Action::PreviousTab)
        );
        assert_eq!(map.lookup(gdk::Key::_3, ALT), Some(Action::GoToTab(2)));
        assert_eq!(
            map.lookup(gdk::Key::D, CTRL | SHIFT),
            Some(Action::DetachTab)
        );
        // Caps Lock doesn't matter; an extra modifier does.
        assert_eq!(
            map.lookup(gdk::Key::C, CTRL | SHIFT | gdk::ModifierType::LOCK_MASK),
            Some(Action::Copy)
        );
        assert_eq!(map.lookup(gdk::Key::C, CTRL | SHIFT | ALT), None);
        let (map, _) = Keymap::new(&BTreeMap::new(), false);
        assert_eq!(map.lookup(gdk::Key::Return, ALT), None);
    }

    #[test]
    fn layouts_without_latin_letters() {
        assert!(is_non_ascii(gdk::Key::Cyrillic_em));
        assert!(is_non_ascii(gdk::Key::Greek_tau));
        assert!(!is_non_ascii(gdk::Key::v));
        assert!(!is_non_ascii(gdk::Key::Page_Up));
        // The V key with a Russian layout first and a US one second.
        let both = [
            (0, gdk::Key::Cyrillic_em),
            (1, gdk::Key::Cyrillic_EM),
            (0, gdk::Key::v),
            (1, gdk::Key::V),
        ];
        assert_eq!(latin_equivalent(both), Some(gdk::Key::v));
        let (map, _) = Keymap::new(&BTreeMap::new(), true);
        assert_eq!(
            map.lookup(latin_equivalent(both).unwrap(), CTRL | SHIFT),
            Some(Action::Paste)
        );
        assert_eq!(latin_equivalent([(0, gdk::Key::Cyrillic_em)]), None);
    }

    #[test]
    fn vte_clipboard_keys() {
        assert!(is_vte_clipboard_key(gdk::Key::Insert, SHIFT));
        assert!(is_vte_clipboard_key(gdk::Key::Insert, CTRL | SHIFT));
        assert!(is_vte_clipboard_key(gdk::Key::Insert, CTRL));
        assert!(is_vte_clipboard_key(gdk::Key::KP_Insert, SHIFT));
        assert!(!is_vte_clipboard_key(
            gdk::Key::Insert,
            gdk::ModifierType::empty()
        ));
        assert!(!is_vte_clipboard_key(gdk::Key::v, CTRL | SHIFT));
    }

    #[test]
    fn user_shortcuts_win() {
        let mut keys = BTreeMap::new();
        set_from_text(&mut keys, "find", "Ctrl+Shift+T, F3");
        set_from_text(&mut keys, "copy", "none");
        set_from_text(&mut keys, "paste", "  ");
        let (map, problems) = Keymap::new(&keys, true);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(map.lookup(gdk::Key::T, CTRL | SHIFT), Some(Action::Find));
        assert_eq!(
            map.lookup(gdk::Key::F3, gdk::ModifierType::empty()),
            Some(Action::Find)
        );
        assert_eq!(map.lookup(gdk::Key::C, CTRL | SHIFT), None);
        assert_eq!(map.lookup(gdk::Key::V, CTRL | SHIFT), Some(Action::Paste));
        assert_eq!(setting_text(keys.get("copy").map(Vec::as_slice)), "none");
        assert_eq!(setting_text(keys.get("paste").map(Vec::as_slice)), "");
        assert_eq!(
            setting_text(keys.get("find").map(Vec::as_slice)),
            "Ctrl+Shift+T, F3"
        );
    }

    #[test]
    fn problems_are_reported_not_fatal() {
        let mut keys = BTreeMap::new();
        keys.insert("no-such-action".to_string(), vec!["F5".to_string()]);
        keys.insert(
            "find".to_string(),
            vec!["Ctrl+Bogus".to_string(), "F3".to_string()],
        );
        let (map, problems) = Keymap::new(&keys, true);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(
            map.lookup(gdk::Key::F3, gdk::ModifierType::empty()),
            Some(Action::Find)
        );
        assert!(check(&keys).is_err());
        assert!(check(&BTreeMap::new()).is_ok());
    }
}
