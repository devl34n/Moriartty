#!/usr/bin/env bash
# Build the Moriartty source RPM into OUTDIR. Used by both the local container
# build (packaging/rpm/build-in-container.sh) and Fedora COPR (.copr/Makefile).
#
# Usage: packaging/rpm/make-srpm.sh OUTDIR
# Needs: cargo, rpm-build, tar, gzip. Needs network access (cargo vendor).
set -euo pipefail

OUTDIR="$(readlink -f "${1:?usage: make-srpm.sh OUTDIR}")"
TOPSRC="$(readlink -f "$(dirname "$0")/../..")"
NAME=moriartty
cd "$TOPSRC"

VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)
SPEC_VERSION=$(sed -n 's/^Version:[[:space:]]*//p' packaging/$NAME.spec | head -n1)
if [[ "$VERSION" != "$SPEC_VERSION" ]]; then
    echo "Version mismatch: Cargo.toml=$VERSION, spec=$SPEC_VERSION" >&2
    exit 1
fi
if [[ ! -f Cargo.lock ]]; then
    echo "Cargo.lock is missing; it must be committed to the repository." >&2
    exit 1
fi

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
SRC="$WORK/$NAME-$VERSION"
mkdir -p "$SRC" "$WORK/SOURCES" "$OUTDIR"

cp -a Cargo.toml Cargo.lock src packaging README.md LICENSE "$SRC/"
tar -C "$WORK" -czf "$WORK/SOURCES/$NAME-$VERSION.tar.gz" "$NAME-$VERSION"

# Vendor crates, exactly as pinned in Cargo.lock, so the RPM build itself
# runs without network access.
( cd "$SRC" && cargo vendor --locked vendor >/dev/null )
tar -C "$SRC" -czf "$WORK/SOURCES/$NAME-$VERSION-vendor.tar.gz" vendor

rpmbuild -bs \
    --define "_sourcedir $WORK/SOURCES" \
    --define "_srcrpmdir $OUTDIR" \
    "packaging/$NAME.spec"
