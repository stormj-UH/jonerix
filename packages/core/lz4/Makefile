# packages/core/lz4/Makefile — LZ4 compression library and tool
# SPDX-License-Identifier: MIT

PKG_NAME     = lz4
PKG_VERSION  = 1.10.0
PKG_LICENSE  = BSD-2-Clause
PKG_SOURCE   = https://github.com/lz4/lz4/releases/download/v$(PKG_VERSION)/lz4-$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	@echo ">>> [$(PKG_NAME)] No configure step needed (plain Makefile build)"

build: configure
	$(MAKE) -C $(SRC_DIR) \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)" \
		PREFIX=/ \
		-j$$(nproc)

install: build
	$(MAKE) -C $(SRC_DIR) \
		PREFIX=/ \
		DESTDIR=$(DESTDIR) \
		install
