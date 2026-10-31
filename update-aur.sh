#!/usr/bin/env bash
# Prepare the AUR package for a published release: the PKGBUILD with the
# checksum of GitHub's archive of the tag, and the .SRCINFO the AUR needs.
# Runs in an Arch container (podman or docker) and writes dist/aur/.
#
# Usage:
#   ./update-aur.sh      # after the tag v<version in Cargo.toml> is on GitHub
#
# It never pushes. To publish, copy dist/aur/PKGBUILD and dist/aur/.SRCINFO
# into your clone of ssh://aur@aur.archlinux.org/moriartty.git, then commit
# and push there. Needs network access (downloads the tagged source).
set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")"

IMAGE=docker.io/library/archlinux:latest
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)
PKG_VERSION=$(sed -n 's/^pkgver=//p' packaging/arch/PKGBUILD | head -n1)
OUT="$PWD/dist/aur"

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

mkdir -p "$OUT"
rm -f "$OUT/PKGBUILD" "$OUT/.SRCINFO"

echo "Preparing the AUR package for moriartty $VERSION (tag v$VERSION)..."
tar -c packaging/arch |
    "$ENGINE" run --rm -i \
        -v "$OUT":/out:Z \
        "$IMAGE" \
        bash -c 'set -e; mkdir /build; tar -x -C /build; exec bash /build/packaging/arch/aur-in-container.sh'

echo
echo "Wrote $OUT/PKGBUILD and $OUT/.SRCINFO. To publish:"
echo "  cp $OUT/PKGBUILD $OUT/.SRCINFO <your clone of ssh://aur@aur.archlinux.org/moriartty.git>/"
echo "  cd <that clone> && git add PKGBUILD .SRCINFO && git commit -m \"Update to $VERSION\" && git push"
