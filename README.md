# Moriartty

A fast, private-by-default terminal emulator for Linux, inspired by PuTTY's
`pterm`. Written in Rust with GTK 4 + VTE.

![Moriartty terminal window](docs/screenshots/terminal.png)

![Moriartty settings window](docs/screenshots/settings.png)

* Launching opens a terminal running your shell straight away.
* **Tabs**, in a single slim title bar. Click the **+** right after the
  tabs for a new one (or Ctrl+Shift+T); middle-click a tab to close it. Tabs are named like kitty's: the title the program sets, otherwise
  "program: directory" (`bash: ~/src`, `vim: ~/src`). A bell in a
  background tab makes its name bold.
* **Select to copy, right-click to paste** (PuTTY's "Compromise" mouse
  mode). Selections go to the PRIMARY selection only, so clipboard managers
  don't see them. Ctrl+Shift+C copies to the clipboard, and Settings →
  Selection can send every selection there too.
* The title bar has the tabs, a **⚙ Settings** button and a **☰ menu**; the
  two buttons can be hidden for a minimal look. The same menu opens with **Ctrl+Right-click**:
  Copy, Paste, Copy All, Find, Zoom, New Tab, New Window, Close Tab, Restart Shell, Settings,
  Event Log, Clear Scrollback, Reset Terminal, Full Screen, About.
* **Find in scrollback** (Ctrl+Shift+F) and **font zoom** (Ctrl+= / Ctrl+- /
  Ctrl+0). New windows inherit the zoom level, and it can be saved as the
  default.
* **Works as your desktop's default terminal**, for "Open in Terminal" and
  apps that need a terminal (see below).
* **Ctrl+click opens web links** in your default browser. Only
  `http`/`https` links are opened, and the full address is shown in a
  tooltip first. If a link can't be opened, Moriartty says why.
* **Follows your desktop's light or dark style** with the Moriartty Dark
  and Light colours, and the title bar takes the terminal's colours.
  Other schemes: PuTTY classic, Tango dark/light, Solarized dark/light,
  Dracula. Picking a scheme, or any single colour, shows in the terminal
  at once; Cancel undoes it. Optional background transparency
  (Appearance → Background opacity).
* **Light when idle:** a tab that shows nothing new does no work at all.
* **Drag tabs** to reorder them; the mouse wheel scrolls through them when
  they don't all fit.
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

