# Packaging Guide

This document explains how to create packages for jonerix using the jpkg package manager, including the recipe format, build process, and repository management.

## Package Format

A `.jpkg` file is a zstd-compressed tarball with a prepended metadata header:

```
+------------------------+
| PKG magic (8 bytes)    |  "JPKG\x00\x01\x00\x00"
| Header length (4 bytes)|
| PKG metadata (TOML)    |
| -------------------------
| zstd-compressed tar    |  (the actual files)
+------------------------+
```

Target package sizes:
- Most packages: 50 KB - 2 MB
- LLVM/Clang: ~200 MB (the exception)

## Build Recipe Format

Each package lives in its own directory under `packages/core/` with a standardized `Makefile`:

```
packages/core/<package-name>/
  Makefile        -- build recipe (required)
  patches/        -- patches to apply (optional)
  <name>.config   -- configuration files (optional)
  files/          -- additional files to install (optional)
```

### Makefile Structure

```makefile
# packages/core/toybox/Makefile
PKG_NAME     = toybox
PKG_VERSION  = 0.8.11
PKG_LICENSE  = 0BSD
PKG_SOURCE   = https://github.com/landley/toybox/archive/$(PKG_VERSION).tar.gz
PKG_SHA256   = <sha256-of-source-tarball>

# Optional fields
PKG_DEPENDS  = musl
PKG_BDEPENDS = clang samurai
PKG_DESC     = BSD-licensed replacement for BusyBox

include ../../rules.mk

configure:
	cp $(PKG_DIR)/toybox.config $(SRC_DIR)/.config

build:
	$(MAKE) -C $(SRC_DIR) CC="$(CC)" CFLAGS="$(CFLAGS)" LDFLAGS="$(LDFLAGS)"

install:
	$(MAKE) -C $(SRC_DIR) PREFIX=$(DESTDIR) install
```

### Required Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `PKG_NAME` | Package name (lowercase, alphanumeric + hyphens) | `toybox` |
| `PKG_VERSION` | Upstream version | `0.8.11` |
| `PKG_LICENSE` | SPDX license identifier | `0BSD` |
| `PKG_SOURCE` | URL to source tarball | `https://...` |
| `PKG_SHA256` | SHA256 hash of the source tarball | `abc123...` |

### Optional Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `PKG_DEPENDS` | Runtime dependencies (space-separated) | (empty) |
| `PKG_BDEPENDS` | Build-time dependencies | (empty) |
| `PKG_DESC` | Short description | (empty) |
| `PKG_SUBDIR` | Subdirectory inside extracted tarball | `$(PKG_NAME)-$(PKG_VERSION)` |
| `PKG_BUILD_STYLE` | Build system type: `cmake`, `meson`, `configure`, `make` | `make` |

### Build Targets

Your Makefile should implement these targets (all optional -- `rules.mk` provides defaults):

| Target | Purpose | When Called |
|--------|---------|-------------|
| `configure` | Run configure scripts, generate build files | After extract + patch |
| `build` | Compile the package | After configure |
| `install` | Install to `$(DESTDIR)` | After build |
| `check` | Run test suite | After build (optional) |

### Available Variables from rules.mk

| Variable | Value | Description |
|----------|-------|-------------|
| `$(CC)` | `clang` | C compiler |
| `$(CXX)` | `clang++` | C++ compiler |
| `$(LD)` | `ld.lld` | Linker |
| `$(AR)` | `llvm-ar` | Archiver |
| `$(RANLIB)` | `llvm-ranlib` | Ranlib |
| `$(STRIP)` | `llvm-strip` | Strip |
| `$(CFLAGS)` | `-Os -pipe -fstack-protector-strong ...` | C compiler flags |
| `$(LDFLAGS)` | `-Wl,-z,relro,-z,now -pie` | Linker flags |
| `$(DESTDIR)` | `/jonerix-sysroot` | Installation prefix |
| `$(SRC_DIR)` | `/jonerix-build/<pkg>-<ver>/` | Extracted source directory |
| `$(PKG_DIR)` | Path to the recipe directory | For accessing patches, configs |

