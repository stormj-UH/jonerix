#!/bin/sh
set -e

arch=$(uname -m)
case "$arch" in
    x86_64|aarch64) ;;
    *)
        echo "bootstrap-cmake: unsupported architecture: $arch" >&2
        exit 1
        ;;
esac

src_tar="/workspace/sources/cmake-4.1.0.tar.gz"
work_root="/tmp/bootstrap-cmake-$arch"
src_root="$work_root/src"
prefix_root="$work_root/prefix"
tool_root="$work_root/tools"
bootstrap_log="$work_root/bootstrap.log"
build_log="$work_root/build.log"
install_log="$work_root/install.log"

if [ ! -f "$src_tar" ]; then
    echo "bootstrap-cmake: missing source tarball: $src_tar" >&2
    exit 1
fi

ensure_linux_fs_header_compat() {
    [ "$(uname -s)" = "Linux" ] || return 0
    if [ -f /include/linux/fs.h ] || [ -f /usr/include/linux/fs.h ]; then
        return 0
    fi
    mkdir -p /include/linux /usr/include/linux
    cat > /include/linux/fs.h <<'EOF'
#ifndef JONERIX_COMPAT_LINUX_FS_H
#define JONERIX_COMPAT_LINUX_FS_H

#include <sys/ioctl.h>

#ifndef FS_IOC_GETFLAGS
#define FS_IOC_GETFLAGS _IOR('f', 1, long)
#endif

#ifndef FICLONE
#define FICLONE _IOW(0x94, 9, int)
#endif

#endif
EOF
    if [ ! -f /usr/include/linux/fs.h ]; then
        cp /include/linux/fs.h /usr/include/linux/fs.h
    fi
    echo "bootstrap-cmake: installed minimal linux/fs.h compatibility header" >&2
}

find_tool() {
    for candidate in "$@"; do
        resolved=$(command -v "$candidate" 2>/dev/null || true)
        if [ -n "$resolved" ] && [ -x "$resolved" ]; then
            printf '%s\n' "$resolved"
            return 0
        fi
        for d in /lib/llvm*/bin /usr/lib/llvm*/bin /usr/local/lib/llvm*/bin; do
            [ -d "$d" ] || continue
            if [ -x "$d/$candidate" ]; then
                printf '%s\n' "$d/$candidate"
                return 0
            fi
        done
    done
    return 1
}

emit_tool_diagnostics() {
    echo "bootstrap-cmake: PATH=$PATH" >&2
    for tool in clang clang-21 clang++ clang++-21 cc c++ ld ld.lld ld.lld-21 llvm-ar llvm-ranlib; do
        found=$(command -v "$tool" 2>/dev/null || true)
        [ -n "$found" ] && echo "bootstrap-cmake: $tool -> $found" >&2
    done
    for d in /bin /usr/bin /usr/local/bin /lib/llvm*/bin /usr/lib/llvm*/bin /usr/local/lib/llvm*/bin; do
        [ -d "$d" ] || continue
        echo "bootstrap-cmake: scanned $d" >&2
    done
}

clang_real=$(find_tool clang clang-21 cc clang-20 || true)
clangxx_real=$(find_tool clang++ clang++-21 c++ clang++-20 || true)
clangxx_mode=
if [ -z "$clangxx_real" ] && [ -n "$clang_real" ]; then
    clangxx_real="$clang_real"
    clangxx_mode='--driver-mode=g++'
fi
ld_real=$(find_tool ld.lld ld.lld-21 lld ld || true)
if [ -z "$clang_real" ] || [ -z "$clangxx_real" ]; then
    echo "bootstrap-cmake: compiler toolchain is incomplete" >&2
    emit_tool_diagnostics
    exit 1
fi

if command -v samu >/dev/null 2>&1; then
    generator="Ninja"
    make_cmd=$(command -v samu)
elif command -v ninja >/dev/null 2>&1; then
    generator="Ninja"
    make_cmd=$(command -v ninja)
elif command -v make >/dev/null 2>&1; then
    generator="Unix Makefiles"
    make_cmd=$(command -v make)
else
    echo "bootstrap-cmake: no supported build tool found (samu/ninja/make)" >&2
    exit 1
fi

clang_triple=$("$clang_real" -dumpmachine 2>/dev/null || echo "$arch-jonerix-linux-musl")
clang_cfg="/etc/clang/${clang_triple}.cfg"
if [ ! -f "$clang_cfg" ]; then
    mkdir -p /etc/clang
    printf -- '--rtlib=compiler-rt\n--unwindlib=libunwind\n-fuse-ld=lld\n' > "$clang_cfg"
