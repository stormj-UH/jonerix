# packages/core/unbound/Makefile — Recursive DNS resolver
# SPDX-License-Identifier: MIT

PKG_NAME     = unbound
PKG_VERSION  = 1.22.0
PKG_LICENSE  = BSD-3-Clause
PKG_SOURCE   = https://nlnetlabs.nl/downloads/unbound/unbound-$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	cd $(SRC_DIR) && \
	./configure \
		--prefix=/ \
		--sysconfdir=/etc \
		--with-ssl=$(DESTDIR) \
		--with-run-dir=/run/unbound \
		--with-pidfile=/run/unbound/unbound.pid \
		--with-username=unbound \
		--with-chroot-dir="" \
		--disable-gost \
		--enable-dnscrypt \
		--enable-subnet \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)"

build: configure
	$(MAKE) -C $(SRC_DIR) -j$$(nproc)

install: build
	$(MAKE) -C $(SRC_DIR) DESTDIR=$(DESTDIR) install
	mkdir -p $(DESTDIR)/run/unbound
