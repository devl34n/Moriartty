#!/usr/bin/env bash
# Dependency helper. Runs cargo inside a Fedora container (podman or docker),
# like the build scripts, so no Rust toolchain is needed on this machine.
#
# Usage:
#   ./update-deps.sh probe   # newest gtk4 / vte4 / nix / toml: which versions
#                            # exist, how they fit together, and their sources
#   ./update-deps.sh lock    # rewrite Cargo.lock for the current Cargo.toml
#                            # (newest versions it allows) and vendor them
#
# Output goes to dist/deps/ (not committed):
#   report.txt   versions, duplicate crates, rustc version
#   vendor/      the crates' source code, for checking APIs before a build
#   Cargo.lock   (lock mode) also copied over ./Cargo.lock; review with git diff
#
# Needs network access (crates.io). The source is streamed into the
# container as a tar archive; only dist/deps is shared.
set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")"

MODE="${1:-}"
IMAGE="${2:-registry.fedoraproject.org/fedora:latest}"
OUT="$PWD/dist/deps"

case "$MODE" in
    probe|lock) ;;
    *)
        echo "usage: ./update-deps.sh probe|lock [IMAGE]" >&2
        exit 1
        ;;
esac

if command -v podman >/dev/null; then
    ENGINE=podman
elif command -v docker >/dev/null; then
    ENGINE=docker
else
    echo "Need podman or docker (Fedora: sudo dnf install podman, Debian/Ubuntu: sudo apt install podman)." >&2
    exit 1
fi

rm -rf "$OUT"
mkdir -p "$OUT"

files=(Cargo.toml src tests)
[[ -f Cargo.lock ]] && files+=(Cargo.lock)

# Runs inside the container. No GTK/VTE development packages are needed:
# nothing is compiled, cargo only resolves, lists and downloads.
INNER='
set -euo pipefail
dnf install -y -q --setopt=install_weak_deps=False cargo rust tar >/dev/null
mkdir /build
tar -x -C /build

report() { tee -a /out/report.txt; }
echo "$(rustc --version)" | report
echo "$(cargo --version)" | report
echo | report

if [[ "$MODE" == probe ]]; then
    cd /tmp
    cargo new --quiet --vcs none --bin probe
    cd probe
    # Newest release of each, chosen independently: if vte4 needs an older
    # gtk4 than the newest, the duplicates list below shows it.
    cargo add --quiet gtk4 vte4 nix toml
    cargo generate-lockfile --quiet
    { echo "== Newest versions (direct dependencies)"; cargo tree --depth 1; echo;
      echo "== Duplicate crates"; cargo tree -d || true; } | report
    cp Cargo.toml /out/probe-Cargo.toml
    cp Cargo.lock /out/probe-Cargo.lock
    cargo vendor --quiet --locked /out/vendor >/dev/null
else
    cd /build
    cargo generate-lockfile --quiet
    { echo "== Direct dependencies"; cargo tree --depth 1; echo;
      echo "== Duplicate crates"; cargo tree -d || true; } | report
    cp Cargo.lock /out/Cargo.lock
    cargo vendor --quiet --locked /out/vendor >/dev/null
fi
echo "Done." | report
'

echo "Running cargo ($MODE) in $IMAGE using $ENGINE..."
tar -c "${files[@]}" |
    "$ENGINE" run --rm -i \
        -e MODE="$MODE" \
        -v "$OUT":/out:Z \
        "$IMAGE" \
        bash -c "$INNER"

if [[ "$MODE" == lock ]]; then
    cp "$OUT/Cargo.lock" Cargo.lock
    echo
    echo "Cargo.lock updated. Review it with:  git diff --stat Cargo.lock"
fi

echo
echo "Report: $OUT/report.txt"
echo "Sources: $OUT/vendor/"