Ready-made packages are on the
[Releases page](https://github.com/devl34n/Moriartty/releases), with a
`SHA256SUMS` file signed by the author (`SHA256SUMS.asc`). Download the
package, both checksum files and `signing-key.asc`, then check the signature
and the package:

```sh
gpg --import signing-key.asc                # once; compare the fingerprint
                                            # with the one the author publishes
gpg --verify SHA256SUMS.asc SHA256SUMS
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
./build-deb.sh docker.io/library/ubuntu:26.04
```

Targets need GTK ≥ 4.12 and VTE (GTK 4) ≥ 0.76; building needs Rust ≥ 1.92
(for the gtk-rs 0.22 bindings). Current Fedora has all of them. On Debian 13
`build-deb.sh` takes the compiler from trixie-backports, because Debian 13's
own Rust (1.85) is too old; the package still only needs Debian 13's
libraries. Ubuntu 26.04 works with its own Rust (the second example above).

For development with a local toolchain: `cargo run`, `cargo test`.

## Command line

```
moriartty                     # terminal with your shell
moriartty -config             # settings window first
moriartty -load "big font"    # use a saved session
moriartty -geometry 120x40 -title build -e make -j8
moriartty -dir ~/src -e make  # start in a directory
moriartty -hold -e ./test.sh  # keep the window open after the program exits
moriartty -ls                 # login shell
moriartty -log ~/term.log     # log all output
```

`man moriartty` has the details; bash, zsh and fish complete the options
and saved session names.

## Documentation

* [CHANGELOG.md](CHANGELOG.md): what changed in each release
* [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md): what Moriartty protects
  against, and what it doesn't
* [docs/SETTINGS_FORMAT.md](docs/SETTINGS_FORMAT.md): the saved settings
  files, and the compatibility promise
* [docs/PERFORMANCE.md](docs/PERFORMANCE.md): the performance budget
* [CONTRIBUTING.md](CONTRIBUTING.md) and [docs/ROADMAP.md](docs/ROADMAP.md)

## Make Moriartty the default terminal

**Newer GNOME, KDE and others (`xdg-terminal-exec`):** put Moriartty's
desktop ID first in `~/.config/xdg-terminals.list`:

```sh
echo io.github.moriartty.Moriartty.desktop > ~/.config/xdg-terminals.list
```

**Debian / Ubuntu (`x-terminal-emulator`):** the `.deb` registers Moriartty
as an alternative. To select it:

```sh
sudo update-alternatives --config x-terminal-emulator
```

## Shortcuts

| Shortcut | Action |
|---|---|
| Ctrl+Shift+C, Ctrl+Insert | Copy |
| Ctrl+Shift+V, Shift+Insert | Paste |
| Ctrl+Shift+F | Find in scrollback. Enter goes to older matches, Ctrl+G to newer ones, Escape closes. |
| Ctrl+= / Ctrl+- / Ctrl+0 | Zoom in / out / back to normal |
| Ctrl+click | Open the web link under the pointer |
| Ctrl+Shift+T | New tab in the same directory |
| Ctrl+Shift+W | Close tab (asks first if a program is running) |
| Ctrl+PgUp / Ctrl+PgDn, Ctrl+Shift+Tab / Ctrl+Tab | Previous / next tab (with two or more tabs) |
| Ctrl+Shift+PgUp / Ctrl+Shift+PgDn | Move tab left / right |
| Alt+1 … Alt+9 | Go to tab 1–9 (with two or more tabs) |
| Ctrl+Shift+N | New window in the same directory |
| Ctrl+Shift+P | Settings |
| Alt+Enter | Full screen, if enabled on the Behaviour page |

## Security design

* **Paste:** escape and other control characters are stripped. The
  bracketed-paste terminator is always removed, so pasted text can't escape
  bracketed-paste mode. Pastes with several lines, or with invisible or
  direction-changing characters, ask for confirmation (Cancel is the
  default button); the preview spells those characters out and says how
  much is not shown. A paste
  is dropped if the shell was restarted in the meantime. Middle-click
  (PRIMARY selection) pastes go through the same filter.
* **What programs can do to the terminal:** no answerback, no title
  reporting, and no clipboard writes (OSC 52). Window titles set by programs
  have control characters stripped and their length capped. OSC 8 hyperlinks
  are off by default.
* **Links:** only `http://` and `https://` URLs can be opened, never `file:`
  or custom schemes, and never links with a user name before the host
  (`https://bank.example@evil.example/`), invisible or direction-changing
  characters, or a backslash in the host. Opening one always takes a
  Ctrl+click, and the tooltip shows the full address first. OSC 8 links,
  whose text the program chooses, and international host names, which can
  imitate other sites letter for letter, ask before opening and show the
  real address (the `xn--` host for international names).
* **Clipboard and logs:** selections stay out of the clipboard unless you
  turn that on, and Copy All asks before copying a large scrollback. A red
  dot on a tab shows that its output is being logged; hover it for the
  file.
* **Files:** the config directory is 0700 and session files are 0600, written
  atomically through an unpredictable temporary name. Symlinks and files
  owned by other users are refused. A Default Settings file that can't be
  read is moved aside to `Default Settings.toml.bad` and the window says
  so, instead of it being overwritten. Log files
  must have an absolute path, are 0600, are opened with `O_NOFOLLOW` and
  without blocking on FIFOs, and are only emptied after those checks pass.
* **Bad settings never lock you out:** if a saved shell, terminal type or
  log file becomes invalid, the terminal falls back to a safe value and says
  so, instead of refusing to start.
* **Running programs:** the shell and `-e` commands are started directly,
  never through an extra `sh -c`. A custom shell must be an absolute path to
  an executable. The pty is created close-on-exec from the start, and
  launcher variables such as `XDG_ACTIVATION_TOKEN` are not passed on. A
  program that ignores the hangup when its tab closes is killed two seconds
  later.
* **Process:** Moriartty is non-dumpable (`PR_SET_DUMPABLE=0`), so there are
  no core dumps of your scrollback and other programs running as you can't
  ptrace it. All `unsafe` code is in `src/sys.rs`. Release builds keep
  overflow checks on and abort on panic.

## Differences from PuTTY

Moriartty keeps what PuTTY users rely on: the settings tree, saved sessions,
pterm's command-line options and the mouse model. On purpose, it also:

* has **tabs**, named after the program and directory they run in;
* adds **find in scrollback**, **font zoom** and **Ctrl+click web links**;
* has no answerback string and never answers window-title queries (see
  [Security design](#security-design));
* can be the desktop's **default terminal**.

Not available:

* **Not exposed by VTE:** function-key and Home/End sequence variants,
  initial keypad and cursor-key modes, disabling mouse reporting or the
  alternate screen, and font quality.
* **Character sets:** the terminal is UTF-8 only.
* **Shift+click:** Shift+Right-click and Shift+Middle-click go to the
  terminal application, for mouse-aware programs like vim or htop.
