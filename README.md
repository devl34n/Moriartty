# Moriartty

A PuTTY-style terminal emulator for Linux (think PuTTY's `pterm`), written in
Rust with GTK 4 + VTE.

![Moriartty terminal window](docs/screenshots/terminal.png)

![Moriartty settings window](docs/screenshots/settings.png)

* Launching opens a terminal running your shell straight away.
* **Copy on select** and **right-click paste** are on by default (PuTTY's
  "Compromise" mouse mode).
* The title bar has a **⚙ Settings** button and a **☰ menu**. The same menu
  opens with **Ctrl+Right-click**: Copy, Paste, Copy All, Find, Zoom, New
  Window, Restart Shell, Settings, Event Log, Clear Scrollback, Reset
  Terminal, Full Screen, About.
* **Find in scrollback** (Ctrl+Shift+F) and **font zoom** (Ctrl+= / Ctrl+- /
  Ctrl+0).
* **Ctrl+click opens web links.** Only `http`/`https` links are opened, and
  the full address is shown in a tooltip first.
* **Colour schemes:** PuTTY classic, Tango dark/light, Solarized dark/light,
  Dracula, plus every colour adjustable individually.
* The settings follow PuTTY's tree: Session (shell, saved sessions, close on
  exit) · Logging · Terminal (Keyboard, Bell, Features) · Window (Appearance,
  Behaviour, Selection, Colours).
* "Default Settings" applies to every new window. Changes made in the
  settings window are saved there unless you untick **Save as Default
  Settings**. Other saved sessions are settings profiles you can use with
  `-load`.

## Install on Fedora (COPR)

```sh
sudo dnf copr enable devl34n/moriartty
sudo dnf install moriartty
```

Updates then arrive with normal `dnf upgrade`.

## Install a prebuilt package

Ready-made packages are in [`releases/`](releases/). Check them against
`SHA256SUMS` first:

```sh
sha256sum -c SHA256SUMS --ignore-missing
sudo dnf install ./moriartty-*.rpm      # Fedora
sudo apt install ./moriartty_*.deb      # Debian 13+ / Ubuntu 25.04+
```

## Build packages (from any Linux distribution)

Both packages are built inside containers of their target distribution, so
the only thing you need is **podman** (or docker). Rust, GTK and the
packaging tools are installed inside the container, not on your machine.

```sh
./build-rpm.sh     # Fedora container    -> dist/rpm/moriartty-<version>-1.fc*.x86_64.rpm
./build-deb.sh     # Debian 13 container -> dist/deb/moriartty_<version>-1_amd64.deb
```

You can pass a different image to target another release:

```sh
./build-rpm.sh registry.fedoraproject.org/fedora:42
./build-deb.sh docker.io/library/ubuntu:25.04
```

Targets need GTK ≥ 4.12, VTE (GTK 4) ≥ 0.76 and Rust ≥ 1.80. That means
Fedora 39+, Debian 13+ or Ubuntu 25.04+. Debian 12 and Ubuntu 24.04 are too
old.

For development with a local toolchain: `cargo run`, `cargo test`.

## Command line

```
moriartty                     # terminal with your shell
moriartty -config             # settings window first
moriartty -load "big font"    # use a saved session
moriartty -geometry 120x40 -title build -e make -j8
moriartty -ls                 # login shell
moriartty -log ~/term.log     # log all output
```

## Shortcuts

| Shortcut | Action |
|---|---|
| Ctrl+Shift+C, Ctrl+Insert | Copy |
| Ctrl+Shift+V, Shift+Insert | Paste |
| Ctrl+Shift+F | Find in scrollback. Enter goes to older matches, Ctrl+G to newer ones, Escape closes. |
| Ctrl+= / Ctrl+- / Ctrl+0 | Zoom in / out / back to normal (this window only) |
| Ctrl+click | Open the web link under the pointer |
| Ctrl+Shift+N | New window in the same directory |
| Ctrl+Shift+P | Settings |
| Alt+Enter | Full screen, if enabled on the Behaviour page |

## Security design

* **Paste:** escape and other control characters are stripped. The
  bracketed-paste terminator is always removed, so pasted text can't escape
  bracketed-paste mode. Multi-line pastes ask for confirmation. Middle-click
  (PRIMARY selection) pastes go through the same filter.
* **What programs can do to the terminal:** no answerback, no title
  reporting, and no clipboard writes (OSC 52). Window titles set by programs
  have control characters stripped and their length capped. OSC 8 hyperlinks
  are off by default.
* **Links:** only `http://` and `https://` URLs can be opened, never `file:`
  or custom schemes. Opening one always takes a Ctrl+click, and the tooltip
  shows the full address first.
* **Files:** the config directory is 0700 and session files are 0600, written
  atomically. Symlinks and files owned by other users are refused. Log files
  are 0600 and opened with `O_NOFOLLOW`.
* **Running programs:** the shell and `-e` commands are started directly,
  never through an extra `sh -c`. A custom shell must be an absolute path to
  an executable.
* **Process:** Moriartty is non-dumpable (`PR_SET_DUMPABLE=0`), so there are
  no core dumps of your scrollback and other programs running as you can't
  ptrace it. All `unsafe` code is in `src/sys.rs`. Release builds keep
  overflow checks on and abort on panic.

## PuTTY settings not available

* **Not exposed by VTE:** function-key and Home/End sequence variants,
  initial keypad and cursor-key modes, disabling mouse reporting or the
  alternate screen, and font quality.
* **Character sets:** the terminal is UTF-8 only.
* **Shift+click:** Shift+Right-click and Shift+Middle-click go to the
  terminal application, for mouse-aware programs like vim or htop.
