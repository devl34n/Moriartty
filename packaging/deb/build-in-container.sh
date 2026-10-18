#!/usr/bin/env bash
# Runs INSIDE the Debian/Ubuntu build container (see build-deb.sh).
# Expects the source in /build, VERSION in the environment, output in /out.
set -euo pipefail

: "${VERSION:?VERSION not set}"
NAME=moriartty
REVISION=1
export DEBIAN_FRONTEND=noninteractive

apt-get update -qq
apt-get install -y -qq --no-install-recommends \
    ca-certificates build-essential pkg-config dpkg-dev file \
    cargo rustc libgtk-4-dev libvte-2.91-gtk4-dev imagemagick >/dev/null

echo "rustc: $(rustc --version)"
cd /build

# Pin dependency versions; hand the lock file back to the host.
[[ -f Cargo.lock ]] || cargo generate-lockfile
cp Cargo.lock /out/Cargo.lock

# Hardening: full RELRO + immediate binding (Debian's usual linker flags).
export RUSTFLAGS="-C link-arg=-Wl,-z,relro -C link-arg=-Wl,-z,now"
cargo build --release --locked
cargo test --release --locked

ARCH=$(dpkg --print-architecture)
ROOT=/tmp/pkgroot
rm -rf "$ROOT"
install -Dm0755 -s target/release/$NAME "$ROOT/usr/bin/$NAME"
install -Dm0644 packaging/io.github.moriartty.Moriartty.desktop \
    "$ROOT/usr/share/applications/io.github.moriartty.Moriartty.desktop"

MAGICK=convert
command -v magick >/dev/null && MAGICK=magick
for size in 16 24 32 48 64 128 256 512; do
    dir="$ROOT/usr/share/icons/hicolor/${size}x${size}/apps"
    install -d "$dir"
    "$MAGICK" packaging/moriartty-logo.png -resize "${size}x${size}" \
        "$dir/io.github.moriartty.Moriartty.png"
done

DOC="$ROOT/usr/share/doc/$NAME"
install -d "$DOC"
install -m0644 README.md "$DOC/README.md"
{
    echo "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/"
    echo "Upstream-Name: $NAME"
    echo
    echo "Files: *"
    echo "Copyright: 2026 Moriartty contributors"
    echo "License: MIT"
    sed 's/^$/./; s/^/ /' LICENSE
} > "$DOC/copyright"
chmod 0644 "$DOC/copyright"

# Work out library dependencies from the binary itself.
DEPENDS="libc6, libgtk-4-1 (>= 4.12), libvte-2.91-gtk4-0 (>= 0.76)"
SHL=/tmp/shlibs
mkdir -p "$SHL/debian"
printf 'Source: %s\n\nPackage: %s\nArchitecture: any\n' "$NAME" "$NAME" > "$SHL/debian/control"
if out=$(cd "$SHL" && dpkg-shlibdeps -O "$ROOT/usr/bin/$NAME" 2>/dev/null); then
    DEPENDS=$(printf '%s\n' "$out" | sed -n 's/^shlibs:Depends=//p')
fi

SIZE=$(du -sk "$ROOT" | cut -f1)
install -d "$ROOT/DEBIAN"
cat > "$ROOT/DEBIAN/control" <<EOF
Package: $NAME
Version: $VERSION-$REVISION
Architecture: $ARCH
Maintainer: Moriartty Developers <moriartty@localhost>
Installed-Size: $SIZE
Depends: $DEPENDS
Section: x11
Priority: optional
Description: PuTTY-style terminal emulator
 Moriartty is a terminal emulator modelled on PuTTY's pterm: the same
 configuration tree (Session, Logging, Terminal, Keyboard, Bell, Features,
 Window, Appearance, Behaviour, Selection, Colours), saved setting profiles,
 copy-on-select and right-click paste, with safe paste filtering.
EOF

DEB="/out/${NAME}_${VERSION}-${REVISION}_${ARCH}.deb"
dpkg-deb --root-owner-group -Zxz --build "$ROOT" "$DEB"
echo "Depends: $DEPENDS"
echo "Wrote $DEB"
