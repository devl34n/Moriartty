# Roadmap to 1.0

Moriartty 1.0 should be a terminal you can trust by default: nothing leaves
the machine, nothing risky happens without asking, and it stays small, fast
and pleasant to look at.

## Guiding principles

1. **Safe defaults, opt-in risk.** Anything that can leak data or run
   something needs an explicit setting, and the setting says so.
2. **No network, no telemetry, ever.** Opening a link stays the only outbound
   action.
3. **Lightweight.** No heavy new dependencies (no libadwaita, no async
   runtime). Startup and idle cost are measured, not guessed.
4. **Fail safe, never lock out.** Bad settings fall back to safe values and
   say so, as they already do for the shell, terminal type and log file.
5. **PuTTY-inspired, not a PuTTY clone.** Keep what PuTTY users rely on: the
   settings tree, saved sessions, pterm's command-line options and the mouse
   model. Everything else is free to modernise.

## 0.5: Security and privacy defaults ✅ (0.5.0)

Links and display
- [x] Reject URLs containing invisible or direction-changing characters
      (U+200B, U+202E, U+2066 …) and backslashes in the host part
      (`matching.rs`, `safe_url`).
- [x] ~~Spell out invisible characters in link tooltips~~: such links are
      now refused outright, so tooltips never show them.
- [x] Ask before opening international (`xn--`) hosts and OSC 8 links.
      VTE doesn't expose an OSC 8 link's visible text, so every OSC 8 link
      asks.
- [x] Strip control characters from local titles too (`-title`, `wintitle`).

Clipboard and paste
- [x] **Selections fill PRIMARY only.** New option "Also copy selected text
      to the clipboard", off by default, so clipboard managers don't capture
      selected secrets. Right-click (Compromise mode) pastes PRIMARY, so
      select-then-right-click still works.
- [x] Show the paste preview for single-line pastes that contain invisible
      or direction-changing characters.
- [x] Ask before "Copy All" on scrollbacks over 1000 lines.

Visibility
- [x] Show an indicator on a tab while its output is being logged.
- [x] Cap the in-memory event log (1000 lines) and collapse repeated events.

Process and files
- [x] After SIGHUP, send SIGKILL after a timeout so no reaper thread can
      hang (`pty.rs`).
- [x] Check that `$SHELL` is executable before using it.
- [x] Atomic writes: random temporary file name, fsync of the directory
      after rename (`storage.rs`).
- [x] Add a `config_version` field and a migration hook.
- [x] A Default Settings file that fails to parse is moved aside to
      `Default Settings.toml.bad` and reported in the window, never
      silently overwritten. (Named sessions are only written by an explicit
      Save, so they are left in place and the error is shown.)

## 0.6: Modern look (plain GTK 4) (0.6.0)

- [x] Follow the desktop's light/dark preference (freedesktop
      `color-scheme` setting, read through the Settings portal).
- [x] New default: **Moriartty Dark / Light**, a palette pair that switches
      with the desktop ("Follow the desktop's light or dark style").
      PuTTY classic, Tango, Solarized and Dracula stay. On by default, also
      for saved settings (their colours are kept for when it is turned
      off). GNOME's light style reads as "no preference" and counts as
      light; desktops without the portal are followed through their GTK
      theme.
- [x] Title bar colours derived from the terminal palette, so chrome and
      terminal read as one surface.
- [x] Tabs: clear active indicator, close button on hover, drag to reorder,
      mouse-wheel scrolling when they overflow; the current tab is
      scrolled into view.
- [x] Default inner padding 6 px (range unchanged).
- [x] Optional background opacity, off by default.
- [ ] App icon: icons are now square (the logo is padded instead of
      stretched) and there is a symbolic icon. Still to do: a full-colour
      **SVG** of the logo, which needs the original artwork.
- [x] New screenshots (0.9.0), with a neutral `demo@moriartty` prompt.
- [x] Settings: one window per terminal window, live preview of the
      colours that Cancel undoes.
