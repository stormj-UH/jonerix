# packages/core/snooze/Makefile — Lightweight cron alternative
# SPDX-License-Identifier: MIT

PKG_NAME     = snooze
PKG_VERSION  = 0.5
PKG_LICENSE  = CC0-1.0
PKG_SOURCE   = https://github.com/leahneukirchen/snooze/archive/v$(PKG_VERSION).tar.gz
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
	install -m 755 $(SRC_DIR)/snooze $(DESTDIR)/bin/snooze
	# Install man page
	mkdir -p $(DESTDIR)/share/man/man1
	install -m 644 $(SRC_DIR)/snooze.1 $(DESTDIR)/share/man/man1/snooze.1
