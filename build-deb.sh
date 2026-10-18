#!/usr/bin/env bash
# Build a Moriartty .deb inside a Debian/Ubuntu container (podman or docker),
# so the binary links against that distribution's libraries.
#
# Usage:
#   ./build-deb.sh                                   # Debian 13 (trixie)
#   ./build-deb.sh docker.io/library/ubuntu:25.04    # another target
#
# Requirements on the target: GTK >= 4.12, VTE (GTK 4) >= 0.76, Rust >= 1.80.
# That means Debian 13+, or Ubuntu 25.04+. Debian 12 and Ubuntu 24.04 are too old.
#
# The source is streamed into the container as a tar archive (nothing from
# the project is mounted); only the output directory dist/deb is shared.
set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")"

IMAGE="${1:-docker.io/library/debian:trixie}"
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)
OUT="$PWD/dist/deb"

if command -v podman >/dev/null; then
    ENGINE=podman
elif command -v docker >/dev/null; then
    ENGINE=docker
else
    echo "Need podman or docker (Fedora: sudo dnf install podman, Debian/Ubuntu: sudo apt install podman)." >&2
    exit 1
fi

if [[ ! -f packaging/moriartty-logo.png ]]; then
    echo "Missing packaging/moriartty-logo.png (the application logo)" >&2
    exit 1
fi

mkdir -p "$OUT"
rm -f "$OUT"/Cargo.lock

files=(Cargo.toml src packaging README.md LICENSE)
[[ -f Cargo.lock ]] && files+=(Cargo.lock)

echo "Building moriartty $VERSION .deb in $IMAGE using $ENGINE..."
tar -c "${files[@]}" |
    "$ENGINE" run --rm -i \
        -e VERSION="$VERSION" \
        -v "$OUT":/out:Z \
        "$IMAGE" \
        bash -c 'set -e; mkdir /build; tar -x -C /build; exec bash /build/packaging/deb/build-in-container.sh'

# Keep the lock file the container generated, if the project had none.
if [[ -f "$OUT/Cargo.lock" ]]; then
    [[ -f Cargo.lock ]] || mv "$OUT/Cargo.lock" Cargo.lock
    rm -f "$OUT/Cargo.lock"
fi

echo
echo "Built:"
ls -1 "$OUT"/*.deb
echo
echo "Install on Debian/Ubuntu with:  sudo apt install ./$(basename "$(ls -1t "$OUT"/*.deb | head -n1)")"
