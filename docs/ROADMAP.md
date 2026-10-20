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
- [ ] New screenshots (the current one predates tabs).
- [x] Settings: one window per terminal window, live preview of the
      colours that Cancel undoes.
- [ ] Accessibility: high contrast respected (the title bar keeps the
      theme's colours) and tabs exposed to screen readers as tabs. The
      settings tree already works with the keyboard. Still to do: keyboard
      focus on the tabs themselves.

## 0.7: Lightweight and robust

- [ ] Replace the once-a-second `/proc` polling per tab with OSC 7 and VTE
      signals, falling back to polling only the focused tab. Goal: no idle
      wakeups.
- [ ] Compile the URL regex once and share it between tabs.
- [ ] Performance budget, checked before each release and kept in the repo:
  - time to first prompt under 150 ms on a reference machine
  - memory per additional tab
  - stripped binary size
- [ ] Move to the current gtk-rs / vte4 / nix releases and drop duplicate
      crate versions.
- [ ] Don't hold the settings borrow across VTE calls in `apply_appearance`.

## 0.8: Assurance and supply chain

Testing
- [ ] Tests for `storage.rs` (symlink refusal, foreign owner, permission
      repair) and log file opening, run against a temporary
      `XDG_CONFIG_HOME`. No test touches the real home directory.
- [ ] `cargo-fuzz` targets for paste sanitising, the log stripper, URL
      checks and punycode, and settings loading, with a short fuzz run in CI.
- [ ] An "evil bytes" corpus (OSC 52, title reports, DECRQSS, answerback,
      bracketed-paste escapes) fed into VTE in a headless test, so
      protections that today rely on VTE not implementing a feature stay
      locked in.

CI
- [ ] `cargo fmt --check`, a wider clippy lint set, an MSRV (1.80) job.
- [ ] Pin `cargo-deny` and the container images (by digest) in CI and the
      build scripts.
- [ ] Dependabot for GitHub Actions, Cargo and containers.

Releases
- [ ] Build provenance (`actions/attest-build-provenance`) and an SBOM
      (CycloneDX) for every release.
- [ ] The maintainer's GPG signature of `SHA256SUMS` is attached to the
      GitHub Release, so there is one signed set of checksums.
- [ ] Published release assets are never replaced (no `--clobber`).
- [ ] Reproducible builds: `SOURCE_DATE_EPOCH`, `--remap-path-prefix`,
      normalised archive timestamps, and a CI job that builds twice and
      compares.
- [ ] Stop committing packages to `releases/`; GitHub Releases and COPR are
      the channels.

## 0.9: Release candidate

- [ ] `docs/THREAT_MODEL.md`: what Moriartty defends against (malicious
      output, malicious pastes, lookalike links, local file attacks) and what
      it does not (a compromised account, the shell itself).
- [ ] `CHANGELOG.md` and `CONTRIBUTING.md`.
- [ ] Man page and bash/zsh/fish completions in both packages.
- [ ] Security review of everything since 0.5, and a manual run of the evil
      bytes corpus on Fedora, Debian 13 and Ubuntu 25.04+.
- [ ] Freeze the settings format: version 1 documented, forward
      compatibility promised.

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
