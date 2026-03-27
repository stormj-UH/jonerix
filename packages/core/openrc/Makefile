# packages/core/openrc/Makefile — OpenRC init system
# SPDX-License-Identifier: MIT

PKG_NAME     = openrc
PKG_VERSION  = 0.54
PKG_LICENSE  = BSD-2-Clause
PKG_SOURCE   = https://github.com/OpenRC/openrc/releases/download/$(PKG_VERSION)/openrc-$(PKG_VERSION).tar.gz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	mkdir -p $(BUILD_DIR)
	cd $(BUILD_DIR) && \
	meson setup \
		--prefix=/ \
		--buildtype=release \
		-Dos=Linux \
		-Dbranding="jonerix" \
		-Dpam=false \
		-Daudit=disabled \
		-Dselinux=disabled \
		-Dnewnet=false \
		-Dbash-completions=false \
		-Dzsh-completions=false \
		$(SRC_DIR)

build: configure
	ninja -C $(BUILD_DIR) -j$$(nproc)

install: build
	DESTDIR=$(DESTDIR) ninja -C $(BUILD_DIR) install
	# Ensure required directories exist
	mkdir -p $(DESTDIR)/etc/init.d
	mkdir -p $(DESTDIR)/etc/conf.d
	mkdir -p $(DESTDIR)/etc/runlevels/sysinit
	mkdir -p $(DESTDIR)/etc/runlevels/boot
	mkdir -p $(DESTDIR)/etc/runlevels/default
	mkdir -p $(DESTDIR)/etc/runlevels/shutdown
