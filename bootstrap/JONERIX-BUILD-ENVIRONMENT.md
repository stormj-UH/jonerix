# jonerix Build Environment Reference

This documents all fixups required to build packages from source on
jonerix:builder. These are necessary because jonerix uses a merged-usr
layout, musl libc, and permissive-only tools (clang, not gcc).

## 1. Compiler & Toolchain

```sh
# Default compiler — NEVER use gcc
export CC=clang
export CXX=clang++
export LD=ld.lld
export AR=llvm-ar
export NM=llvm-nm
export RANLIB=llvm-ranlib

# cc/c++ symlinks (many configure scripts look for 'cc')
# Already set in builder image:
#   cc -> clang, c++ -> clang++, ld -> ld.lld
#   ar -> llvm-ar, nm -> llvm-nm, ranlib -> llvm-ranlib
#   make -> bmake, ninja -> samu
```

The builder image ships clang wrapper scripts at `/bin/clang` and
`/bin/clang++` that pass `--config=/etc/clang/<triple>.cfg` to load
`--rtlib=compiler-rt --unwindlib=libunwind -fuse-ld=lld` automatically.

## 2. Include & Library Paths (merged-usr layout)

jonerix has headers at `/include/` not `/usr/include/`. The merged-usr
symlink (`usr -> /`) means both resolve to the same place, but tools
don't always know that.

```sh
# Use C_INCLUDE_PATH, NOT -I flags (recipes override CFLAGS)
export C_INCLUDE_PATH=/include
export CPLUS_INCLUDE_PATH=/include

# Library search path
export LIBRARY_PATH=/lib

# CRITICAL: Do NOT add /usr/include — Alpine overlay may have
# fortify wrapper headers there that cause #include_next loops
```

## 3. Clang Intrinsics Headers

Clang's resource directory contains intrinsic headers (emmintrin.h,
cpuid.h) AND wrapper headers (inttypes.h, stdint.h). The wrappers
use `#include_next` and MUST NOT be copied to `/include/` — they
shadow musl headers and create circular includes.

```sh
# Clang finds its own resource dir automatically at:
#   /lib/llvm21/lib/clang/21/include/
# Do NOT copy these to /include/!

# If a build needs intrinsics explicitly, add clang's dir:
CLANG_RES=$(clang -print-resource-dir)/include
export CFLAGS="$CFLAGS -I$CLANG_RES"
```

## 4. musl Stub Libraries

musl includes libm/libpthread/etc in libc, but packages expect
separate .so files. Create symlinks:

```sh
LIBC=$(ls /lib/libc.musl-*.so.1 | head -1)
LIBC_NAME=$(basename "$LIBC")
for stub in libm libpthread libcrypt librt libdl libutil libresolv libxnet; do
    ln -sf "$LIBC_NAME" "/lib/${stub}.so"
    ln -sf "$LIBC_NAME" "/lib/${stub}.so.1"
done
ln -sf "$LIBC_NAME" "/lib/libc.so"
```

## 5. C++ Standard Library

The builder ships libc++ (LLVM) as the C++ standard library.
Compatibility symlinks for libstdc++ exist for packages that hardcode it:

```sh
# Already set in builder image:
ln -sf libgcc_s.so.1 /lib/libgcc_s.so
ln -sf libstdc++.so.6 /lib/libstdc++.so

# Empty libssp stub (some builds link -lssp_nonshared)
printf '!<arch>\n' > /lib/libssp_nonshared.a
```

## 6. Build Tools

The builder image includes all build tools needed. No Alpine overlay —
everything is installed via jpkg.

| Tool    | Package     | License       | Notes                          |
|---------|-------------|---------------|--------------------------------|
| make    | bmake       | MIT           | BSD make, symlinked as /bin/make |
| ninja   | samurai     | Apache-2.0    | ninja-compatible, symlinked as /bin/ninja |
| awk     | onetrueawk  | MIT           | One True Awk (Kernighan)       |
| tar     | bsdtar      | BSD-2-Clause  | Replaces toybox tar (symlink handling) |
| yacc    | byacc       | Public Domain | Replaces GPL bison             |
| lex     | flex        | BSD-2-Clause  | Lexer generator                |
| cmake   | cmake       | BSD-3-Clause  | Build system generator         |
| grep    | toybox      | 0BSD          | Pass GREP=/bin/grep for autoconf |
| python  | python3     | PSF-2.0       | Needed by some build systems   |
| perl    | perl        | Artistic-2.0  | Needed by autoconf/OpenSSL     |

