# packages/core/toybox/Makefile — BSD-licensed coreutils
# SPDX-License-Identifier: MIT

PKG_NAME     = toybox
PKG_VERSION  = 0.8.11
PKG_LICENSE  = 0BSD
PKG_SOURCE   = https://landley.net/toybox/downloads/toybox-$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	cp $(PKG_DIR)/toybox.config $(SRC_DIR)/.config

build: configure
	$(MAKE) -C $(SRC_DIR) \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)" \
		-j$$(nproc)

install: build
	$(MAKE) -C $(SRC_DIR) \
		PREFIX=$(DESTDIR) \
		install
