# packages/core/linux/Makefile — Linux kernel
# SPDX-License-Identifier: MIT
#
# NOTE: Linux is GPLv2 — the sole exception in jonerix. The license gate
# in rules.mk does not trigger because we set PKG_LICENSE to the SPDX
# identifier WITHOUT the forbidden substring. This is the only package
# where this exception is permitted. See DESIGN.md section 1.

PKG_NAME     = linux
PKG_VERSION  = 6.12.5
PKG_LICENSE  = GPL-2.0-only
PKG_SOURCE   = https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-$(PKG_VERSION).tar.xz
PKG_SHA256   = FIXME

# Override the license gate — Linux is the sole GPL exception in jonerix.
# We must clear FORBIDDEN_LICENSES before including rules.mk.
FORBIDDEN_LICENSES =

include ../../rules.mk

# Path to kernel config (architecture-specific)
KERNEL_ARCH    ?= x86_64
KERNEL_CONFIG  ?= $(TOPDIR)/config/kernel/$(KERNEL_ARCH).config

configure: patch
	cp $(KERNEL_CONFIG) $(SRC_DIR)/.config
	$(MAKE) -C $(SRC_DIR) \
		LLVM=1 \
		ARCH=x86 \
		olddefconfig

build: configure
	$(MAKE) -C $(SRC_DIR) \
		LLVM=1 \
		ARCH=x86 \
		-j$$(nproc)

install: build
	mkdir -p $(DESTDIR)/boot
	install -m 644 $(SRC_DIR)/arch/x86/boot/bzImage $(DESTDIR)/boot/vmlinuz
	# Install kernel modules
	$(MAKE) -C $(SRC_DIR) \
		LLVM=1 \
		ARCH=x86 \
		INSTALL_MOD_PATH=$(DESTDIR) \
		modules_install
	# Remove build/source symlinks from modules directory
	rm -f $(DESTDIR)/lib/modules/$(PKG_VERSION)/build
	rm -f $(DESTDIR)/lib/modules/$(PKG_VERSION)/source
