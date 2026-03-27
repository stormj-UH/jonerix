# packages/core/nodejs/Makefile — Node.js JavaScript runtime
# SPDX-License-Identifier: MIT
#
# Node.js builds V8 internally. It requires Python 3 for its GYP build system.
# Link against libc++ (not libstdc++) to avoid GPL dependency.
#
# musl compatibility notes:
#   - V8 uses execinfo.h (backtrace) which musl does not provide.
#     A patch or stub may be needed (see patches/ directory).
#   - Some platform-specific code assumes glibc; patches may be required
#     for musl's smaller feature set (e.g., no mallopt, no backtrace).
#   - Node.js 22.x has improved musl support from Alpine contributions.

PKG_NAME     = node
PKG_VERSION  = 22.12.0
PKG_LICENSE  = MIT
PKG_SOURCE   = https://nodejs.org/dist/v$(PKG_VERSION)/node-v$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	cd $(SRC_DIR) && \
	CC="$(CC)" \
	CXX="clang++ -stdlib=libc++" \
	CXXFLAGS="-stdlib=libc++ -I$(DESTDIR)/include/c++/v1 -I$(DESTDIR)/include" \
	LDFLAGS="$(LDFLAGS) -L$(DESTDIR)/lib -lc++ -lc++abi" \
	PYTHON="$(DESTDIR)/bin/python3" \
	./configure \
		--prefix=/ \
		--ninja \
		--partly-static \
		--shared-openssl \
		--shared-openssl-libpath=$(DESTDIR)/lib \
		--shared-openssl-includes=$(DESTDIR)/include \
		--shared-zlib \
		--shared-zlib-libpath=$(DESTDIR)/lib \
		--shared-zlib-includes=$(DESTDIR)/include \
		--with-intl=none \
		--openssl-use-def-ca-store \
		--without-npm \
		--without-corepack

build: configure
	$(MAKE) -C $(SRC_DIR) -j$$(nproc)

install: build
	$(MAKE) -C $(SRC_DIR) \
		DESTDIR=$(DESTDIR) \
		install
	# Remove unnecessary files to save space
	rm -rf $(DESTDIR)/include/node
	rm -rf $(DESTDIR)/share/doc/node
