# packages/core/libressl/Makefile — TLS library (LibreSSL)
# SPDX-License-Identifier: MIT

PKG_NAME     = libressl
PKG_VERSION  = 4.0.0
PKG_LICENSE  = ISC
PKG_SOURCE   = https://ftp.openbsd.org/pub/OpenBSD/LibreSSL/libressl-$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	mkdir -p $(BUILD_DIR)
	cd $(BUILD_DIR) && \
	cmake $(SRC_DIR) \
		-G Ninja \
		-DCMAKE_C_COMPILER="$(CC)" \
		-DCMAKE_C_FLAGS="$(CFLAGS)" \
		-DCMAKE_EXE_LINKER_FLAGS="$(LDFLAGS)" \
		-DCMAKE_INSTALL_PREFIX=/ \
		-DCMAKE_BUILD_TYPE=Release \
		-DLIBRESSL_APPS=ON \
		-DLIBRESSL_TESTS=OFF \
		-DBUILD_SHARED_LIBS=ON

build: configure
	ninja -C $(BUILD_DIR) -j$$(nproc)

install: build
	DESTDIR=$(DESTDIR) ninja -C $(BUILD_DIR) install
	# Install CA certificates directory
	mkdir -p $(DESTDIR)/etc/ssl
