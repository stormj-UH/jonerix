# jpkg test suite
#
# MIT License
# Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software

CC       ?= clang
CFLAGS   += -std=c11 -Wall -Wextra -g -O0
CFLAGS   += -D_POSIX_C_SOURCE=200809L -D_DEFAULT_SOURCE

SRCDIR   = ../src

# Shared source files (used by all tests)
COMMON_SRCS = $(SRCDIR)/util.c $(SRCDIR)/toml.c

# Test binaries
TESTS    = test_toml test_pkg test_deps test_db

.PHONY: all clean run

all: $(TESTS)
	@echo "All tests built."

run: all
	@echo ""
	@echo "=== Running test suite ==="
	@echo ""
	@failed=0; \
	for t in $(TESTS); do \
		echo "--- Running $$t ---"; \
		./$$t; \
		if [ $$? -ne 0 ]; then failed=$$((failed + 1)); fi; \
		echo ""; \
	done; \
	if [ $$failed -gt 0 ]; then \
		echo "$$failed test suite(s) FAILED"; \
		exit 1; \
	else \
		echo "All test suites PASSED"; \
	fi

test_toml: test_toml.c $(COMMON_SRCS) $(SRCDIR)/toml.h $(SRCDIR)/util.h
	$(CC) $(CFLAGS) -o $@ test_toml.c $(COMMON_SRCS)

test_pkg: test_pkg.c $(COMMON_SRCS) $(SRCDIR)/pkg.c $(SRCDIR)/pkg.h $(SRCDIR)/util.h
	$(CC) $(CFLAGS) -o $@ test_pkg.c $(COMMON_SRCS) $(SRCDIR)/pkg.c

test_deps: test_deps.c $(COMMON_SRCS) $(SRCDIR)/deps.c $(SRCDIR)/repo.c \
           $(SRCDIR)/fetch.c $(SRCDIR)/sign.c $(SRCDIR)/pkg.c $(SRCDIR)/db.c \
           $(SRCDIR)/deps.h $(SRCDIR)/repo.h $(SRCDIR)/util.h
	$(CC) $(CFLAGS) -o $@ test_deps.c $(COMMON_SRCS) \
		$(SRCDIR)/deps.c $(SRCDIR)/repo.c $(SRCDIR)/fetch.c \
		$(SRCDIR)/sign.c $(SRCDIR)/pkg.c $(SRCDIR)/db.c

test_db: test_db.c $(COMMON_SRCS) $(SRCDIR)/db.c $(SRCDIR)/pkg.c \
         $(SRCDIR)/db.h $(SRCDIR)/pkg.h $(SRCDIR)/util.h
	$(CC) $(CFLAGS) -o $@ test_db.c $(COMMON_SRCS) \
		$(SRCDIR)/db.c $(SRCDIR)/pkg.c

clean:
	rm -f $(TESTS)
	rm -f *.o

.SUFFIXES:
