# Bootstrapping jonerix

This document explains how to bootstrap jonerix from an Alpine Linux host. The bootstrap process builds a complete, self-contained Linux distribution where every userland component is permissively licensed.

## Prerequisites

### Hardware Requirements

- x86_64 or aarch64 system (or VM)
- At least 4 GB RAM (8 GB recommended for LLVM builds)
- At least 20 GB free disk space
- Internet connection (for downloading source tarballs)

### Software Requirements

You need either:

1. **Docker** (recommended for reproducibility):
   ```sh
   docker pull alpine:latest
   ```

2. **A running Alpine Linux installation** (native or VM)

3. **Any Linux distribution** with `debootstrap` or `chroot` capability

## Overview

The bootstrap has four stages:

```
Stage 0 (Alpine host)     Stage 1 (cross-build)     Stage 2 (rootfs)          Stage 3 (verify)
  Install build deps  -->   Build all components  -->  Assemble clean FS  -->   Self-rebuild
  GPL tools are OK          Mixed toolchain           No GPL at all            Bit-for-bit check
```

**Key principle**: GPL tools (GCC, BusyBox, apk-tools) are used as scaffolding during Stages 0 and 1. They never appear in the final Stage 2 image. The Stage 3 rebuild from within jonerix proves the system is fully self-hosting.

## Stage 0: Alpine Build Host

Stage 0 sets up the Alpine-based build environment. Nothing from this stage enters the final image.

### Using Docker

```sh
docker run -it --privileged -v $(pwd):/jonerix alpine:latest sh
cd /jonerix
sh bootstrap/stage0.sh
```

### Manual Setup

On a running Alpine system:

```sh
# Install build toolchain
apk add clang lld llvm-dev musl-dev cmake samurai git curl patch

# Install additional build dependencies
apk add perl linux-headers flex bison bc

# Install packaging tools
apk add tar gzip zstd xz

# Install disk image tools (for mkimage.sh)
apk add dosfstools e2fsprogs sgdisk losetup
```

### What stage0.sh Does

1. Updates the Alpine package index
2. Installs the Clang/LLVM toolchain and all build dependencies
3. Creates the build directory structure (`/jonerix-sysroot`, `/jonerix-build`)
4. Validates that all required tools are present

## Stage 1: Cross-Compile the Permissive World

Stage 1 compiles every jonerix component from source using the Alpine host's tools. The output goes into a staging sysroot at `/jonerix-sysroot`.

### Build Order

Dependencies are built first. The order matters:

| # | Package | License | Why This Order |
|---|---------|---------|----------------|
| 1 | musl | MIT | C library -- everything links against it |
| 2 | zstd, lz4 | BSD | Compression -- needed by jpkg and kernel |
| 3 | LibreSSL | ISC | TLS -- needed by curl, dropbear |
| 4 | toybox | 0BSD | Coreutils -- ls, cp, cat, grep, sed, awk, tar |
| 5 | mksh | ISC-like | Shell |
| 6 | samurai | Apache-2.0 | Build tool (ninja-compatible) |
| 7 | LLVM/Clang/lld | Apache-2.0 | Compiler + linker (the longest build, ~45 min) |
| 8 | OpenRC | BSD-2-Clause | Init system |
| 9 | dropbear | MIT | SSH server |
| 10 | curl | MIT-like | HTTP client |
| 11 | dhcpcd | BSD-2-Clause | DHCP |
| 12 | unbound | BSD-3-Clause | DNS resolver |
| 13 | doas | ISC | Privilege escalation |
| 14 | socklog | BSD-3-Clause | Logging |
| 15 | snooze | CC0 | Cron |
| 16 | mandoc | ISC | Man pages |
| 17 | ifupdown-ng | ISC | Network configuration |
| 18 | jpkg | MIT | Package manager |
| 19 | pigz | Zlib | Parallel gzip |
| 20 | nvi | BSD | Text editor |

### Compiler Flags

All packages are compiled with hardening flags:

```sh
CC=clang
LD=ld.lld
CFLAGS="-Os -pipe -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2"
LDFLAGS="-Wl,-z,relro,-z,now -pie"
```

These flags enable:
- **`-Os`**: Optimize for size (critical for a small distribution)
- **`-fstack-protector-strong`**: Stack buffer overflow detection
- **`-fPIE` / `-pie`**: Position-independent executables for ASLR
- **`-D_FORTIFY_SOURCE=2`**: Runtime buffer overflow checks
- **`-Wl,-z,relro,-z,now`**: Full RELRO (read-only GOT after load)

### Running Stage 1

```sh
sh bootstrap/stage1.sh
```

This will take 1-3 hours depending on hardware (LLVM is the bottleneck). The script is resumable -- if interrupted, re-running it will skip already-built packages.

### Verifying Stage 1

After completion, verify the sysroot:

