#!/usr/bin/env bash
# Runs INSIDE the Arch Linux build container (see build-arch.sh).
# Expects the source in /build, VERSION in the environment, output in /out.
set -euo pipefail

: "${VERSION:?VERSION not set}"

# Everything the PKGBUILD lists, so this never drifts from it.
mapfile -t DEPS < <(
    # shellcheck source=/dev/null
    source /build/packaging/arch/PKGBUILD
    printf '%s\n' "${depends[@]}" "${makedepends[@]}"
)
pacman -Syu --noconfirm --needed base-devel "${DEPS[@]}" >/dev/null

echo "rustc: $(rustc --version)"
echo "cargo: $(cargo --version)"

# makepkg refuses to run as root.
useradd -m builder
WORK=/home/builder/pkg
mkdir -p "$WORK"
cp /build/packaging/arch/PKGBUILD "$WORK/"

# Pack the working tree in the layout of GitHub's archive of the tag, under
# the name the PKGBUILD's source line gives it: makepkg then uses this file
# instead of downloading, so the package is built from what was streamed in.
mkdir -p /tmp/src
cp -a /build "/tmp/src/Moriartty-$VERSION"
tar -C /tmp/src -czf "$WORK/moriartty-$VERSION.tar.gz" "Moriartty-$VERSION"
chown -R builder: "$WORK"

cd "$WORK"
runuser -u builder -- makepkg -f --skipchecksums --noconfirm

# The -debug- package is not useful to end users (like the RPM's debuginfo).
for f in "$WORK"/moriartty-*.pkg.tar.zst; do
    case "$f" in
        *-debug-*) ;;
        *) cp "$f" /out/ ;;
    esac
done
echo "Wrote:"
ls -1 /out/*.pkg.tar.zst
