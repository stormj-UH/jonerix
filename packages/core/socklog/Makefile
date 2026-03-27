# packages/core/socklog/Makefile — Logging daemon (DJB-style)
# SPDX-License-Identifier: MIT

PKG_NAME     = socklog
PKG_VERSION  = 2.2.3
PKG_LICENSE  = BSD-3-Clause
PKG_SOURCE   = https://smarden.org/socklog/socklog-$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	# DJB-style build: set compiler in conf-cc and conf-ld
	echo "$(CC) $(CFLAGS)" > $(SRC_DIR)/src/conf-cc
	echo "$(CC) $(LDFLAGS)" > $(SRC_DIR)/src/conf-ld

build: configure
	cd $(SRC_DIR)/src && \
	$(MAKE)

install: build
	mkdir -p $(DESTDIR)/bin
	install -m 755 $(SRC_DIR)/src/socklog $(DESTDIR)/bin/socklog
	install -m 755 $(SRC_DIR)/src/socklog-check $(DESTDIR)/bin/socklog-check
	install -m 755 $(SRC_DIR)/src/tryto $(DESTDIR)/bin/tryto
	install -m 755 $(SRC_DIR)/src/uncat $(DESTDIR)/bin/uncat
	# Create log directories
	mkdir -p $(DESTDIR)/var/log/socklog
	# Install man pages
	mkdir -p $(DESTDIR)/share/man/man8
	install -m 644 $(SRC_DIR)/man/socklog.8 $(DESTDIR)/share/man/man8/socklog.8
