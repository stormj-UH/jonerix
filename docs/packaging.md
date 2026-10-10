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

Each package lives in its own directory under `packages/{core,develop,extra}/` with a `recipe.toml`:

```
packages/<category>/<package-name>/
  recipe.toml     -- build recipe (required)
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

## Config files

Since jpkg 2.2.11, a **config file** is any regular file a package ships
under `etc/`, except under `etc/init.d/` and `etc/cron.d/` (OpenRC and
snooze-crond run every file there: that is code, always replaced), and
except the trust store, `etc/ssl/certs/` and `etc/ssl/cert.pem` (Go
programs trust every file in `certs/`, and CA removals must reach every
host, so both are replaced on every update as before). Recipes declare
nothing: the class comes from the path, worked out on the installing host,
and no archive, INDEX or manifest format changed.

jpkg overwrites or deletes a file or symlink at a config path only when it
is **pristine** — missing, or a regular file or symlink jpkg recorded there.
Anything else is yours: never opened, overwritten or deleted. The most jpkg
does to it is move it aside to `<file>.jpkg-save`, when a package changes
the kind of object at that path (see the table). As before 2.2.11, a
symlinked parent directory is followed (move `/etc/foo` elsewhere and link
it back, and package files are written, and on removal pristine ones
deleted, through the link), and a directory a package ships is merged into
an existing directory, or written through your symlink when it leads to a
directory (under `--root`, an absolute link, or one that climbs out of the
root, is not followed: yours is moved to `.jpkg-save`, and another
package's makes the install stop before anything is written, unless the
new package replaces it; then the link is checked as below). Where a package now ships a directory
in place of a link its previous version shipped, the link is removed and
the directory created -- unless the directory the link led to holds
anything but that package's own unchanged files (looking through further
links below it too), in which case the upgrade is refused before anything is
written, naming them.

| Situation | What jpkg does |
|---|---|
| file unchanged, package ships a new version | replaces it |
| file changed locally, package's copy unchanged | keeps yours, says nothing new |
| file changed locally (or there before the package was installed), package's copy differs | keeps yours, writes the packaged version to `<file>.jpkg-new`, warns |
| a symlink, directory, FIFO or device you put where a package ships a file | same: never opened or replaced; the packaged version goes to `<file>.jpkg-new` |
| `<file>.jpkg-new` already there | replaced only if it is an untouched earlier packaged copy; if you edited it, it is left alone and the new packaged version is not written (once you clear it, the next version that changes the file offers its copy) |
| you deleted the file | puts the package's copy back (empty the file to disable it) |
| an upgrade stops shipping a changed file | leaves it in place, owned by no package, warns |
| `jpkg remove` of a package with a changed file | same: left in place, unowned |
| a symlink the package ships, which you pointed elsewhere or replaced with a file or directory | keeps yours; the package's link is not written |
| a package turns a config file (its own or another package's) into a symlink or a directory, and the file was changed | moves the changed file to `<file>.jpkg-save` first; refuses the install rather than overwrite an existing `.jpkg-save` |
| a package puts a directory where your own FIFO, file, or symlink to something that is not a directory sits | moves that to `<file>.jpkg-save` first |
| a package turns a directory into a symlink and you changed a config file in it | refuses the upgrade, naming your file; move it out of the directory (or take the packaged copy back) and retry |
| a package turns its symlink (or that of a package it replaces) into a directory, and the directory the link led to holds your files or another package's | same: refused, naming them |

The installed manifest records what the package shipped, also for a kept
file, so the next upgrade compares against that. Files and links at config
paths are written last, after everything else, each under a temporary
name (`<file>.jpkg-tmp`) and renamed into place, and every decision is
checked again just before that: an edit made earlier in an upgrade is
kept, but one made while those last writes run can be lost, so do not edit
a package's config files while it is being upgraded. A failed or
interrupted install never leaves a half-written config file. If it fails
before that last step, the old copies are left; if it fails during it,
some files may already hold the new version, which jpkg did not record:
the next version keeps them as changed and offers its own copy, and
reinstalling the same version repairs them.

`jpkg verify` does not count a changed config file (or a changed symlink
at a config path) as a failure; a missing one still is. `jpkg verify
<package>` lists each one and any pending `.jpkg-new`; `jpkg verify` with
no arguments shows a count per package. After an upgrade:

```sh
find /etc -name '*.jpkg-new' -o -name '*.jpkg-save'   # waiting for you
```

Once you have merged an offer, delete it. To take the packaged version as
it is: `mv /etc/foo.conf.jpkg-new /etc/foo.conf`. jpkg deletes an offer by
itself only at a later install or upgrade of the package, when your file
is identical to a packaged version, or when the package stops shipping the
file as a config file, or is removed.

`/etc/skel` is config too: after you edit a skeleton file and a package
updates it, `useradd -m` also copies the `.jpkg-new` into new homes until
you delete it.

jonerix has no local CA store: the trust store is the package's, and an
edit inside `/etc/ssl/certs/` or to `/etc/ssl/cert.pem` is undone by the
next update. To trust a private CA, point each program at it. Go programs:
set `SSL_CERT_FILE` to a file holding only your CA (Go still reads
`/etc/ssl/certs/`, so the package's CA updates keep landing; do not also set
`SSL_CERT_DIR`). curl: give `--cacert` (or `SSL_CERT_FILE`, which it also
reads) a file holding the bundle plus your CA, and rebuild that file after
every ca-certificates update (`cat /etc/ssl/certs/ca-certificates.crt
corp.pem > /etc/local-ca.pem`), or it keeps trusting CAs the package has
removed; a single `SSL_CERT_FILE` in the environment cannot suit both. A
program using LibreSSL's libssl or libtls (python3 among them) ignores
both variables and needs its own CA option (`cafile=`, …).

### Upgrading to 2.2.11

The upgrade that installs 2.2.11 is still run by the jpkg it replaces,
which overwrites edited files as before. Upgrade jpkg on its own first:

```sh
jpkg update && jpkg install jpkg && jpkg upgrade jpkg && jpkg upgrade
```

(On hosts made from the minimal, core or router images jpkg is not a
registered package, so `jpkg upgrade` alone never touches it: `jpkg install
jpkg` registers it there and does nothing where it is registered; `jpkg
upgrade jpkg` then upgrades jpkg and nothing else. Do not use `jpkg install
--force jpkg`: it makes the old jpkg reinstall musl, toybox and mksh too,
overwriting their edited config files.)

Hosts installed from an image or WSL rootfs made before 2.2.11 can hold
the image's copy of a packaged file (`/etc/zshrc` on WSL and on hosts from
the minimal image, until zsh is upgraded). 2.2.11 keeps such a copy as a
local change from then on, and offers the package's version as `.jpkg-new`
whenever the package changes it; take it with `mv` as above. fastfetch
2.36.1-r2 ships the banner those images copied in, so a host that holds
it adopts r2 without an offer.

### For recipe authors

- Ship a new package's defaults straight into `etc/`.
- Never edit, from a hook, a file you ship under `etc/`: every host then
  holds bytes jpkg did not record, so jpkg keeps the file as changed
  locally and your updates to it stop landing. Bake the edit into the
  shipped copy. Do not seed or edit other packages' `etc/` files either.
- Do not ship defaults into a directory whose reader loads every file
  (any `foo.d/` read without a name filter): a `.jpkg-new` there would be
  read too. (init.d, cron.d and the trust store are not config, so they
  never get one.)
- Prefer a drop-in directory (`foo.d/*.conf`) for anything an admin is
  likely to tune: their file stays theirs and yours stays pristine.
- Recipes that ship a default outside `etc/` and seed `/etc` from a hook
  (dhcpcd's `share/dhcpcd/dhcpcd.conf`, dropbear's
  `share/dropbear/conf.d/sshd`) must **not** move it into `etc/` yet. An
  older jpkg overwrites the admin's file the first time the package ships
  it, and `jpkg upgrade` does not upgrade jpkg first, so a host can install
  such a revision with 2.2.10 even after 2.2.11 is out. These moves wait
  until every host runs 2.2.11 or later and no image older than 2.2.11 is
  still being installed. The first revision that ships the file in `etc/`
  must then ship exactly the bytes an untouched host holds at that point,
  after every hook that edits it (another package's too: unbound appends to
  `/etc/dhcpcd.conf`), so untouched hosts adopt it silently. A host that
  holds anything else, an older default included, keeps its file as a
  local change from then on, with an offer whenever the package changes
  it. (unbound and jcarp already ship `etc/…/*.conf.default` and seed the
  live file from it; that live file is unowned and stays the admin's.)
- Hooks run by 2.2.11 or later see `JPKG_CONFFILES=1`, for a hook that has
  to know whether jpkg protects config files.

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
| Perl's Artistic License | `Artistic-1.0-Perl` |
| Artistic License 2.0 | `Artistic-2.0` |
| Python Software Foundation | `PSF-2.0` |
| FreeType, HPND, Unicode, libpng | `FTL`, `HPND`, `Unicode-DFS-2016`, `Unicode-3.0`, `libpng-2.0` |

The authoritative list is `PERMISSIVE_LICENSES` in
`packages/core/jpkg/src/util.rs`; `jpkg build` refuses anything else. SPDX
`OR` needs one permissive alternative, `AND` needs all.

**Explicitly forbidden**: GPL, LGPL, AGPL, SSPL, EUPL, MPL, or any other
copyleft license, unless an `OR` alternative is permissive.

**Per-package exceptions**: the Linux kernel (GPLv2) is built out of band,
and `ca-certificates` may carry MPL-2.0 (the CA bundle is data). Both are
documented in DESIGN.md.

## Publishing

Only `main` publishes to the rolling `packages` release:
`.github/workflows/publish-packages.yml` and `build-llvm-chain.yml` refuse
other refs, `scripts/local-build-*.sh upload` refuses other branches, and
`scripts/gen-index.sh` fails when a package it would index is newer than its
recipe. Run `sh scripts/check-index-drift.sh` to compare the live INDEX with
the recipes in your tree.

Source mirrors (`source-<pkg>-v<version>` releases) are created by hand.
Create them with `--latest=false` so they never become the repository's
"Latest" release:

```sh
gh release create source-foo-v1.2.3 sources/foo-1.2.3.tar.gz \
  --repo stormj-UH/jonerix --title "foo 1.2.3 source" --latest=false
```
