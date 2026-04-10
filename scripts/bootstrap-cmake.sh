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

if [ ! -f "$src_tar" ]; then
    echo "bootstrap-cmake: missing source tarball: $src_tar" >&2
    exit 1
fi

clang_real=$(command -v clang-21 2>/dev/null || command -v clang 2>/dev/null || true)
clangxx_real=$(command -v clang++-21 2>/dev/null || command -v clang++ 2>/dev/null || true)
lld_real=$(command -v ld.lld 2>/dev/null || true)
if [ -z "$clang_real" ] || [ -z "$clangxx_real" ] || [ -z "$lld_real" ]; then
    echo "bootstrap-cmake: clang/clang++/ld.lld are required" >&2
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

rm -rf "$work_root"
mkdir -p "$src_root" "$prefix_root" "$tool_root"

cat > "$tool_root/clang" <<EOF
#!/bin/sh
exec "$clang_real" --config="$clang_cfg" "\$@"
EOF
cat > "$tool_root/clang++" <<EOF
#!/bin/sh
exec "$clangxx_real" --config="$clang_cfg" --unwindlib=libunwind -stdlib=libc++ -lc++ -lc++abi "\$@"
EOF
chmod 755 "$tool_root/clang" "$tool_root/clang++"
ln -sf clang "$tool_root/cc"
ln -sf clang++ "$tool_root/c++"
ln -sf "$lld_real" "$tool_root/ld"

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
./bootstrap \
  --prefix="$prefix_root" \
  --parallel="$nproc" \
  --generator="$generator" \
  -- \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_LIBDIR=lib \
  -DCMAKE_USE_OPENSSL=OFF \
  -DBUILD_CursesDialog=OFF

"$make_cmd" -j"$nproc"
"$make_cmd" install

if [ ! -x "$prefix_root/bin/cmake" ]; then
    echo "bootstrap-cmake: expected bootstrap binary missing at $prefix_root/bin/cmake" >&2
    exit 1
fi

printf '%s\n' "$prefix_root/bin/cmake"