fi
ensure_linux_fs_header_compat

rm -rf "$work_root"
mkdir -p "$src_root" "$prefix_root" "$tool_root"

cat > "$tool_root/clang" <<EOF
#!/bin/sh
exec "$clang_real" --config="$clang_cfg" "\$@"
EOF
cat > "$tool_root/clang++" <<EOF
#!/bin/sh
exec "$clangxx_real" $clangxx_mode --config="$clang_cfg" --unwindlib=libunwind -stdlib=libc++ -lc++ -lc++abi "\$@"
EOF
chmod 755 "$tool_root/clang" "$tool_root/clang++"
ln -sf clang "$tool_root/cc"
ln -sf clang++ "$tool_root/c++"
if [ -n "$ld_real" ]; then
    ln -sf "$ld_real" "$tool_root/ld"
    ln -sf "$ld_real" "$tool_root/ld.lld"
fi

if command -v bsdtar >/dev/null 2>&1 && bsdtar --version >/dev/null 2>&1; then
    bsdtar -xf "$src_tar" -C "$src_root"
elif [ -x /bin/toybox ]; then
    /bin/toybox tar -xf "$src_tar" -C "$src_root"
else
    tar -xf "$src_tar" -C "$src_root"
fi

src_dir="$src_root/cmake-4.1.0"
if [ ! -d "$src_dir" ]; then
    echo "bootstrap-cmake: extracted source directory not found" >&2
    exit 1
fi

nproc=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 1)
[ -n "$nproc" ] || nproc=1

cd "$src_dir"
export PATH="$tool_root:/bin:/usr/bin:/usr/local/bin${PATH:+:$PATH}"
export CC=clang
export CXX=clang++
export MAKE="$make_cmd"
export CFLAGS='-Os -pipe -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2 --rtlib=compiler-rt --unwindlib=libunwind -fuse-ld=lld'
export CXXFLAGS='-Os -pipe -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2 --rtlib=compiler-rt --unwindlib=libunwind -fuse-ld=lld -stdlib=libc++'
export LDFLAGS='-Wl,-z,relro,-z,now -pie --rtlib=compiler-rt --unwindlib=libunwind -fuse-ld=lld -stdlib=libc++ -lc++ -lc++abi'

echo "bootstrap-cmake: bootstrapping with $generator via $make_cmd" >&2
echo "bootstrap-cmake: clang=$clang_real clangxx=$clangxx_real ld=${ld_real:-missing}" >&2
if ! ./bootstrap \
  --prefix="$prefix_root" \
  --parallel="$nproc" \
  --generator="$generator" \
  -- \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_LIBDIR=lib \
  -DCMAKE_USE_OPENSSL=OFF \
  -DBUILD_CursesDialog=OFF >"$bootstrap_log" 2>&1; then
    echo "bootstrap-cmake: upstream bootstrap failed" >&2
    echo "bootstrap-cmake: tail of bootstrap.log follows" >&2
    tail -n 200 "$bootstrap_log" >&2 || true
    echo "bootstrap-cmake: recent error lines from bootstrap.log" >&2
    grep -En 'error:|fatal error:|undefined reference|ld\.lld|collect2|cannot find|No such file|undefined symbol|library not found|ninja:' "$bootstrap_log" | tail -n 120 >&2 || true
    if [ -f "$src_dir/Bootstrap.cmk/cmake_bootstrap.log" ]; then
        echo "bootstrap-cmake: compiler probe log follows" >&2
        tail -n 120 "$src_dir/Bootstrap.cmk/cmake_bootstrap.log" >&2 || true
    fi
    exit 1
fi

if ! "$make_cmd" -j"$nproc" >"$build_log" 2>&1; then
    echo "bootstrap-cmake: build failed" >&2
    tail -n 200 "$build_log" >&2
    exit 1
fi

if ! "$make_cmd" install >"$install_log" 2>&1; then
    echo "bootstrap-cmake: install failed" >&2
    tail -n 200 "$install_log" >&2
    exit 1
fi

if [ ! -x "$prefix_root/bin/cmake" ]; then
    echo "bootstrap-cmake: expected bootstrap binary missing at $prefix_root/bin/cmake" >&2
    exit 1
fi

printf '%s\n' "$prefix_root/bin/cmake"
