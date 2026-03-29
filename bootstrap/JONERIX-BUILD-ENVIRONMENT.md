# jonerix Build Environment Reference

This documents all fixups required to build packages from source on
jonerix-develop. These are necessary because jonerix uses a merged-usr
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

# cc symlink (many configure scripts look for 'cc')
ln -sf clang /bin/cc
```

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

## 5. libstdc++ Path

LLVM shared libs need libstdc++. It lives at `/lib/libstdc++.so.6`
but some build systems (nodejs gyp) look at `/usr/lib/`.

```sh
mkdir -p /usr/lib 2>/dev/null
ln -sf /lib/libstdc++.so.6 /usr/lib/libstdc++.so
ln -sf /lib/libstdc++.so.6 /usr/lib/libstdc++.so.6
```

## 6. Missing POSIX Tools

toybox provides most POSIX tools but some are missing or rejected
by autoconf. These come from the Alpine overlay in Dockerfile.develop:

| Tool    | Source        | License     | Notes                          |
|---------|---------------|-------------|--------------------------------|
| grep    | Alpine grep   | GPL-3.0*    | Build-time only, not in runtime|
| awk     | mawk          | BSD         | Permissive                     |
| m4      | Alpine m4     | GPL-3.0*    | Build-time only                |
| bash    | Alpine bash   | GPL-3.0*    | Build-time only (toybox genconfig.sh) |
| tar     | bsdtar        | Apache-2.0  | Replaces toybox tar (symlink handling) |
| yacc    | byacc         | Public Domain | Replaces GPL bison           |
| ninja   | samurai       | Apache-2.0  | ninja-compatible build tool    |
| make    | bmake         | BSD         | GNU make compat via gnu-compat.patch |

*GPL tools are ONLY in the build environment overlay, never in the runtime image.

## 7. Build System Specifics

### autoconf/configure packages
```sh
# Most just need CC and the environment above
CC=clang ./configure --prefix=/
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
# Needs bash for genconfig.sh
ln -sf bash /bin/bash 2>/dev/null || ln -sf sh /bin/bash
make defconfig
make CC=clang -j$(nproc)
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
- **nodejs**: Python subprocess can't find clang (PATH not propagated).
  Needs explicit PATH in recipe configure step.
- **toybox tar**: Can't handle symlinks in archives. Use bsdtar instead.
- **Large Rust builds**: Disable LTO, limit codegen-units. 4GB+ RAM needed.
- **C++ packages**: Need libstdc++ symlink at /usr/lib/.
