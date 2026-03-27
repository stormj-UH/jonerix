# packages/core/curl/Makefile — HTTP/HTTPS client
# SPDX-License-Identifier: MIT

PKG_NAME     = curl
PKG_VERSION  = 8.11.1
PKG_LICENSE  = curl
PKG_SOURCE   = https://curl.se/download/curl-$(PKG_VERSION).tar.xz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	cd $(SRC_DIR) && \
	./configure \
		--prefix=/ \
		--with-ssl=$(DESTDIR) \
		--with-ca-bundle=/etc/ssl/cert.pem \
		--enable-ipv6 \
		--enable-unix-sockets \
		--disable-ldap \
		--disable-ldaps \
		--disable-rtsp \
		--disable-dict \
		--disable-telnet \
		--disable-tftp \
		--disable-pop3 \
		--disable-imap \
		--disable-smb \
		--disable-smtp \
		--disable-gopher \
		--disable-mqtt \
		--disable-manual \
		--without-libpsl \
		--without-libidn2 \
		--without-brotli \
		--without-nghttp2 \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)"

build: configure
	$(MAKE) -C $(SRC_DIR) -j$$(nproc)

install: build
	$(MAKE) -C $(SRC_DIR) DESTDIR=$(DESTDIR) install
