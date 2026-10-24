Name:           moriartty
Version:        0.9.5
Release:        1%{?dist}
Summary:        Private-by-default terminal for Linux, inspired by PuTTY
License:        MIT
Source0:        %{name}-%{version}.tar.gz
# Vendored crates, produced by build-rpm.sh (`cargo vendor`), so the build
# needs no network access.
Source1:        %{name}-%{version}-vendor.tar.gz

BuildRequires:  rust >= 1.92
BuildRequires:  cargo
BuildRequires:  gcc
BuildRequires:  pkgconfig(gtk4) >= 4.12
BuildRequires:  pkgconfig(vte-2.91-gtk4)
BuildRequires:  desktop-file-utils
BuildRequires:  appstream
# Scales packaging/moriartty-logo.png to the hicolor icon sizes.
BuildRequires:  ImageMagick

%if %{undefined build_rustflags}
%global debug_package %{nil}
%endif

%description
Moriartty is a fast, private-by-default terminal emulator inspired by
PuTTY's pterm. It keeps pterm's configuration tree (Session, Logging, Terminal, Keyboard, Bell, Features,
Window, Appearance, Behaviour, Selection, Colours), saved setting profiles,
copy-on-select and right-click paste, with safe paste filtering.

%prep
%autosetup -n %{name}-%{version}
tar -xzf %{SOURCE1}
mkdir -p .cargo
cat > .cargo/config.toml <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

%build
export RUSTFLAGS="%{?build_rustflags}"
cargo build --release --frozen --offline

%install
install -Dpm0755 target/release/moriartty %{buildroot}%{_bindir}/moriartty
install -Dpm0644 packaging/io.github.moriartty.Moriartty.desktop \
    %{buildroot}%{_datadir}/applications/io.github.moriartty.Moriartty.desktop
install -Dpm0644 packaging/io.github.moriartty.Moriartty.metainfo.xml \
    %{buildroot}%{_metainfodir}/io.github.moriartty.Moriartty.metainfo.xml
# The logo is 3:2; pad it to a square instead of letting the icons come
# out non-square.
for size in 16 24 32 48 64 128 256 512; do
    install -d %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps
    magick packaging/moriartty-logo.png -resize ${size}x${size} \
        -background none -gravity center -extent ${size}x${size} \
        %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/io.github.moriartty.Moriartty.png
done
install -Dpm0644 packaging/io.github.moriartty.Moriartty-symbolic.svg \
    %{buildroot}%{_datadir}/icons/hicolor/symbolic/apps/io.github.moriartty.Moriartty-symbolic.svg
install -Dpm0644 packaging/moriartty.1 %{buildroot}%{_mandir}/man1/moriartty.1
install -Dpm0644 packaging/completions/moriartty.bash \
    %{buildroot}%{_datadir}/bash-completion/completions/moriartty
install -Dpm0644 packaging/completions/_moriartty \
    %{buildroot}%{_datadir}/zsh/site-functions/_moriartty
install -Dpm0644 packaging/completions/moriartty.fish \
    %{buildroot}%{_datadir}/fish/vendor_completions.d/moriartty.fish
for sh in bash zsh fish; do
    install -Dpm0644 packaging/shell-integration/moriartty.$sh \
        %{buildroot}%{_datadir}/moriartty/shell-integration/moriartty.$sh
done

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/io.github.moriartty.Moriartty.desktop
appstreamcli validate --no-net %{buildroot}%{_metainfodir}/io.github.moriartty.Moriartty.metainfo.xml
export RUSTFLAGS="%{?build_rustflags}"
cargo test --release --frozen --offline

