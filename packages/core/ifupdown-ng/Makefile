# packages/core/ifupdown-ng/Makefile — Network interface configuration
# SPDX-License-Identifier: MIT

PKG_NAME     = ifupdown-ng
PKG_VERSION  = 0.12.1
PKG_LICENSE  = ISC
PKG_SOURCE   = https://github.com/ifupdown-ng/ifupdown-ng/releases/download/ifupdown-ng-$(PKG_VERSION)/ifupdown-ng-$(PKG_VERSION).tar.xz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	@echo ">>> [$(PKG_NAME)] No configure step needed"

build: configure
	$(MAKE) -C $(SRC_DIR) \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)" \
		PREFIX=/ \
		SYSCONFDIR=/etc \
		-j$$(nproc)

install: build
	$(MAKE) -C $(SRC_DIR) \
		PREFIX=/ \
		SYSCONFDIR=/etc \
		DESTDIR=$(DESTDIR) \
		install
	# Create default interfaces directory
	mkdir -p $(DESTDIR)/etc/network
