# packages/core/mksh/Makefile — MirBSD Korn Shell
# SPDX-License-Identifier: MIT

PKG_NAME     = mksh
PKG_VERSION  = R59c
PKG_LICENSE  = MirOS
PKG_SOURCE   = https://www.mirbsd.org/MirOS/dist/mir/mksh/mksh-$(PKG_VERSION).tgz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	@echo ">>> [$(PKG_NAME)] Build.sh handles configuration"

build: configure
	cd $(SRC_DIR) && \
	CC="$(CC)" \
	CFLAGS="$(CFLAGS)" \
	LDFLAGS="$(LDFLAGS)" \
	sh Build.sh -r

install: build
	mkdir -p $(DESTDIR)/bin
	install -m 755 $(SRC_DIR)/mksh $(DESTDIR)/bin/mksh
	# Install as /bin/sh for POSIX compatibility
	ln -sf mksh $(DESTDIR)/bin/sh
	# Install man page
	mkdir -p $(DESTDIR)/share/man/man1
	install -m 644 $(SRC_DIR)/mksh.1 $(DESTDIR)/share/man/man1/mksh.1
