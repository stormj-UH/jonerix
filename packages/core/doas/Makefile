# packages/core/doas/Makefile — Privilege escalation (OpenDoas)
# SPDX-License-Identifier: MIT

PKG_NAME     = opendoas
PKG_VERSION  = 6.3p12
PKG_LICENSE  = ISC
PKG_SOURCE   = https://github.com/Duncaen/OpenDoas/releases/download/v$(PKG_VERSION)/opendoas-$(PKG_VERSION).tar.xz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	cd $(SRC_DIR) && \
	./configure \
		--prefix=/ \
		--with-timestamp \
		--without-pam

build: configure
	$(MAKE) -C $(SRC_DIR) \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)" \
		-j$$(nproc)

install: build
	$(MAKE) -C $(SRC_DIR) DESTDIR=$(DESTDIR) install
	# Install default config
	mkdir -p $(DESTDIR)/etc
	echo "permit :wheel" > $(DESTDIR)/etc/doas.conf
	chmod 600 $(DESTDIR)/etc/doas.conf
