#!/usr/bin/env bash
# Build the Moriartty RPM (and SRPM) inside a Fedora container (podman or
# docker), so it works from any Linux distribution and links against
# Fedora's libraries.
#
# Usage:
#   ./build-rpm.sh                                              # latest Fedora
#   ./build-rpm.sh registry.fedoraproject.org/fedora:42         # a specific release
#
# The source is streamed into the container as a tar archive (nothing from
# the project is mounted); only the output directory dist/rpm is shared.
# Output: dist/rpm/*.rpm
set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")"

IMAGE="${1:-registry.fedoraproject.org/fedora:latest}"
NAME=moriartty
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)
SPEC_VERSION=$(sed -n 's/^Version:[[:space:]]*//p' packaging/$NAME.spec | head -n1)
OUT="$PWD/dist/rpm"

if [[ "$VERSION" != "$SPEC_VERSION" ]]; then
    echo "Version mismatch: Cargo.toml=$VERSION, spec=$SPEC_VERSION" >&2
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

mkdir -p "$OUT"
rm -f "$OUT"/Cargo.lock

files=(Cargo.toml src packaging README.md LICENSE)
[[ -f Cargo.lock ]] && files+=(Cargo.lock)

echo "Building $NAME $VERSION RPM in $IMAGE using $ENGINE..."
tar -c "${files[@]}" |
    "$ENGINE" run --rm -i \
        -e VERSION="$VERSION" \
        -v "$OUT":/out:Z \
        "$IMAGE" \
        bash -c 'set -e; mkdir /build; tar -x -C /build; exec bash /build/packaging/rpm/build-in-container.sh'

# Keep the lock file the container generated, if the project had none.
if [[ -f "$OUT/Cargo.lock" ]]; then
    [[ -f Cargo.lock ]] || mv "$OUT/Cargo.lock" Cargo.lock
    rm -f "$OUT/Cargo.lock"
fi

echo
echo "Built:"
ls -1 "$OUT"/*.rpm
echo
echo "Install on Fedora with:  sudo dnf install $OUT/$NAME-$VERSION-*.$(uname -m).rpm"
