# packages/core/pigz/Makefile — Parallel gzip compression
# SPDX-License-Identifier: MIT

PKG_NAME     = pigz
PKG_VERSION  = 2.8
PKG_LICENSE  = Zlib
PKG_SOURCE   = https://zlib.net/pigz/pigz-$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	@echo ">>> [$(PKG_NAME)] No configure step needed"

build: configure
	$(MAKE) -C $(SRC_DIR) \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)" \
		-j$$(nproc)

install: build
	mkdir -p $(DESTDIR)/bin
	install -m 755 $(SRC_DIR)/pigz $(DESTDIR)/bin/pigz
	# Provide gzip/gunzip/zcat symlinks
	ln -sf pigz $(DESTDIR)/bin/gzip
	ln -sf pigz $(DESTDIR)/bin/gunzip
	ln -sf pigz $(DESTDIR)/bin/zcat
	# Install man page
	mkdir -p $(DESTDIR)/share/man/man1
	install -m 644 $(SRC_DIR)/pigz.1 $(DESTDIR)/share/man/man1/pigz.1
