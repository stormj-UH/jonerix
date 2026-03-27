# packages/core/python3/Makefile — Python 3 interpreter
# SPDX-License-Identifier: MIT
#
# Python 3 is needed as a build dependency for Node.js (GYP build system)
# and is useful as a general-purpose scripting language.
#
# Build notes for musl:
#   - --with-system-ffi=no: use bundled libffi to avoid extra dependency
#   - --without-ensurepip: pip is not needed in the base system
#   - Modules requiring glibc-specific features are disabled via Setup.local
#   - --enable-optimizations enables PGO (profile-guided optimization) but
#     significantly increases build time; omit for cross-compilation

PKG_NAME     = Python
PKG_VERSION  = 3.12.8
PKG_LICENSE  = PSF-2.0
PKG_SOURCE   = https://www.python.org/ftp/python/$(PKG_VERSION)/Python-$(PKG_VERSION).tar.xz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	cd $(SRC_DIR) && \
	CC="$(CC)" \
	CXX="clang++" \
	CFLAGS="$(CFLAGS) -I$(DESTDIR)/include" \
	LDFLAGS="$(LDFLAGS) -L$(DESTDIR)/lib" \
	CPPFLAGS="-I$(DESTDIR)/include" \
	./configure \
		--prefix=/ \
		--without-ensurepip \
		--with-openssl=$(DESTDIR) \
		--with-system-ffi=no \
		--with-lto \
		--disable-shared \
		--enable-ipv6 \
		--with-computed-gotos \
		--with-system-expat=no \
		--with-ssl-default-suites=openssl \
		--disable-test-modules \
		ac_cv_file__dev_ptmx=yes \
		ac_cv_file__dev_ptc=no

build: configure
	$(MAKE) -C $(SRC_DIR) -j$$(nproc)

install: build
	$(MAKE) -C $(SRC_DIR) \
		DESTDIR=$(DESTDIR) \
		install
	# Provide python3 -> python symlink
	ln -sf python3 $(DESTDIR)/bin/python
	# Remove test suite and other unnecessary files to save space
	rm -rf $(DESTDIR)/lib/python3.12/test
	rm -rf $(DESTDIR)/lib/python3.12/unittest/test
	rm -rf $(DESTDIR)/lib/python3.12/lib2to3/tests
	rm -rf $(DESTDIR)/lib/python3.12/idlelib
	rm -rf $(DESTDIR)/lib/python3.12/tkinter
	rm -rf $(DESTDIR)/lib/python3.12/turtledemo
	# Remove .pyc files to save space (they will be regenerated on first use)
	find $(DESTDIR)/lib/python3.12 -name '__pycache__' -type d -exec rm -rf {} + 2>/dev/null || true
