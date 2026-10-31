#!/usr/bin/env bash
# Build the Moriartty Arch Linux package inside an Arch container (podman or
# docker), from the working tree, so it works from any Linux distribution.
#
# Usage:
#   ./build-arch.sh                                # latest Arch Linux
#   ./build-arch.sh docker.io/library/archlinux:base-devel
#
# The source is streamed into the container as a tar archive (nothing from
# the project is mounted); only the output directory dist/arch is shared.
# Output: dist/arch/moriartty-<version>-1-x86_64.pkg.tar.zst
set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")"

IMAGE="${1:-docker.io/library/archlinux:latest}"
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)
PKG_VERSION=$(sed -n 's/^pkgver=//p' packaging/arch/PKGBUILD | head -n1)
OUT="$PWD/dist/arch"

if [[ "$VERSION" != "$PKG_VERSION" ]]; then
    echo "Version mismatch: Cargo.toml=$VERSION, PKGBUILD=$PKG_VERSION" >&2
    exit 1
fi

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
if [[ ! -f Cargo.lock ]]; then
    echo "Cargo.lock is missing; it must be committed to the repository." >&2
    exit 1
fi

mkdir -p "$OUT"
rm -f "$OUT"/*.pkg.tar.zst

echo "Building moriartty $VERSION Arch package in $IMAGE using $ENGINE..."
tar -c Cargo.toml Cargo.lock src tests packaging README.md LICENSE |
    "$ENGINE" run --rm -i \
        -e VERSION="$VERSION" \
        -v "$OUT":/out:Z \
        "$IMAGE" \
        bash -c 'set -e; mkdir /build; tar -x -C /build; exec bash /build/packaging/arch/build-in-container.sh'

echo
echo "Built:"
ls -1 "$OUT"/*.pkg.tar.zst
echo
echo "Install on Arch Linux with:  sudo pacman -U ./$(basename "$(ls -1t "$OUT"/*.pkg.tar.zst | head -n1)")"
