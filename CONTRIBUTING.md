# Contributing

Thanks for helping. Moriartty is a small terminal with a strong focus on
safety and privacy; please read [docs/ROADMAP.md](docs/ROADMAP.md) (its
guiding principles) and [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) first.

Security problems: don't open an issue, see [SECURITY.md](SECURITY.md).

## Building and testing

Everything builds in containers, so you only need podman (or docker):

```sh
./build-rpm.sh        # Fedora package, runs the tests
./build-deb.sh        # Debian 13 package, runs the tests
./format.sh           # format the code as CI expects
./update-deps.sh lock # refresh Cargo.lock within Cargo.toml's versions
./perf-check.sh       # performance budget (in a desktop session)
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
* Run `./format.sh` before committing; CI checks formatting, builds on Fedora
  and Debian, runs clippy and the minimum Rust version.
* Add a line to [CHANGELOG.md](CHANGELOG.md) for anything a user would notice.
