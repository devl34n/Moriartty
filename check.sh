#!/usr/bin/env bash
# Run the checks CI runs on Fedora (formatting, clippy, the tests with a
# virtual display) inside a Fedora container like the build scripts, so no
# Rust toolchain is needed here. Nothing in the working tree changes.
#
# Debug mode, so repeat runs are quick. CI also builds in release mode and on
# Debian, checks the minimum Rust version and runs cargo deny.
#
# The first run takes a few minutes. Downloads and build output stay in the
# volume "moriartty-check" (to remove it: podman volume rm moriartty-check).
#
# Usage: ./check.sh [IMAGE]
set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")"

IMAGE="${1:-registry.fedoraproject.org/fedora:latest}"

if command -v podman >/dev/null; then
    ENGINE=podman
elif command -v docker >/dev/null; then
    ENGINE=docker
else
    echo "Need podman or docker (Fedora: sudo dnf install podman, Debian/Ubuntu: sudo apt install podman)." >&2
    exit 1
fi

# Runs inside the container. A formatting difference doesn't stop the rest, so
# one run shows everything; a compile error does, as the tests would only
# repeat it.
INNER='
set -euo pipefail
dnf install -y -q --setopt=install_weak_deps=False --setopt=keepcache=True --setopt=cachedir=/cache/dnf \
    rust cargo clippy rustfmt gcc gtk4-devel vte291-gtk4-devel tar /usr/bin/xvfb-run /usr/bin/xauth >/dev/null
mkdir /build
tar -x -C /build
cd /build
export CARGO_HOME=/cache/cargo CARGO_TARGET_DIR=/cache/target
rustc --version

failed=0
echo "== Formatting"
if ! find src tests -name "*.rs" -print0 | xargs -0 rustfmt --check --edition 2021; then
    echo "Formatting differs: ./format.sh fixes it."
    failed=1
fi

echo "== Clippy"
cargo clippy --locked --all-targets -- \
    -D clippy::correctness -D clippy::suspicious \
    -W clippy::perf -W clippy::complexity

echo "== Tests"
MORIARTTY_REQUIRE_DISPLAY=1 GSK_RENDERER=cairo xvfb-run -a cargo test --locked

exit "$failed"
'

# The source goes in on stdin; nothing from the project is mounted.
echo "Checking in $IMAGE using $ENGINE..."
tar -c Cargo.toml Cargo.lock README.md src tests |
    "$ENGINE" run --rm -i \
        -v moriartty-check:/cache:Z \
        "$IMAGE" \
        bash -c "$INNER"

echo
echo "All checks passed."