- [ ] Accessibility: high contrast respected (the title bar keeps the
      theme's colours) and tabs exposed to screen readers as tabs. The
      settings tree already works with the keyboard. Still to do: keyboard
      focus on the tabs themselves.

## 0.7: Lightweight and robust ✅ (in 0.9.0)

- [x] No more once-a-second `/proc` polling. A tab's name is re-read once,
      shortly after output, Enter or an OSC 7 directory report; idle tabs
      never wake up (better than the planned "poll only the focused tab").
      OSC 7 is only a cue: the directory is always read from the shell.
- [x] Compile the URL regex once and share it between tabs.
- [x] Performance budget in `docs/PERFORMANCE.md`, measured by
      `perf-check.sh`: time to first output (< 150 ms), idle wakeups, memory
      per additional tab, stripped binary size. Still to do: record the
      0.9.0 numbers.
- [x] Moved to gtk4 0.11 / vte4 0.10 (gtk-rs 0.22), nix 0.31 and toml 1; the
      duplicate toml 0.8 crates are gone. (gtk-rs dropped
      `glib::unix_fd_add_local`; `sys.rs` now calls `g_unix_fd_add_full`.)
      Building needs Rust 1.92: Debian 13 gets it from trixie-backports.
- [x] Don't hold the settings borrow across VTE calls in `apply_appearance`.

## 0.8: Assurance and supply chain (in 0.9.0)

Testing
- [x] Tests for `storage.rs` (symlink refusal, foreign owner, permission
      repair, size limit, quarantine of unreadable files) and log file
      opening (symlinks, devices, FIFOs, append/overwrite). Under `cargo
      test` the config directory is a private temporary directory, so no
      test touches the real home directory (before, the `cli` tests did).
- [ ] `cargo-fuzz` targets. Moved to 1.0 preparation: the code is a single
      binary crate, so fuzzing first needs a library/binary split, and the
      fuzz run needs a nightly toolchain in CI.
- [x] "Evil bytes" corpus fed into VTE (`tests/evil_bytes.rs`); CI runs it
      under `xvfb-run` on Fedora.

CI
- [x] Formatting check (`./format.sh` fixes it), wider clippy lints
      (performance and complexity reported), a job building with the
      `rust-version` from `Cargo.toml` (now 1.92).
- [ ] Pin `cargo-deny` and the container images by digest: needs the exact
      versions and digests to be chosen by the maintainer.
- [x] Dependabot for GitHub Actions and Cargo (the container images are
      named in scripts, not Dockerfiles, which Dependabot can't update).

Releases
- [ ] Build provenance and an SBOM: needs the attestation actions pinned to
      reviewed commit SHAs.
- [x] The maintainer's GPG signature attached to the GitHub Release:
      `sign-release.sh` downloads what CI published, checks it, signs
      `SHA256SUMS` and uploads the signature (0.x tags are pre-releases).
- [x] Published release assets are never replaced (no `--clobber`).
- [ ] Reproducible builds and a build-twice CI job.
- [ ] Stop committing packages to `releases/` (`git rm` by the maintainer).

## 0.9: Release candidate (0.9.0)

- [x] `docs/THREAT_MODEL.md`.
- [x] `CHANGELOG.md` and `CONTRIBUTING.md`.
- [x] Man page and bash/zsh/fish completions in both packages.
- [ ] Security review of everything since 0.5 (code review done while
      preparing 0.9: settings saved by newer versions no longer lose their
      format version), and a manual run of the evil bytes corpus on Fedora,
      Debian 13 and Ubuntu 26.04: maintainer testing.
- [x] Settings format frozen at version 2 and documented in
      `docs/SETTINGS_FORMAT.md`, with forward compatibility promised.

Fixed during 0.9 testing
- [x] Ctrl+click did not open links: GTK's launcher can go through the
      OpenURI portal, which refuses non-dumpable processes, and failed
      silently. Links now go to the default browser through GIO, and
      failures are reported.
- [x] A colour scheme was only kept after pressing Try. Picking one now
      applies it at once; Try is gone.
- [x] Background opacity showed a darker colour instead of the desktop with
      themes that paint stacks or boxes. Moriartty's widgets between the
      window and the terminal are now kept clear.

Known, left as is
- GTK 4.20 warns that `gtk-application-prefer-dark-theme` (used to make
  dialogs follow the desktop) is replaced by `gtk-interface-color-scheme`.
  The old property still works; switching needs GTK 4.20, newer than
  Debian 13's.

## 1.0: Release criteria

- No open security issues; fuzzers have run 24 h without findings.
- Every privacy-sensitive feature is off or asks first by default: clipboard
  on select, OSC 8 hyperlinks, logging, risky pastes, international links.
- Releases can be rebuilt and verified by anyone (reproducible build,
  provenance, GPG signature).
- The performance budget is met and published.
- `SECURITY.md` states that 1.x receives security fixes, with a response
  time.

## Not planned for 1.0

- **Flatpak.** A sandboxed terminal has to run your shell outside the sandbox
  (`flatpak-spawn --host`), which makes the sandbox misleading. To be
  revisited after 1.0.
- aarch64 packages (after 1.0).
- SSH or serial connections: Moriartty stays a local terminal, like `pterm`.
- Split panes, libadwaita, single-instance mode.