```sh
# All binaries should be statically linked against musl
file /jonerix-sysroot/bin/* | head -20

# No glibc references
ldd /jonerix-sysroot/bin/toybox  # Should show musl or "not a dynamic executable"

# Check for GPL contamination
find /jonerix-sysroot -name "COPYING" -exec grep -l "GNU General Public" {} \;
```

## Stage 2: Assemble the Root Filesystem

Stage 2 takes the compiled artifacts from Stage 1 and assembles them into a clean root filesystem. No Alpine bits carry over.

### What stage2.sh Does

1. Creates the FHS directory structure (merged `/usr`)
2. Copies binaries and libraries from `/jonerix-sysroot`
3. Installs default configurations from `config/defaults/`
4. Installs OpenRC service scripts from `config/openrc/`
5. Compiles and installs the Linux kernel with EFISTUB
6. Sets file permissions and ownership
7. Generates the jpkg package database
8. Produces output artifacts

### Running Stage 2

```sh
sh bootstrap/stage2.sh
```

### Output Artifacts

Stage 2 produces three artifacts in `/jonerix-output/`:

| File | Description |
|------|-------------|
| `jonerix-rootfs-<version>.tar.zst` | Compressed rootfs tarball |
| `jonerix-<version>.img` | Bootable disk image (GPT + ESP + ext4) |
| `jonerix-<version>-oci.tar` | OCI container image |

### Verifying Stage 2

```sh
# Check that no GPL binaries leaked into the rootfs
find /jonerix-output/rootfs -type f -exec file {} \; | grep -v "musl" | grep "ELF"

# All ELF binaries should link against musl, not glibc
for f in /jonerix-output/rootfs/bin/*; do
    [ -f "$f" ] || continue
    if file "$f" | grep -q "ELF"; then
        if readelf -d "$f" 2>/dev/null | grep -q "glibc"; then
            echo "FAIL: $f links against glibc"
        fi
    fi
done

# Run license audit
sh scripts/license-audit.sh --rootfs /jonerix-output/rootfs --verbose

# Run size report
sh scripts/size-report.sh --target server /jonerix-output/rootfs
```

## Stage 3: Self-Hosting Verification

Stage 3 is the ultimate proof that jonerix is fully self-contained. It boots the Stage 2 image and rebuilds the entire system from source using only jonerix's own tools.

### Running Stage 3

```sh
# Boot the Stage 2 image in QEMU
qemu-system-x86_64 \
    -bios /usr/share/OVMF/OVMF_CODE.fd \
    -drive file=/jonerix-output/jonerix-0.1.0.img,format=raw \
    -m 4G -smp 4 \
    -nographic

# Inside jonerix:
jpkg build-world

# Compare output to the Stage 2 rootfs
# If bit-for-bit identical, the bootstrap is proven.
```

### What Self-Hosting Proves

- jonerix can compile all of its own components (including the Clang/LLVM compiler)
- No GPL tools are required at runtime
- The build is reproducible

## Troubleshooting

### LLVM Build Fails with OOM

LLVM requires significant memory to compile. Solutions:

```sh
# Add swap
dd if=/dev/zero of=/swapfile bs=1M count=4096
mkswap /swapfile
swapon /swapfile

# Or reduce parallelism
export MAKEFLAGS="-j2"
```

### Stage 1 Takes Too Long

The LLVM build is the bottleneck (~45 minutes on a fast machine). Use the CI cache to skip rebuilding if nothing changed:

```sh
# The build system caches completed packages in /jonerix-build/stamps/
ls /jonerix-build/stamps/
```

### EFI Boot Fails

Ensure you're using UEFI firmware with QEMU:

```sh
# Install OVMF
apk add ovmf  # or: apt install ovmf

# Boot with UEFI
qemu-system-x86_64 -bios /usr/share/OVMF/OVMF_CODE.fd -drive file=jonerix.img,format=raw
```

### License Audit Finds Violations

If `license-audit.sh` reports violations, investigate:

1. Check if the package is a build-time dependency only (OK if it doesn't ship in the rootfs)
2. Check if there's a permissive alternative
3. If the package is essential and no alternative exists, document it as an exception in DESIGN.md

## Quick Reference

```sh
# Full bootstrap from scratch
sh bootstrap/stage0.sh
sh bootstrap/stage1.sh
sh bootstrap/stage2.sh

# Create disk image
sh image/mkimage.sh /jonerix-output/jonerix-rootfs-0.1.0.tar.zst

# Create OCI image
sh image/oci.sh /jonerix-output/jonerix-rootfs-0.1.0.tar.zst

# Create cloud images
sh image/cloud/aws-ami.sh jonerix-0.1.0.img
sh image/cloud/gcp-image.sh jonerix-0.1.0.img

# Audit
sh scripts/license-audit.sh --verbose
sh scripts/size-report.sh --target server /jonerix-output/rootfs
```
