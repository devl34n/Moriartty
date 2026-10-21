#!/usr/bin/env bash
# Sign a published GitHub Release. Pushing a tag (vX.Y.Z) makes CI build the
# packages and publish them with an unsigned SHA256SUMS; this signs exactly
# those files, so the signature matches what people download.
#
# Usage:
#   ./sign-release.sh KEY [TAG]    # KEY: your GPG key's e-mail or fingerprint
#                                  # TAG: default v<version in Cargo.toml>
#
# Downloads the release's SHA256SUMS and packages into a temporary directory,
# checks the packages against it, signs it, and uploads SHA256SUMS.asc and
# signing-key.asc (your public key) to the same release. Files already on the
# release are never replaced. Needs gpg and the GitHub CLI (gh, logged in).
set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")"

KEY="${1:?usage: ./sign-release.sh KEY [TAG]   (KEY: your GPG key e-mail or fingerprint)}"
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)
TAG="${2:-v$VERSION}"

for tool in gpg gh sha256sum; do
    if ! command -v "$tool" >/dev/null; then
        echo "Need $tool (Fedora: sudo dnf install gnupg2 gh coreutils)." >&2
        exit 1
    fi
done
if ! gpg --list-secret-keys "$KEY" >/dev/null 2>&1; then
    echo "No secret GPG key for $KEY. Create one with: gpg --quick-gen-key \"$KEY\"" >&2
    exit 1
fi
# The repository this checkout belongs to (gh needs it named once we leave
# the checkout).
REPO=$(gh repo view --json nameWithOwner --jq .nameWithOwner)

if ! assets=$(gh release view "$TAG" --repo "$REPO" --json assets --jq '.assets[].name' 2>/dev/null); then
    echo "No GitHub release $TAG in $REPO (yet). Push the tag and wait for the Release workflow." >&2
    exit 1
fi
if grep -qx 'SHA256SUMS.asc' <<<"$assets"; then
    echo "$TAG is already signed (SHA256SUMS.asc is on the release); nothing to do." >&2
    exit 1
fi

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

echo "Downloading $TAG from $REPO..."
gh release download "$TAG" --repo "$REPO" --dir "$WORK" \
    --pattern 'SHA256SUMS' --pattern '*.rpm' --pattern '*.deb'
cd "$WORK"

echo "Checking the packages against SHA256SUMS:"
sha256sum -c SHA256SUMS

gpg --local-user "$KEY" --armor --detach-sign --output SHA256SUMS.asc SHA256SUMS
gpg --armor --export "$KEY" > signing-key.asc

# No --clobber: never replace what is already published.
gh release upload "$TAG" --repo "$REPO" SHA256SUMS.asc signing-key.asc

echo
echo "Signed and uploaded SHA256SUMS.asc and signing-key.asc to $TAG."
echo "Key fingerprint (publish it somewhere other than this repository, e.g. your"
echo "GitHub profile, so users can check signing-key.asc really is yours):"
gpg --fingerprint "$KEY" | sed -n '2p'
