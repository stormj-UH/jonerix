# packages/core/musl/Makefile — C library
# SPDX-License-Identifier: MIT

PKG_NAME     = musl
PKG_VERSION  = 1.2.5
PKG_LICENSE  = MIT
PKG_SOURCE   = https://musl.libc.org/releases/musl-$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	cd $(SRC_DIR) && \
	./configure \
		--prefix=/ \
		--syslibdir=/lib \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)"

build: configure
	$(MAKE) -C $(SRC_DIR) -j$$(nproc)

install: build
	$(MAKE) -C $(SRC_DIR) DESTDIR=$(DESTDIR) install
	# Create compatibility symlinks
	mkdir -p $(DESTDIR)/lib
	ln -sf libc.so $(DESTDIR)/lib/ld-musl-$(JONERIX_ARCH).so.1
