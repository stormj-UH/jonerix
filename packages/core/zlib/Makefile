# packages/core/zlib/Makefile — zlib compression library
# SPDX-License-Identifier: MIT

PKG_NAME     = zlib
PKG_VERSION  = 1.3.1
PKG_LICENSE  = Zlib
PKG_SOURCE   = https://github.com/madler/zlib/releases/download/v$(PKG_VERSION)/zlib-$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	cd $(SRC_DIR) && \
	CC="$(CC)" \
	CFLAGS="$(CFLAGS)" \
	LDFLAGS="$(LDFLAGS)" \
	./configure \
		--prefix=/ \
		--static

build: configure
	$(MAKE) -C $(SRC_DIR) \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)" \
		-j$$(nproc)

install: build
	$(MAKE) -C $(SRC_DIR) \
		DESTDIR=$(DESTDIR) \
		install
