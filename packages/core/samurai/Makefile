# packages/core/samurai/Makefile — ninja-compatible build tool
# SPDX-License-Identifier: MIT

PKG_NAME     = samurai
PKG_VERSION  = 1.2
PKG_LICENSE  = Apache-2.0
PKG_SOURCE   = https://github.com/nicknamenamenick/samurai/releases/download/$(PKG_VERSION)/samurai-$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	@echo ">>> [$(PKG_NAME)] No configure step needed (simple C project)"

build: configure
	$(MAKE) -C $(SRC_DIR) \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)" \
		-j$$(nproc)

install: build
	mkdir -p $(DESTDIR)/bin
	install -m 755 $(SRC_DIR)/samu $(DESTDIR)/bin/samu
	# Provide ninja symlink for compatibility
	ln -sf samu $(DESTDIR)/bin/ninja
	# Install man page
	mkdir -p $(DESTDIR)/share/man/man1
	install -m 644 $(SRC_DIR)/samu.1 $(DESTDIR)/share/man/man1/samu.1
