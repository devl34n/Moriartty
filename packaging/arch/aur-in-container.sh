#!/usr/bin/env bash
# Runs INSIDE the Arch Linux container (see update-aur.sh).
# Expects packaging/arch in /build, output in /out.
set -euo pipefail

pacman -Syu --noconfirm --needed base-devel pacman-contrib >/dev/null

# makepkg and updpkgsums refuse to run as root.
useradd -m builder
WORK=/home/builder/aur
mkdir -p "$WORK"
cp /build/packaging/arch/PKGBUILD "$WORK/"
chown -R builder: "$WORK"
cd "$WORK"

# Downloads GitHub's archive of the tag and puts its checksum in the PKGBUILD.
runuser -u builder -- updpkgsums
runuser -u builder -- makepkg --printsrcinfo > .SRCINFO

cp PKGBUILD .SRCINFO /out/
echo "Checksum: $(sed -n "s/^sha256sums=//p" PKGBUILD)"
