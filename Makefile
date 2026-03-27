# jonerix top-level Makefile
#
# Targets:
#   bootstrap  - Run stage0 through stage2 to produce the jonerix rootfs
#   image      - Build a bootable disk image (requires stage2 output)
#   oci        - Build an OCI container image (requires stage2 output)
#   verify     - Run stage3 self-hosting verification
#   clean      - Remove all build artifacts
#   license-audit - Verify no GPL binaries in the rootfs
#
# Usage:
#   make bootstrap   # full build from Alpine host
#   make image       # after bootstrap, produce bootable image
#   make oci         # after bootstrap, produce OCI image

SHELL       := /bin/sh
ROOTDIR     := $(CURDIR)
BOOTSTRAP   := $(ROOTDIR)/bootstrap
OUTPUT      := $(ROOTDIR)/output
IMAGE_DIR   := $(ROOTDIR)/image
SCRIPTS_DIR := $(ROOTDIR)/scripts

.PHONY: all bootstrap stage0 stage1 stage2 image oci verify clean license-audit help

all: bootstrap

# --------------------------------------------------------------------------
# Bootstrap: stages 0 through 2
# --------------------------------------------------------------------------

bootstrap: stage0 stage1 stage2
	@echo "=== Bootstrap complete ==="
	@echo "Rootfs tarball is in $(OUTPUT)/"

stage0:
	@echo "=== Stage 0: Alpine build host setup ==="
	sh $(BOOTSTRAP)/stage0.sh

stage1: stage0
	@echo "=== Stage 1: Cross-compile permissive world ==="
	sh $(BOOTSTRAP)/stage1.sh

stage2: stage1
	@echo "=== Stage 2: Assemble root filesystem ==="
	sh $(BOOTSTRAP)/stage2.sh

# --------------------------------------------------------------------------
# Image targets (require completed stage2)
# --------------------------------------------------------------------------

image:
	@echo "=== Building bootable disk image ==="
	@test -d $(OUTPUT) || { echo "ERROR: Run 'make bootstrap' first."; exit 1; }
	sh $(IMAGE_DIR)/mkimage.sh

oci:
	@echo "=== Building OCI container image ==="
	@test -d $(OUTPUT) || { echo "ERROR: Run 'make bootstrap' first."; exit 1; }
	sh $(IMAGE_DIR)/oci.sh

# --------------------------------------------------------------------------
# Verification
# --------------------------------------------------------------------------

verify:
	@echo "=== Stage 3: Self-hosting verification ==="
	sh $(BOOTSTRAP)/stage3-verify.sh

license-audit:
	@echo "=== License audit ==="
	sh $(SCRIPTS_DIR)/license-audit.sh

# --------------------------------------------------------------------------
# Cleanup
# --------------------------------------------------------------------------

clean:
	@echo "=== Cleaning build artifacts ==="
	rm -rf /jonerix-sysroot
	rm -rf /jonerix-rootfs
	rm -rf $(OUTPUT)
	rm -rf /jonerix-build
	@echo "Clean complete."

# --------------------------------------------------------------------------
# Help
# --------------------------------------------------------------------------

help:
	@echo "jonerix build system"
	@echo ""
	@echo "Targets:"
	@echo "  bootstrap      Run full Stage 0-2 build (requires Alpine host)"
	@echo "  stage0         Stage 0 only: install Alpine build dependencies"
	@echo "  stage1         Stage 1 only: cross-compile all components"
	@echo "  stage2         Stage 2 only: assemble clean rootfs"
	@echo "  image          Build bootable disk image (run after bootstrap)"
	@echo "  oci            Build OCI container image (run after bootstrap)"
	@echo "  verify         Stage 3: self-hosting verification"
	@echo "  license-audit  Verify all rootfs binaries are permissive"
	@echo "  clean          Remove all build artifacts"
	@echo "  help           Show this help message"