%files
%license LICENSE
%doc README.md
%{_bindir}/moriartty
%{_datadir}/applications/io.github.moriartty.Moriartty.desktop
%{_datadir}/icons/hicolor/*/apps/io.github.moriartty.Moriartty.png
%{_datadir}/icons/hicolor/symbolic/apps/io.github.moriartty.Moriartty-symbolic.svg
%{_metainfodir}/io.github.moriartty.Moriartty.metainfo.xml
%{_mandir}/man1/moriartty.1*
%{_datadir}/bash-completion/completions/moriartty
%{_datadir}/zsh/site-functions/_moriartty
%{_datadir}/fish/vendor_completions.d/moriartty.fish
%{_datadir}/moriartty/

%changelog
* Sat Oct 24 2026 Moriartty Developers - 0.9.5-1
- Saved sessions can run a command (ssh prod, toolbox enter, ...) instead of
  the shell, started directly, never through a shell
- A drop-down next to the + button opens a saved session in a new window
  (also Ctrl+Shift+S and the menu)
- Shell integration for bash, zsh and fish (opt-in scripts in
  /usr/share/moriartty/shell-integration): jump between prompts with
  Ctrl+Shift+Up/Down, and a notice when a long command ends in a background
  tab
- Every keyboard shortcut can be changed or turned off (Terminal > Keyboard)
- TERM_PROGRAM=moriartty is set for programs in the terminal
- 0.9.5 is a regular release, not a pre-release

* Wed Oct 21 2026 Moriartty Developers - 0.9.0-1
- Release candidate for 1.0 (includes the unreleased 0.7 and 0.8 work)
- Built with gtk-rs 0.22 (gtk4 0.11, vte4 0.10), nix 0.31 and toml 1;
  needs Rust 1.92 to build
- Idle tabs no longer wake up every second: a tab's name is re-read only
  after output, Enter or a directory change
- The link pattern is compiled once for all tabs
- Ctrl+click opens links with the desktop's browser directly (not through
  the portal, which refuses Moriartty); failures are shown
- Picking a colour scheme applies it at once; the Try button is gone
- Background opacity works with themes that paint widget backgrounds
- Man page and bash, zsh and fish completions
- Tests for settings and log files (symlinks, other owners, permissions,
  FIFOs), run in a temporary directory; tests no longer read the real
  ~/.config
- A headless test that hostile terminal output gets no answer containing
  attacker-chosen text and never touches the clipboard
- Performance budget (docs/PERFORMANCE.md, perf-check.sh)
- Threat model, settings format and changelog documents

* Tue Oct 20 2026 Moriartty Developers - 0.6.0-1
- New default colours, Moriartty Dark and Light, following the desktop's
  light or dark style; dialogs and menus follow it too
- The title bar takes the terminal's colours (not with high contrast)
- Tabs: underline on the current tab, close button shown on hover, drag to
  reorder, mouse-wheel scrolling when they don't fit
- 6px default gap between text and window edge
- Optional background opacity
- One settings window per terminal window, with a live preview of colours
  that Cancel undoes
- Square app icons and a symbolic icon
- Screen readers see the tabs as tabs
- Saved settings switch to following the desktop (saved colours are kept
  for when that is turned off); a border still at the old default moves
  to the new one
- Desktops without the portal (Cinnamon, MATE, XFCE) are followed through
  their GTK theme

* Tue Oct 20 2026 Moriartty Developers - 0.5.0-1
- Selected text goes to PRIMARY only; copying it to the clipboard too is an
  option, off by default. Right-click pastes the selection
- Ask before opening OSC 8 links and international domain names
- Refuse links with invisible or direction-changing characters, or a
  backslash in the host
- Paste preview also for single lines with hidden characters
- Copy All asks before copying a large scrollback
- A tab shows when its output is being logged
- A Default Settings file that can't be read is moved aside to .toml.bad
  and reported in the window, instead of being overwritten
- Settings files carry a format version
- The Event Log is capped and collapses repeated messages
- A program that ignores the hangup is killed when its tab closes
- Settings are saved through an unpredictable temporary name and the
  directory is synced

* Mon Oct 19 2026 Moriartty Developers - 0.4.4-1
- The × on a tab closes it again

* Mon Oct 19 2026 Moriartty Developers - 0.4.3-1
- Title bar about 22px high: smaller icons, window controls and tabs
- Empty title bar space no longer opens a tab (clashed with double-click);
  the + button next to the tabs does

* Mon Oct 19 2026 Moriartty Developers - 0.4.2-1
- One slim title bar with the tabs in it, always shown
- Click next to the tabs or on + for a new tab; middle-click closes a tab
- Tabs are named after the program's title, else "program: directory"
- Tab shortcuts work wherever the keyboard focus is; Ctrl+Tab switches tabs

* Mon Oct 19 2026 Moriartty Developers - 0.4.1-1
- Tabs: Ctrl+Shift+T / Ctrl+Shift+W, Ctrl+PgUp/PgDn, Ctrl+Shift+PgUp/PgDn, Alt+1..9
- A bad saved shell, terminal type or log file no longer stops Moriartty starting
- Paste confirmation defaults to Cancel and shows hidden characters
- Pastes are dropped if the shell was restarted in the meantime
- Links with a user name before the host are refused
- Link tooltips show the real (xn--) host of international domain names
- The Event Log window updates live; VTE_VERSION is set for the shell
- Log files: no blocking on FIFOs, emptied only after the safety checks
- Pty created close-on-exec; launcher variables not passed to the shell

* Sun Oct 18 2026 Moriartty Developers - 0.3.0-1
- Usable as the desktop's default terminal (xdg-terminal-exec keys, -dir, -hold)
- AppStream metadata, so it appears in GNOME Software / KDE Discover
- Font zoom is remembered and can be saved as default
- Option to hide the title bar buttons
- Warn before closing a window running an -e program
- Resize the pty on layout changes instead of polling

* Sun Oct 18 2026 Moriartty Developers - 0.2.0-1
- Settings dialog can save changes as Default Settings
- Font zoom (Ctrl+= / Ctrl+- / Ctrl+0)
- Find in scrollback (Ctrl+Shift+F)
- Ctrl+click opens http(s) links
- Colour scheme presets

* Sun Oct 18 2026 Moriartty Developers - 0.1.0-1
- Initial package
