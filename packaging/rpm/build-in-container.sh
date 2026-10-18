#!/usr/bin/env bash
# Runs INSIDE the Fedora build container (see build-rpm.sh).
# Expects the source in /build, VERSION in the environment, output in /out.
set -euo pipefail

: "${VERSION:?VERSION not set}"
NAME=moriartty

dnf install -y -q --setopt=install_weak_deps=False \
    rust cargo gcc rpm-build gtk4-devel vte291-gtk4-devel \
    desktop-file-utils ImageMagick tar gzip >/dev/null

echo "rustc: $(rustc --version)"
cd /build

# Pin dependency versions; hand the lock file back to the host.
[[ -f Cargo.lock ]] || cargo generate-lockfile
cp Cargo.lock /out/Cargo.lock

TOP=/tmp/rpmbuild
mkdir -p "$TOP"/{SOURCES,SPECS,BUILD,RPMS,SRPMS}
STAGE=$(mktemp -d)
SRC="$STAGE/$NAME-$VERSION"
mkdir -p "$SRC"
cp -a Cargo.toml Cargo.lock src packaging README.md LICENSE "$SRC/"
tar -C "$STAGE" -czf "$TOP/SOURCES/$NAME-$VERSION.tar.gz" "$NAME-$VERSION"

# Vendor crates (the only network step) so rpmbuild itself runs offline.
( cd "$SRC" && cargo vendor --locked vendor >/dev/null )
tar -C "$SRC" -czf "$TOP/SOURCES/$NAME-$VERSION-vendor.tar.gz" vendor

cp "packaging/$NAME.spec" "$TOP/SPECS/"
rpmbuild --define "_topdir $TOP" -ba "$TOP/SPECS/$NAME.spec"

find "$TOP/RPMS" "$TOP/SRPMS" -name '*.rpm' -exec cp -t /out {} +
echo "Wrote:"
ls -1 /out/*.rpm
