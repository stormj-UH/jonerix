# packages/core/mandoc/Makefile — mandoc man page system
# SPDX-License-Identifier: MIT

PKG_NAME     = mandoc
PKG_VERSION  = 1.14.6
PKG_LICENSE  = ISC
PKG_SOURCE   = https://mandoc.bsd.lv/snapshots/mandoc-$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	# mandoc uses a configure.local file instead of autoconf
	cat > $(SRC_DIR)/configure.local <<-'LOCALEOF'
	PREFIX=/
	BINDIR=/bin
	SBINDIR=/bin
	LIBDIR=/lib
	MANDIR=/share/man
	INCLUDEDIR=/include
	MANPATH_DEFAULT="/share/man"
	CC="$(CC)"
	CFLAGS="$(CFLAGS)"
	LDFLAGS="$(LDFLAGS)"
	HAVE_WCHAR=1
	HAVE_SQLITE3=0
	UTF8_LOCALE="C.UTF-8"
	LOCALEOF
	cd $(SRC_DIR) && \
	./configure

build: configure
	$(MAKE) -C $(SRC_DIR) -j$$(nproc)

install: build
	$(MAKE) -C $(SRC_DIR) DESTDIR=$(DESTDIR) install
