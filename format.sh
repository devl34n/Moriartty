#!/usr/bin/env bash
# Format the Rust code with rustfmt (what CI's "cargo fmt --check" expects),
# inside a Fedora container like the build scripts, so no Rust toolchain is
# needed here. Rewrites src/ and tests/ in place: review with "git diff".
#
# Usage: ./format.sh [IMAGE]
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

OUT=$(mktemp)
trap 'rm -f "$OUT"' EXIT

# Sources go in on stdin, the formatted ones come back on stdout as a tar
# archive; nothing from the project is mounted. Edition 2021, as in
# Cargo.toml, and no rustfmt.toml: rustfmt's defaults.
echo "Formatting with rustfmt in $IMAGE using $ENGINE..."
tar -c src tests |
    "$ENGINE" run --rm -i "$IMAGE" bash -c '
        set -euo pipefail
        dnf install -y -q --setopt=install_weak_deps=False rustfmt tar >/dev/null 2>&1
        mkdir /build
        tar -x -C /build
        cd /build
        rustfmt --version >&2
        find src tests -name "*.rs" -print0 | xargs -0 rustfmt --edition 2021 >&2
        tar -c src tests' > "$OUT"

# Only unpacked once the container succeeded, so a failure changes nothing.
tar -x -f "$OUT"
echo "Done. Review the changes with: git diff --stat"
