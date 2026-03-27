# packages/core/dropbear/Makefile — Lightweight SSH server
# SPDX-License-Identifier: MIT

PKG_NAME     = dropbear
PKG_VERSION  = 2024.86
PKG_LICENSE  = MIT
PKG_SOURCE   = https://matt.ucc.asn.au/dropbear/releases/dropbear-$(PKG_VERSION).tar.bz2
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	cd $(SRC_DIR) && \
	./configure \
		--prefix=/ \
		--disable-wtmp \
		--disable-lastlog \
		--enable-static \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)"

build: configure
	$(MAKE) -C $(SRC_DIR) \
		PROGRAMS="dropbear dbclient dropbearkey dropbearconvert scp" \
		-j$$(nproc)

install: build
	$(MAKE) -C $(SRC_DIR) \
		DESTDIR=$(DESTDIR) \
		PROGRAMS="dropbear dbclient dropbearkey dropbearconvert scp" \
		install
	# Create host key directory
	mkdir -p $(DESTDIR)/etc/dropbear
