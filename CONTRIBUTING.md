# Contributing

Thanks for helping. Moriartty is a small terminal with a strong focus on
safety and privacy; please read the guiding principles below and
[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) first.

Security problems: don't open an issue, see [SECURITY.md](SECURITY.md).

## Guiding principles

1. **Safe defaults, opt-in risk.** Anything that can leak data or run
   something needs an explicit setting, and the setting says so.
2. **No network, no telemetry, ever.** Opening a link stays the only outbound
   action.
3. **Lightweight.** No heavy new dependencies (no libadwaita, no async
   runtime), and a tab with no new output does no work.
4. **Fail safe, never lock out.** Bad settings fall back to safe values and
   say so, as they already do for the shell, terminal type and log file.
5. **PuTTY-inspired, not a PuTTY clone.** Keep what PuTTY users rely on: the
   settings tree, saved sessions, pterm's command-line options and the mouse
   model. Everything else is free to modernise.

## Building and testing

Everything builds in containers, so you only need podman (or docker):

```sh
./build-rpm.sh        # Fedora package, runs the tests
./build-deb.sh        # Debian 13 package, runs the tests
./build-arch.sh       # Arch Linux package, runs the tests
./update-aur.sh       # after a release: PKGBUILD + .SRCINFO for the AUR (dist/aur)
./format.sh           # format the code as CI expects
./check.sh            # formatting, clippy and tests, as CI runs them
./update-deps.sh lock # refresh Cargo.lock within Cargo.toml's versions
```

With a local toolchain (Rust ≥ 1.92, GTK ≥ 4.12 and VTE ≥ 0.76 development
files): `cargo run`, `cargo test`. `MORIARTTY_DEBUG=1 cargo run` prints
diagnostics. The evil bytes test needs a display; under CI it runs with
`xvfb-run`.

## Rules for changes

* **Safe defaults.** Anything that can leak data or run something is off or
  asks first, and its setting says it is risky.
* **No network access, no telemetry.**
* **`unsafe` only in `src/sys.rs`**, each block with a `SAFETY:` comment.
* **No heavy dependencies.** New crates need a good reason; `cargo deny`
  checks licences and advisories.
* **Settings format:** keys are never renamed, removed or reused (see
  [docs/SETTINGS_FORMAT.md](docs/SETTINGS_FORMAT.md)). New keys need a
  default; changes of meaning need a `config_version` bump and a migration.
* **Tests never touch the real home directory**; use
  `storage::test_support` for temporary directories.
* Run `./format.sh` and `./check.sh` before committing; CI checks formatting,
  builds on Fedora and Debian, runs clippy and the minimum Rust version.
* Add a line to [CHANGELOG.md](CHANGELOG.md) for anything a user would notice.

## Not planned

* **Flatpak.** A sandboxed terminal has to run your shell outside the sandbox
  (`flatpak-spawn --host`), which makes the sandbox misleading.
* aarch64 packages.
* SSH or serial connections: Moriartty stays a local terminal, like `pterm`.
* Split panes, libadwaita, single-instance mode.
