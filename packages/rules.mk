# jonerix shared build rules for package recipes
#
# Every package under packages/core/ includes this file. It provides:
#   - Standard variables (CC, CFLAGS, LDFLAGS, paths)
#   - fetch:     download source tarball and verify SHA256
#   - extract:   unpack tarball into $(SRC_DIR)
#   - patch:     apply any patches from patches/ directory
#   - configure: (hook — overridden by package Makefile)
#   - build:     (hook — overridden by package Makefile)
#   - install:   (hook — overridden by package Makefile)
#   - clean:     remove build artifacts
#   - LICENSE GATE: aborts if PKG_LICENSE contains GPL, LGPL, or AGPL
#
# Required variables (set by each package Makefile before include):
#   PKG_NAME     — package name (e.g., toybox)
#   PKG_VERSION  — package version (e.g., 0.8.11)
#   PKG_LICENSE  — SPDX license identifier (e.g., 0BSD)
#   PKG_SOURCE   — download URL for source tarball
#   PKG_SHA256   — expected SHA256 checksum of tarball
#
# SPDX-License-Identifier: MIT

# =========================================================================
# License gate — abort if package is GPL/LGPL/AGPL
# =========================================================================

FORBIDDEN_LICENSES = GPL LGPL AGPL
$(foreach lic,$(FORBIDDEN_LICENSES),\
  $(if $(findstring $(lic),$(PKG_LICENSE)),\
    $(error BLOCKED: $(PKG_NAME) is $(PKG_LICENSE) — not permitted in jonerix)))

# =========================================================================
# Toolchain defaults
# =========================================================================

CC       ?= clang
LD       ?= ld.lld
AR       ?= llvm-ar
RANLIB   ?= llvm-ranlib
STRIP    ?= llvm-strip
NM       ?= llvm-nm
OBJCOPY  ?= llvm-objcopy

CFLAGS   ?= -Os -pipe -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2
LDFLAGS  ?= -Wl,-z,relro,-z,now -pie

# =========================================================================
# Directory layout
# =========================================================================

# Root of the jonerix repo
TOPDIR     ?= $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/..)

# Where the package Makefile lives (e.g., packages/core/toybox/)
PKG_DIR    := $(CURDIR)

# Where downloaded tarballs are cached
DL_DIR     ?= $(TOPDIR)/downloads

# Where source is extracted for building
SRC_DIR    ?= $(TOPDIR)/build/src/$(PKG_NAME)-$(PKG_VERSION)

# Where out-of-tree build artifacts go
BUILD_DIR  ?= $(TOPDIR)/build/obj/$(PKG_NAME)

# Installation destination (sysroot during bootstrap, / during self-host)
DESTDIR    ?= /jonerix-sysroot

# The tarball filename (derived from the URL)
PKG_TARBALL = $(DL_DIR)/$(notdir $(PKG_SOURCE))

# =========================================================================
# Phony targets
# =========================================================================

.PHONY: all fetch extract patch configure build install clean distclean

all: build

# =========================================================================
# fetch — download source tarball and verify SHA256
# =========================================================================

fetch: $(PKG_TARBALL)

$(PKG_TARBALL):
	@echo ">>> [$(PKG_NAME)] Fetching $(PKG_SOURCE)"
	@mkdir -p $(DL_DIR)
	curl -fSL -o $@.tmp $(PKG_SOURCE)
	@mv $@.tmp $@
	@echo ">>> [$(PKG_NAME)] Verifying SHA256..."
	@_actual=$$(sha256sum $@ | cut -d' ' -f1); \
	if [ "$(PKG_SHA256)" = "FIXME" ]; then \
		echo "WARNING: SHA256 not set for $(PKG_NAME) (FIXME placeholder)"; \
		echo "  Actual SHA256: $$_actual"; \
	elif [ "$$_actual" != "$(PKG_SHA256)" ]; then \
		echo "ERROR: SHA256 mismatch for $(PKG_NAME)"; \
		echo "  Expected: $(PKG_SHA256)"; \
		echo "  Actual:   $$_actual"; \
		rm -f $@; \
		exit 1; \
	else \
		echo ">>> [$(PKG_NAME)] SHA256 OK"; \
	fi

# =========================================================================
# extract — unpack tarball into $(SRC_DIR)
# =========================================================================

extract: fetch
	@if [ -d "$(SRC_DIR)" ]; then \
		echo ">>> [$(PKG_NAME)] Already extracted"; \
	else \
		echo ">>> [$(PKG_NAME)] Extracting $(PKG_TARBALL)"; \
		mkdir -p $(dir $(SRC_DIR)); \
		case "$(PKG_TARBALL)" in \
			*.tar.gz|*.tgz)  tar xzf $(PKG_TARBALL) -C $(dir $(SRC_DIR)) ;; \
			*.tar.xz)        tar xJf $(PKG_TARBALL) -C $(dir $(SRC_DIR)) ;; \
			*.tar.bz2)       tar xjf $(PKG_TARBALL) -C $(dir $(SRC_DIR)) ;; \
			*.tar.zst)       zstd -d $(PKG_TARBALL) --stdout | tar xf - -C $(dir $(SRC_DIR)) ;; \
			*)               echo "ERROR: Unknown archive format"; exit 1 ;; \
		esac; \
	fi

# =========================================================================
# patch — apply patches from patches/ directory
# =========================================================================

patch: extract
	@if [ -d "$(PKG_DIR)/patches" ]; then \
		for p in $(PKG_DIR)/patches/*.patch; do \
			[ -f "$$p" ] || continue; \
			echo ">>> [$(PKG_NAME)] Applying patch: $$p"; \
			patch -d $(SRC_DIR) -p1 < "$$p" || exit 1; \
		done; \
	else \
		echo ">>> [$(PKG_NAME)] No patches to apply"; \
	fi

# =========================================================================
# configure — hook for package-specific configuration
#
# Override this target in the package Makefile. Default is a no-op.
# =========================================================================

configure: patch
	@echo ">>> [$(PKG_NAME)] Configure (default: no-op)"

# =========================================================================
# build — hook for package-specific build commands
#
# Override this target in the package Makefile.
# =========================================================================

build: configure
	@echo ">>> [$(PKG_NAME)] Build (default: no-op — override in package Makefile)"

# =========================================================================
# install — hook for package-specific install commands
#
# Override this target in the package Makefile.
# =========================================================================

install: build
	@echo ">>> [$(PKG_NAME)] Install (default: no-op — override in package Makefile)"

# =========================================================================
# clean — remove build artifacts for this package
# =========================================================================

clean:
	@echo ">>> [$(PKG_NAME)] Cleaning..."
	rm -rf $(SRC_DIR)
	rm -rf $(BUILD_DIR)

# =========================================================================
# distclean — also remove downloaded tarball
# =========================================================================

distclean: clean
	@echo ">>> [$(PKG_NAME)] Removing downloaded tarball..."
	rm -f $(PKG_TARBALL)

# =========================================================================
# info — display package metadata
# =========================================================================

.PHONY: info
info:
	@echo "Package:  $(PKG_NAME)"
	@echo "Version:  $(PKG_VERSION)"
	@echo "License:  $(PKG_LICENSE)"
	@echo "Source:   $(PKG_SOURCE)"
	@echo "SHA256:   $(PKG_SHA256)"
	@echo "SRC_DIR:  $(SRC_DIR)"
	@echo "DESTDIR:  $(DESTDIR)"
	@echo "CC:       $(CC)"
	@echo "CFLAGS:   $(CFLAGS)"
	@echo "LDFLAGS:  $(LDFLAGS)"
