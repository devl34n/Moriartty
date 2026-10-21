#!/usr/bin/env bash
# Runs INSIDE the Debian/Ubuntu build container (see build-deb.sh).
# Expects the source in /build, VERSION in the environment, output in /out.
set -euo pipefail

: "${VERSION:?VERSION not set}"
NAME=moriartty
REVISION=1
export DEBIAN_FRONTEND=noninteractive

# Debian 13's own Rust (1.85) is too old for gtk-rs 0.22 (needs 1.92), so
# on trixie the compiler comes from trixie-backports. Only the build tools
# do: the package is still built against Debian 13's GTK and VTE, so it
# installs on Debian 13 and newer. Other images (e.g. Ubuntu 26.04) use
# their own Rust.
# Read in a subshell: /etc/os-release also sets NAME, which would replace
# this script's NAME=moriartty.
CODENAME=$(. /etc/os-release && echo "${VERSION_CODENAME:-}")
RUST_FROM=()
if [[ "$CODENAME" == trixie ]]; then
    echo "deb http://deb.debian.org/debian trixie-backports main" \
        > /etc/apt/sources.list.d/trixie-backports.list
    RUST_FROM=(-t trixie-backports)
fi

apt-get update -qq
apt-get install -y -qq --no-install-recommends \
    ca-certificates build-essential pkg-config dpkg-dev file \
    libgtk-4-dev libvte-2.91-gtk4-dev imagemagick >/dev/null
apt-get install -y -qq --no-install-recommends "${RUST_FROM[@]}" cargo rustc >/dev/null

echo "rustc: $(rustc --version)"
echo "cargo: $(cargo --version)"
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
install -Dm0644 packaging/io.github.moriartty.Moriartty.metainfo.xml \
    "$ROOT/usr/share/metainfo/io.github.moriartty.Moriartty.metainfo.xml"

MAGICK=convert
command -v magick >/dev/null && MAGICK=magick
# The logo is 3:2; pad it to a square instead of letting the icons come
# out non-square.
for size in 16 24 32 48 64 128 256 512; do
    dir="$ROOT/usr/share/icons/hicolor/${size}x${size}/apps"
    install -d "$dir"
    "$MAGICK" packaging/moriartty-logo.png -resize "${size}x${size}" \
        -background none -gravity center -extent "${size}x${size}" \
        "$dir/io.github.moriartty.Moriartty.png"
done
install -Dm0644 packaging/io.github.moriartty.Moriartty-symbolic.svg \
    "$ROOT/usr/share/icons/hicolor/symbolic/apps/io.github.moriartty.Moriartty-symbolic.svg"

install -d "$ROOT/usr/share/man/man1"
gzip -9n < packaging/moriartty.1 > "$ROOT/usr/share/man/man1/moriartty.1.gz"
chmod 0644 "$ROOT/usr/share/man/man1/moriartty.1.gz"
install -Dm0644 packaging/completions/moriartty.bash \
    "$ROOT/usr/share/bash-completion/completions/moriartty"
install -Dm0644 packaging/completions/_moriartty \
    "$ROOT/usr/share/zsh/vendor-completions/_moriartty"
install -Dm0644 packaging/completions/moriartty.fish \
    "$ROOT/usr/share/fish/vendor_completions.d/moriartty.fish"

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
Provides: x-terminal-emulator
Section: x11
Priority: optional
Description: private-by-default terminal inspired by PuTTY
 Moriartty is a fast, private-by-default terminal emulator inspired by
 PuTTY's pterm. It keeps pterm's configuration tree (Session, Logging, Terminal, Keyboard, Bell, Features,
 Window, Appearance, Behaviour, Selection, Colours), saved setting profiles,
 copy-on-select and right-click paste, with safe paste filtering.
EOF

# Register as an x-terminal-emulator alternative (Debian's default-terminal
# mechanism; -e and -T are supported as its policy requires). Priority 20 is
# low, so installing Moriartty never silently replaces the user's choice.
cat > "$ROOT/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = "configure" ]; then
    update-alternatives --install /usr/bin/x-terminal-emulator \
        x-terminal-emulator /usr/bin/moriartty 20
fi
EOF
cat > "$ROOT/DEBIAN/prerm" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = "remove" ] || [ "$1" = "deconfigure" ]; then
    update-alternatives --remove x-terminal-emulator /usr/bin/moriartty
fi
EOF
chmod 0755 "$ROOT/DEBIAN/postinst" "$ROOT/DEBIAN/prerm"

DEB="/out/${NAME}_${VERSION}-${REVISION}_${ARCH}.deb"
dpkg-deb --root-owner-group -Zxz --build "$ROOT" "$DEB"
echo "Depends: $DEPENDS"
echo "Wrote $DEB"
