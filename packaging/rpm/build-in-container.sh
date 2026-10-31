#!/usr/bin/env bash
# Runs INSIDE the Fedora build container (see build-rpm.sh).
# Expects the source in /build, VERSION in the environment, output in /out.
set -euo pipefail

: "${VERSION:?VERSION not set}"

dnf install -y -q --setopt=install_weak_deps=False \
    rust cargo gcc rpm-build gtk4-devel vte291-gtk4-devel \
    desktop-file-utils appstream ImageMagick tar gzip >/dev/null

echo "rustc: $(rustc --version)"
cd /build

# Pin dependency versions; hand the lock file back to the host.
[[ -f Cargo.lock ]] || cargo generate-lockfile
cp Cargo.lock /out/Cargo.lock

# Same SRPM step COPR uses, then build the binary RPM from it.
bash packaging/rpm/make-srpm.sh /tmp/srpm
SRPM=$(ls -1 /tmp/srpm/*.src.rpm | head -n1)
TOP=/tmp/rpmbuild
rpmbuild --define "_topdir $TOP" --rebuild "$SRPM"

cp -t /out "$SRPM"
find "$TOP/RPMS" -name '*.rpm' -exec cp -t /out {} +
echo "Wrote:"
ls -1 /out/*.rpm