**Note on autoconf**: toybox grep is functional but autoconf's grep
detection test fails to recognize it. Pass `GREP=/bin/grep` explicitly
to `./configure` to work around this.

**Note on GNU make**: Some upstream projects (Ruby, hostapd,
wpa_supplicant) use GNU make-specific features (ifdef, $(wildcard),
$(shell)) that bmake cannot handle. These must be built in an Alpine
container with GNU make at build time. bmake works for most other
packages.

## 7. Build System Specifics

### autoconf/configure packages
```sh
# Most just need CC and the environment above
GREP=/bin/grep CC=clang ./configure --prefix=/
make -j$(nproc)
make DESTDIR=$DESTDIR install
```

### cmake packages
```sh
cmake -B build -G Ninja \
    -DCMAKE_INSTALL_PREFIX=/ \
    -DCMAKE_C_COMPILER=clang \
    -DCMAKE_CXX_COMPILER=clang++ \
    -DCMAKE_BUILD_TYPE=Release
cmake --build build -j$(nproc)
DESTDIR=$DESTDIR cmake --install build
```

### Rust packages (cargo)
```sh
# Disable LTO to prevent OOM on link
export CARGO_PROFILE_RELEASE_LTO=false
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
cargo build --release
```

### Go packages
```sh
# Build directly — no GNU make needed
go build -trimpath -ldflags "-s -w" -o binary ./cmd/...
```

### Perl
```sh
# Perl Configure needs explicit paths for merged-usr
./Configure -des \
    -Dprefix=/ \
    -Dcc=clang \
    -Dlibc=/lib/libc.so \
    -Dlibpth='/lib /usr/lib' \
    -Dusrinc='/include' \
    -Uuseshrplib
```

### toybox
```sh
# Needs bash for genconfig.sh and scripts/make.sh
bash scripts/genconfig.sh defconfig
# Enable pending commands (sh, getty, login, etc.)
for opt in SH GETTY LOGIN PASSWD SU; do
    sed -i "s/# CONFIG_${opt} is not set/CONFIG_${opt}=y/" .config
done
CC=clang CFLAGS="-Os -fomit-frame-pointer" bash scripts/make.sh
```

### musl (from source)
```sh
# Build with clang, install to isolated DESTDIR
CC=clang AR=llvm-ar RANLIB=llvm-ranlib \
    ./configure --prefix=/
make -j$(nproc)
make DESTDIR=$DESTDIR install
# IMPORTANT: jpkg --build-jpkg keeps this isolated from the running system
```

## 8. jpkg Build Sandbox

jpkg's `run_build_step()` (cmd_build.c) automatically sets:
- `CC=clang LD=ld.lld AR=llvm-ar NM=llvm-nm RANLIB=llvm-ranlib`
- `C_INCLUDE_PATH=/include`
- `LIBRARY_PATH=/lib`
- `DESTDIR=<isolated temp dir>`
- `CFLAGS="-Os -pipe -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2"`
- `LDFLAGS="-Wl,-z,relro,-z,now -pie"`

These can be overridden by recipe.toml build commands.

## 9. Known Limitations

- **musl from-source**: Works but must use `--build-jpkg` (isolated DESTDIR).
  Installing directly contaminates system headers.
- **toybox tar**: Can't handle symlinks in archives. bsdtar is the default
  (`/bin/tar -> bsdtar`).
- **Large Rust builds**: Disable LTO, limit codegen-units. 4GB+ RAM needed.
- **GNU make projects**: Ruby, hostapd, wpa_supplicant require GNU make
  (not available on jonerix). Build these in Alpine containers.
- **autoconf grep**: Always pass `GREP=/bin/grep` to configure — autoconf
  rejects toybox grep.
