# jpkg - jonerix package manager
# Makefile - builds jpkg binary
#
# MIT License
# Copyright (c) 2026 jonerix contributors

# Compiler and flags
CC       ?= clang
LD       ?= ld.lld
AR       ?= llvm-ar

# Build configuration
PREFIX   ?= /usr/local
DESTDIR  ?=

# Detect build environment
UNAME_S  := $(shell uname -s)

# Core CFLAGS (always applied)
CFLAGS   += -std=c11 -Wall -Wextra -Wpedantic -Werror=implicit-function-declaration
CFLAGS   += -Wno-unused-parameter -Wshadow -Wstrict-prototypes
CFLAGS   += -Os -pipe
CFLAGS   += -D_POSIX_C_SOURCE=200809L -D_DEFAULT_SOURCE

# Security hardening flags
CFLAGS   += -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2
LDFLAGS  += -pie

# Static linking (for production builds on musl)
# Uncomment for production:
# CFLAGS  += -static
# LDFLAGS += -static -Wl,-z,relro,-z,now

# LibreSSL support (uncomment when available)
# CFLAGS  += -DJPKG_USE_LIBRESSL
# LDFLAGS += -ltls -lssl -lcrypto

# Source files
SRCDIR   = src
SRCS     = $(SRCDIR)/main.c \
           $(SRCDIR)/util.c \
           $(SRCDIR)/toml.c \
           $(SRCDIR)/pkg.c \
           $(SRCDIR)/repo.c \
           $(SRCDIR)/db.c \
           $(SRCDIR)/fetch.c \
           $(SRCDIR)/sign.c \
           $(SRCDIR)/deps.c \
           $(SRCDIR)/cmd_install.c \
           $(SRCDIR)/cmd_remove.c \
           $(SRCDIR)/cmd_update.c \
           $(SRCDIR)/cmd_upgrade.c \
           $(SRCDIR)/cmd_search.c \
           $(SRCDIR)/cmd_info.c \
           $(SRCDIR)/cmd_build.c \
           $(SRCDIR)/cmd_verify.c \
           $(SRCDIR)/cmd_audit.c \
           $(SRCDIR)/cmd_keygen.c \
           $(SRCDIR)/cmd_sign.c

OBJS     = $(SRCS:.c=.o)
DEPS     = $(SRCS:.c=.d)
TARGET   = jpkg

# Header dependencies
HDRS     = $(SRCDIR)/util.h \
           $(SRCDIR)/toml.h \
           $(SRCDIR)/pkg.h \
           $(SRCDIR)/repo.h \
           $(SRCDIR)/db.h \
           $(SRCDIR)/fetch.h \
           $(SRCDIR)/sign.h \
           $(SRCDIR)/deps.h

.PHONY: all clean install test check

all: $(TARGET)

$(TARGET): $(OBJS)
	$(CC) $(OBJS) $(LDFLAGS) -o $@
	@echo "Built $(TARGET) ($$(wc -c < $(TARGET) | tr -d ' ') bytes)"

# Compile with dependency tracking
%.o: %.c
	$(CC) $(CFLAGS) -MMD -MP -c $< -o $@

# Include auto-generated dependencies
-include $(DEPS)

# Installation
install: $(TARGET)
	install -d $(DESTDIR)$(PREFIX)/bin
	install -m 755 $(TARGET) $(DESTDIR)$(PREFIX)/bin/$(TARGET)
	install -d $(DESTDIR)/etc/jpkg/keys
	install -d $(DESTDIR)/var/db/jpkg/installed
	install -d $(DESTDIR)/var/cache/jpkg
	@echo "Installed $(TARGET) to $(DESTDIR)$(PREFIX)/bin/"

# Uninstall
uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/$(TARGET)
	@echo "Uninstalled $(TARGET)"

# Run tests
test: $(TARGET)
	$(MAKE) -C tests

check: test

# Clean
clean:
	rm -f $(OBJS) $(DEPS) $(TARGET)
	$(MAKE) -C tests clean

# Size report
size: $(TARGET)
	@echo "=== Binary Size ==="
	@ls -lh $(TARGET)
	@echo ""
	@echo "=== Section Sizes ==="
	@size $(TARGET) 2>/dev/null || true
	@echo ""
	@echo "=== Object Sizes ==="
	@for f in $(OBJS); do printf "  %6s  %s\n" "$$(wc -c < $$f | tr -d ' ')" "$$f"; done

# Format check (requires clang-format)
fmt:
	clang-format -i $(SRCS) $(HDRS)

# Static analysis (requires clang-tidy or scan-build)
analyze:
	scan-build --use-cc=$(CC) $(MAKE) clean all

.SUFFIXES:
