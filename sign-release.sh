#!/usr/bin/env bash
# Publish the built packages: copy them into releases/, rewrite SHA256SUMS
# and sign it with your GPG key. The checksums alone only show a download
# isn't corrupted; the signature shows who built it.
#
# Usage:
#   ./build-deb.sh && ./build-rpm.sh      # build first (either or both)
#   ./sign-release.sh KEY                 # KEY: your key's e-mail or fingerprint
#
# Writes releases/SHA256SUMS, releases/SHA256SUMS.asc (the signature) and
# releases/signing-key.asc (your public key, for users to import).
set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")"

KEY="${1:?usage: ./sign-release.sh KEY (your GPG key e-mail or fingerprint)}"
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)

if ! command -v gpg >/dev/null; then
    echo "Need gpg (Debian/Ubuntu: sudo apt install gnupg, Fedora: sudo dnf install gnupg2)." >&2
    exit 1
fi
if ! gpg --list-secret-keys "$KEY" >/dev/null 2>&1; then
    echo "No secret GPG key for $KEY. Create one with: gpg --quick-gen-key \"$KEY\"" >&2
    exit 1
fi

shopt -s nullglob
pkgs=(dist/rpm/moriartty-"$VERSION"-*.x86_64.rpm dist/deb/moriartty_"$VERSION"-*_amd64.deb)
if (( ${#pkgs[@]} == 0 )); then
    echo "No $VERSION packages in dist/. Run ./build-deb.sh and/or ./build-rpm.sh first." >&2
    exit 1
fi

cp -t releases "${pkgs[@]}"
cd releases
files=(*.rpm *.deb)
sha256sum -- "${files[@]}" > SHA256SUMS
rm -f SHA256SUMS.asc
gpg --local-user "$KEY" --armor --detach-sign --output SHA256SUMS.asc SHA256SUMS
gpg --armor --export "$KEY" > signing-key.asc

echo "Signed SHA256SUMS for:"
printf '  %s\n' "${files[@]}"
echo
echo "Key fingerprint (publish it somewhere other than this repository, e.g. your"
echo "GitHub profile, so users can check signing-key.asc really is yours):"
gpg --fingerprint "$KEY" | sed -n '2p'
