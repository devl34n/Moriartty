# Changelog

All notable changes to Moriartty. The packages carry the same notes
(`packaging/moriartty.spec`, AppStream metadata).

## 0.9.5 — 2026-10-24

The first regular (non pre-release) release.

Sessions
- A saved session can run a command instead of the shell (Session page:
  "Command to run instead"), e.g. `ssh admin@prod.example.com` or
  `toolbox enter dev`. It is started directly, never through a shell; a
  command that can't be found falls back to the shell with a notice. `-e`
  still wins over it.
- A ▾ next to the + button, Ctrl+Shift+S and the menu's Saved Sessions list
  the saved sessions; picking one opens it in a new window with all of its
  settings (colours included).
- Settings window: the "Save as Default Settings" tick is gone. Apply saves
  to the session highlighted on the Session page, which starts as the
  window's own session (or Default Settings). Clicking a session only
  highlights it; double-click loads it into the form, without applying or
  closing. The window is always titled "Moriartty Configuration".

Shell integration
- Scripts for bash, zsh and fish (`/usr/share/moriartty/shell-integration/`,
  opt-in: source one from your shell's startup file) mark prompts and
  commands with OSC 133.
- Ctrl+Shift+Up / Ctrl+Shift+Down jump to the previous / next prompt.
  Without the script the keys go to the program as before.
- When a command that ran 10 s or more (Features page; 0 turns it off) ends
  in a background tab, the tab's name turns bold and the desktop shows a
  notice with the tab's number and the exit status, never the command or
  its output.
- Programs see `TERM_PROGRAM=moriartty` and `TERM_PROGRAM_VERSION`.

Keyboard
- Every shortcut can be changed or turned off on the Keyboard page (new
  `keys` settings table). Bad entries are reported in the Event Log and
  skipped; the other shortcuts keep working.

Releases
- Only `-rc` tags are published as pre-releases now.

## 0.9.0 — 2026-10-21 (release candidate)

Includes the 0.7 and 0.8 work, which was not released separately.

Lightweight and robust
- Idle tabs no longer wake up: a tab's name ("program: directory") is re-read
  shortly after output, Enter or a directory change reported by the shell
  (OSC 7), instead of every second. The directory itself always comes from
  the shell (`/proc`), never from OSC 7.
- The link pattern is compiled once and shared by all tabs.
- Fixed: Ctrl+click did not open links. Links now go straight to the
  desktop's default browser through GIO instead of GTK's launcher, which can
  use the portal (and the portal refuses non-dumpable processes); failures
  are shown and logged.
- Fixed: background opacity showed a darker colour instead of the desktop
  when the GTK theme paints backgrounds on stacks or boxes. Moriartty's
  widgets between the window and the terminal are now kept clear.
- Fixed: a colour scheme picked in Settings was only kept after pressing
  Try. Picking it now applies it at once (live preview, Cancel undoes it);
  the Try button is gone.
- No settings borrow is held while VTE is reconfigured.
- Built with gtk-rs 0.22 (gtk4 0.11, vte4 0.10), nix 0.31 and toml 1; the
  duplicate toml 0.8 crates are gone. Building needs Rust 1.92; on Debian 13
  the `.deb` build takes it from trixie-backports.
- Performance budget in `docs/PERFORMANCE.md`, measured with `perf-check.sh`.

Assurance
- Tests for settings and log files: symlinks, other users' files, permission
  repair, oversized files, FIFOs and devices, append and overwrite. They run
  in a private temporary directory; tests no longer read or write the real
  `~/.config` (before, they could move an unreadable Default Settings aside).
- `tests/evil_bytes.rs` feeds hostile output (OSC 52, title reports, DECRQSS,
  XTGETTCAP, answerback, paste markers) into VTE and checks that no answer
  carries attacker-chosen text and the clipboard never changes.
- CI: formatting check, wider clippy lints, a minimum-Rust-version job, the
  evil bytes test under a virtual display, Dependabot.
- Releases never replace files already published.
- Settings saved by a newer Moriartty keep their format version.

Release candidate
- Man page and bash, zsh and fish completions in both packages.
- `docs/THREAT_MODEL.md`, `docs/SETTINGS_FORMAT.md` (format frozen at
  version 2), `CONTRIBUTING.md` and this changelog.
- New screenshots, with tabs and the new look.

## 0.6.0 — 2026-10-20

- New default colours, Moriartty Dark and Light, following the desktop's light
  or dark style; dialogs and menus follow it too.
- The title bar takes the terminal's colours (not with high contrast).
- Tabs: underline on the current tab, close button on hover, drag to reorder,
  mouse-wheel scrolling when they don't fit.
- 6 px default gap between text and window edge; optional background opacity.
- One settings window per terminal window, with a live colour preview that
  Cancel undoes.
- Square app icons and a symbolic icon; screen readers see tabs as tabs.

## 0.5.0 — 2026-10-20

- Selected text goes to PRIMARY only; copying it to the clipboard too is an
  option, off by default.
- Ask before opening OSC 8 links and international domain names; refuse links
  with invisible or direction-changing characters or a backslash in the host.
- Paste preview also for single lines with hidden characters; Copy All asks
  before copying a large scrollback.
- A tab shows when its output is being logged.
- An unreadable Default Settings file is moved aside to `.toml.bad`.
- Settings files carry a format version; the Event Log is capped.
- A program that ignores the hangup is killed when its tab closes.

## 0.4.x — 2026-10-19

- Tabs with keyboard shortcuts, one slim title bar, tabs named after what
  runs in them.
- Bad saved shell, terminal type or log file no longer stop Moriartty
  starting; safer paste confirmation; links with a user name are refused.

## 0.3.0 — 2026-10-18

- Usable as the desktop's default terminal (`-dir`, `-hold`); AppStream
  metadata; zoom remembered.

## 0.2.0 — 2026-10-18

- Find in scrollback, font zoom, Ctrl+click links, colour schemes.

## 0.1.0 — 2026-10-18

- First release.
