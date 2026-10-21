# Settings file format

Saved sessions live in `$XDG_CONFIG_HOME/moriartty/sessions/` (default
`~/.config/moriartty/sessions/`), one TOML file per session, named after it:
`Default Settings.toml`, `big font.toml`. The directory is 0700 and the
files 0600.

## Stability promise (from 0.9)

The format is frozen at **`config_version = 2`**:

* Every Moriartty from 0.9 on reads files written by any earlier version,
  and files written by later versions.
* A key, once defined, keeps its name, type and meaning. Keys are never
  reused for something else.
* Unknown keys are ignored, so a file written by a newer Moriartty still
  loads in an older one (the new settings are simply not used).
* Missing keys take their default, so files may list only what they change.
* A value out of range is clamped, and an invalid shell, terminal type or log
  file falls back to a safe default with a notice. A file that is not valid
  TOML is moved aside to `….toml.bad` (Default Settings) or reported (other
  sessions); it is never overwritten.
* If the meaning of the format ever has to change, `config_version` goes up
  and the file is migrated on load (`Settings::migrate`), as for versions 1
  and 2 below.

## Keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `config_version` | integer | 2 | Format version; missing means 0 (before 0.5) |
| `shell` | string | `""` | Absolute path of the program to run; empty = login shell |
| `login_shell` | bool | false | Start the shell as a login shell |
| `close_on_exit` | `"always"`, `"never"`, `"clean"` | `"clean"` | Close the tab when the program exits |
| `log_type` | `"none"`, `"printable"`, `"all"` | `"none"` | Session logging |
| `log_file` | string | `"~/moriartty-&Y&M&D-&T.log"` | Absolute path; `&Y &M &D &T &&` placeholders |
| `log_exists` | `"ask"`, `"overwrite"`, `"append"` | `"ask"` | Existing log file |
| `log_flush` | bool | true | Flush the log often |
| `log_header` | bool | true | Write a header line |
| `term_type` | string | `"xterm-256color"` | `$TERM` |
| `blink_text` | bool | false | Blinking text |
| `cjk_ambig_wide` | bool | false | CJK ambiguous-width characters are wide |
| `backspace` | `"control_h"`, `"control_question"` | `"control_question"` | What Backspace sends |
| `bell` | `"none"`, `"audible"`, `"visual"` | `"audible"` | Bell style |
| `bell_overload` | bool | true | Silence an over-used bell |
| `bell_overload_n` | integer 1–1000 | 5 | … this many bells |
| `bell_overload_t` | integer 1–3600 | 2 | … in this many seconds |
| `bell_overload_s` | integer 1–3600 | 5 | Seconds of silence |
| `no_remote_wintitle` | bool | false | Ignore titles set by programs |
| `allow_hyperlinks` | bool | false | OSC 8 hyperlinks |
| `clickable_urls` | bool | true | Ctrl+click opens http(s) links |
| `columns` | integer 1–1000 | 80 | Initial width |
| `rows` | integer 1–500 | 24 | Initial height |
| `scrollback` | integer 0–1000000 | 10000 | Lines of scrollback |
| `scrollbar` | bool | true | Show the scrollbar |
| `scroll_on_key` | bool | true | Scroll to the bottom on a key press |
| `scroll_on_output` | bool | false | Scroll to the bottom on output |
| `cursor` | `"block"`, `"underline"`, `"vertical"` | `"block"` | Cursor shape |
| `cursor_blink` | bool | false | Blinking cursor |
| `font` | string | `"Monospace 12"` | Pango font description |
| `font_zoom` | integer 30–400 | 100 | Zoom in percent |
| `hide_mouse` | bool | true | Hide the pointer while typing |
| `window_border` | integer 0–64 | 6 | Gap between text and window edge, in pixels |
| `background_opacity` | integer 30–100 | 100 | Background opacity in percent |
| `wintitle` | string | `""` | Window title |
| `warn_on_close` | bool | true | Ask before closing with a running program |
| `fullscreen_on_alt_enter` | bool | false | Alt+Enter toggles full screen |
| `title_buttons` | bool | true | Settings and menu buttons in the title bar |
| `mouse_action` | `"compromise"`, `"windows"`, `"xterm"` | `"compromise"` | Mouse button model |
| `copy_to_clipboard` | bool | false | Also copy selections to the clipboard |
| `word_chars` | string | `"-,./?%&#:_=+@~"` | Extra characters that are part of a word |
| `paste_controls` | bool | false | Allow control characters in pastes (unsafe) |
| `confirm_multiline_paste` | bool | true | Ask before risky pastes |
| `bold_as_colour` | bool | true | Bold text in a brighter colour |
| `auto_colours` | bool | true | Moriartty Dark/Light following the desktop |
| `colours` | table | Moriartty Dark | Used when `auto_colours` is false |

`colours` has `foreground`, `bold_foreground`, `background`, `cursor_text`
and `cursor`, each `[r, g, b]` (0–255), and `ansi`, sixteen `[r, g, b]`
entries: 0–7 normal, 8–15 bright.

## History

* **0** (before 0.5): no `config_version`. `copy_on_select` existed.
* **1** (0.5): `copy_on_select` replaced by `copy_to_clipboard`, off by
  default; the old key is ignored.
* **2** (0.6): files from before are switched to `auto_colours = true`
  (their colours stay in `colours`), and a `window_border` still at the old
  default 1 becomes 6.