### Available Targets from rules.mk

| Target | Description |
|--------|-------------|
| `fetch` | Download source tarball + verify SHA256 |
| `extract` | Extract tarball to `$(SRC_DIR)` |
| `patch` | Apply all patches from `patches/` directory |
| `clean` | Remove build artifacts |
| `package` | Create `.jpkg` file from installed files |

## License Gate

`rules.mk` enforces the licensing policy automatically. If `PKG_LICENSE` contains `GPL`, `LGPL`, or `AGPL`, the build aborts immediately:

```makefile
# From rules.mk:
FORBIDDEN_LICENSES = GPL LGPL AGPL
$(foreach lic,$(FORBIDDEN_LICENSES),\
  $(if $(findstring $(lic),$(PKG_LICENSE)),\
    $(error BLOCKED: $(PKG_NAME) is $(PKG_LICENSE) -- not permitted in jonerix)))
```

This is a hard block. There are no overrides. The only accepted exception is the Linux kernel, which has special handling in `packages/core/linux/Makefile`.

## Package Metadata (PKG)

The PKG metadata inside a `.jpkg` file uses TOML format:

```toml
[package]
name = "toybox"
version = "0.8.11"
license = "0BSD"
description = "BSD-licensed replacement for BusyBox"
arch = "x86_64"
maintainer = "Jon-Erik G. Storm, Inc. DBA Lava Goat Software"
url = "https://github.com/landley/toybox"

[depends]
runtime = ["musl"]
build = ["clang", "samurai"]

[files]
sha256 = "abc123..."
size = 245760
install_size = 524288
file_count = 42
```

## Writing a New Recipe

### Step-by-Step

1. **Create the directory**:
   ```sh
   mkdir -p packages/core/mypackage
   ```

2. **Verify the license**: Before writing a single line, confirm the upstream project uses a permissive license. Check:
   - The `LICENSE` or `COPYING` file in the source
   - SPDX identifier on the project's website
   - Run `scripts/license-audit.sh` after adding the recipe

3. **Download and hash the source**:
   ```sh
   curl -LO https://example.com/mypackage-1.0.tar.gz
   sha256sum mypackage-1.0.tar.gz
   ```

4. **Write the Makefile**:
   ```makefile
   PKG_NAME     = mypackage
   PKG_VERSION  = 1.0
   PKG_LICENSE  = MIT
   PKG_SOURCE   = https://example.com/$(PKG_NAME)-$(PKG_VERSION).tar.gz
   PKG_SHA256   = <hash-from-step-3>
   PKG_DEPENDS  = musl libressl
   PKG_BDEPENDS = clang samurai
   PKG_DESC     = My awesome package

   include ../../rules.mk

   configure:
   	cd $(SRC_DIR) && cmake -G Ninja \
   		-DCMAKE_C_COMPILER=$(CC) \
   		-DCMAKE_INSTALL_PREFIX=/ \
   		-DCMAKE_BUILD_TYPE=Release \
   		-B build

   build:
   	cmake --build $(SRC_DIR)/build

   install:
   	DESTDIR=$(DESTDIR) cmake --install $(SRC_DIR)/build
   ```

5. **Add patches** (if needed):
   ```sh
   mkdir -p packages/core/mypackage/patches
   # Patches are applied in alphabetical order
   # Name them: 001-fix-something.patch, 002-add-feature.patch
   ```

6. **Build and test**:
   ```sh
   cd packages/core/mypackage
   make fetch extract patch configure build install
   ```

7. **Run the license audit**:
   ```sh
   sh scripts/license-audit.sh --recipes --verbose
   ```

### Common Build System Patterns

#### CMake Projects

