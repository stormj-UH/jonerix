# packages/core/nvi/Makefile — nvi2 text editor
# SPDX-License-Identifier: MIT

PKG_NAME     = nvi2
PKG_VERSION  = 2.2.1
PKG_LICENSE  = BSD-3-Clause
PKG_SOURCE   = https://github.com/lichray/nvi2/releases/download/v$(PKG_VERSION)/nvi2-$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	mkdir -p $(BUILD_DIR)
	cd $(BUILD_DIR) && \
	cmake $(SRC_DIR) \
		-G Ninja \
		-DCMAKE_C_COMPILER="$(CC)" \
		-DCMAKE_C_FLAGS="$(CFLAGS)" \
		-DCMAKE_EXE_LINKER_FLAGS="$(LDFLAGS)" \
		-DCMAKE_INSTALL_PREFIX=/ \
		-DCMAKE_BUILD_TYPE=Release

build: configure
	ninja -C $(BUILD_DIR) -j$$(nproc)

install: build
	DESTDIR=$(DESTDIR) ninja -C $(BUILD_DIR) install
	# Create vi symlink
	mkdir -p $(DESTDIR)/bin
	ln -sf nvi $(DESTDIR)/bin/vi
