# packages/core/zstd/Makefile — Zstandard compression library and tool
# SPDX-License-Identifier: MIT

PKG_NAME     = zstd
PKG_VERSION  = 1.5.6
PKG_LICENSE  = BSD-3-Clause
PKG_SOURCE   = https://github.com/facebook/zstd/releases/download/v$(PKG_VERSION)/zstd-$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	mkdir -p $(BUILD_DIR)
	cd $(BUILD_DIR) && \
	cmake $(SRC_DIR)/build/cmake \
		-G Ninja \
		-DCMAKE_C_COMPILER="$(CC)" \
		-DCMAKE_C_FLAGS="$(CFLAGS)" \
		-DCMAKE_EXE_LINKER_FLAGS="$(LDFLAGS)" \
		-DCMAKE_INSTALL_PREFIX=/ \
		-DCMAKE_BUILD_TYPE=Release \
		-DZSTD_BUILD_STATIC=ON \
		-DZSTD_BUILD_SHARED=ON \
		-DZSTD_BUILD_PROGRAMS=ON \
		-DZSTD_BUILD_TESTS=OFF \
		-DZSTD_BUILD_CONTRIB=OFF

build: configure
	ninja -C $(BUILD_DIR) -j$$(nproc)

install: build
	DESTDIR=$(DESTDIR) ninja -C $(BUILD_DIR) install