```makefile
configure:
	cd $(SRC_DIR) && cmake -G Ninja \
		-DCMAKE_C_COMPILER=$(CC) \
		-DCMAKE_C_FLAGS="$(CFLAGS)" \
		-DCMAKE_EXE_LINKER_FLAGS="$(LDFLAGS)" \
		-DCMAKE_INSTALL_PREFIX=/ \
		-DCMAKE_BUILD_TYPE=MinSizeRel \
		-B build

build:
	cmake --build $(SRC_DIR)/build -- -j$$(nproc)

install:
	DESTDIR=$(DESTDIR) cmake --install $(SRC_DIR)/build
```

#### Autoconf Projects

```makefile
configure:
	cd $(SRC_DIR) && ./configure \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)" \
		--prefix=/ \
		--host=$(TARGET_TRIPLE)

build:
	$(MAKE) -C $(SRC_DIR) -j$$(nproc)

install:
	$(MAKE) -C $(SRC_DIR) DESTDIR=$(DESTDIR) install
```

#### Simple Makefile Projects

```makefile
build:
	$(MAKE) -C $(SRC_DIR) \
		CC="$(CC)" \
		CFLAGS="$(CFLAGS)" \
		LDFLAGS="$(LDFLAGS)" \
		-j$$(nproc)

install:
	$(MAKE) -C $(SRC_DIR) PREFIX=/ DESTDIR=$(DESTDIR) install
```

## Repository Layout

A jpkg repository is a static HTTPS directory. No database server required.

```
https://pkg.jonerix.org/v1/x86_64/
  INDEX.zst          -- Signed manifest (all packages + versions + hashes)
  INDEX.zst.sig      -- Ed25519 signature
  toybox-0.8.11.jpkg
  mksh-59c.jpkg
  openrc-0.54.jpkg
  ...
```

### INDEX Format

The INDEX file is a zstd-compressed text file with one package per line:

```
toybox 0.8.11 0BSD abc123... 245760 musl
mksh 59c MirOS def456... 189440 musl
openrc 0.54 BSD-2-Clause ghi789... 327680 musl toybox
```

Fields: `name version license sha256 size dependencies...`

### Signing

The INDEX and individual packages are signed with Ed25519. The distribution's public key is compiled into jpkg at build time.

```sh
# Generate a signing key pair
jpkg keygen /etc/jpkg/signing.key

# Sign the repository INDEX
jpkg sign /etc/jpkg/signing.key INDEX.zst

# Verify a signature
jpkg verify INDEX.zst INDEX.zst.sig
```

### Hosting a Repository

Any static file server works:

```sh
# Using nginx
server {
    listen 443 ssl;
    server_name pkg.jonerix.org;
    root /srv/jpkg/v1;
    autoindex on;
}

# Using S3
aws s3 sync ./repo/ s3://pkg.jonerix.org/v1/x86_64/

# Using GitHub Releases
# Upload .jpkg files as release assets
```

## jpkg Commands

```sh
jpkg update                  # Fetch INDEX from mirrors
jpkg install <pkg>           # Install package + dependencies
jpkg remove <pkg>            # Remove package
jpkg upgrade                 # Upgrade all installed packages
jpkg search <query>          # Search package names/descriptions
jpkg info <pkg>              # Show package metadata
jpkg build <recipe-dir>      # Build package from source recipe
jpkg build-world             # Rebuild entire system from source
jpkg verify                  # Check installed files against manifests
jpkg license-audit           # Verify all installed packages are permissive
```

## Accepted Licenses

Packages must use one of these licenses to be included in jonerix:

| License | SPDX Identifier |
|---------|-----------------|
| MIT License | `MIT` |
| BSD 2-Clause | `BSD-2-Clause` |
| BSD 3-Clause | `BSD-3-Clause` |
| ISC License | `ISC` |
| Apache License 2.0 | `Apache-2.0` |
| Zero-Clause BSD | `0BSD` |
| Creative Commons Zero | `CC0-1.0` |
| Public Domain | `public-domain` |
| zlib License | `Zlib` |
| curl License | `curl` |
| Unlicense | `Unlicense` |

**Explicitly forbidden**: GPL, LGPL, AGPL, SSPL, EUPL, or any copyleft license.

**Sole exception**: The Linux kernel (GPLv2), documented in DESIGN.md.
