# packages/core/dhcpcd/Makefile — DHCP client daemon
# SPDX-License-Identifier: MIT

PKG_NAME     = dhcpcd
PKG_VERSION  = 10.1.0
PKG_LICENSE  = BSD-2-Clause
PKG_SOURCE   = https://github.com/NetworkConfiguration/dhcpcd/releases/download/v$(PKG_VERSION)/dhcpcd-$(PKG_VERSION).tar.xz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	cd $(SRC_DIR) && \
	./configure \
		--prefix=/ \
		--sysconfdir=/etc \
		--runstatedir=/run \
		--dbdir=/var/db/dhcpcd \
		--privsepuser=dhcpcd \
		--enable-ipv6 \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)"

build: configure
	$(MAKE) -C $(SRC_DIR) -j$$(nproc)

install: build
	$(MAKE) -C $(SRC_DIR) DESTDIR=$(DESTDIR) install
	mkdir -p $(DESTDIR)/var/db/dhcpcd
