# Fedora COPR entry point ("SCM" source, build SRPM with "make_srpm").
# COPR runs, as root in a disposable Fedora chroot, with network access:
#     make -f <repo>/.copr/Makefile srpm outdir=<dir> spec=<spec>
# The binary RPMs are then built from the SRPM offline, once per chroot.

TOP := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/..)
outdir ?= $(TOP)/dist/copr

.PHONY: srpm
srpm:
	dnf install -y --setopt=install_weak_deps=False cargo rust rpm-build tar gzip
	bash $(TOP)/packaging/rpm/make-srpm.sh $(outdir)
