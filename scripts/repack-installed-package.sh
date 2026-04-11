#!/bin/sh
set -e

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 <package> <output-dir> [recipe-dir]" >&2
    exit 1
fi

pkg_name="$1"
output_dir="$2"
recipe_dir="${3:-}"
db_root="${JPKG_DB_ROOT:-/var/db/jpkg}"
installed_dir="$db_root/installed/$pkg_name"
files_manifest="$installed_dir/files"

if [ -z "$recipe_dir" ]; then
    for d in /workspace/packages/core /workspace/packages/develop /workspace/packages/extra; do
        if [ -f "$d/$pkg_name/recipe.toml" ]; then
            recipe_dir="$d/$pkg_name"
            break
        fi
    done
fi

if [ -z "$recipe_dir" ] || [ ! -f "$recipe_dir/recipe.toml" ]; then
    echo "repack-installed-package: recipe not found for $pkg_name" >&2
    exit 1
fi

tmp_root=$(mktemp -d "/tmp/repack-${pkg_name}-XXXXXX")
tmp_recipe="$tmp_root/recipe"
trap 'rm -rf "$tmp_root"' EXIT INT TERM
mkdir -p "$tmp_recipe"

awk '
    BEGIN { keep = 0 }
    /^\[(package|depends|hooks)\]$/ { keep = 1; print; next }
    /^\[[^]]+\]$/ { keep = 0 }
    keep { print }
' "$recipe_dir/recipe.toml" > "$tmp_recipe/recipe.toml"

cat >> "$tmp_recipe/recipe.toml" <<'EOF'

[build]
system = "custom"
build = ""
EOF

if [ -f "$files_manifest" ]; then
    echo "repack-installed-package: using installed manifest for $pkg_name" >&2
    printf 'install = "sh \\"$RECIPE_DIR/install-from-manifest.sh\\""\n' >> "$tmp_recipe/recipe.toml"
    cp "$files_manifest" "$tmp_recipe/files.manifest"

    cat > "$tmp_recipe/install-from-manifest.sh" <<'EOF'
#!/bin/sh
set -e

manifest="$RECIPE_DIR/files.manifest"

while IFS= read -r line || [ -n "$line" ]; do
    [ -n "$line" ] || continue

    sha=${line%% *}
    rest=${line#* }
    mode=${rest%% *}
    pathspec=${rest#* }

    case "$pathspec" in
        *" -> "*)
            path=${pathspec%% -> *}
            target=${pathspec#* -> }
            mkdir -p "$DESTDIR$(dirname "$path")"
            ln -snf "$target" "$DESTDIR$path"
            ;;
        *)
            path="$pathspec"
            if [ ! -e "$path" ] && [ ! -L "$path" ]; then
                echo "repack-installed-package: missing installed path: $path" >&2
                exit 1
            fi
            mkdir -p "$DESTDIR$(dirname "$path")"
            cp -a "$path" "$DESTDIR$(dirname "$path")/"
            ;;
    esac
done < "$manifest"

if [ -f /workspace/scripts/normalize-lib64.py ]; then
    python3 /workspace/scripts/normalize-lib64.py \
        "$DESTDIR/bin" "$DESTDIR/lib" "$DESTDIR/etc"
fi
EOF
    chmod 755 "$tmp_recipe/install-from-manifest.sh"
elif [ "$pkg_name" = "llvm" ]; then
    echo "repack-installed-package: installed manifest missing; snapshotting live llvm toolchain" >&2
    printf 'install = "sh \\"$RECIPE_DIR/install-llvm-live.sh\\""\n' >> "$tmp_recipe/recipe.toml"

    cat > "$tmp_recipe/install-llvm-live.sh" <<'EOF'
#!/bin/sh
set -e

find_llvm_bin() {
    for d in /lib/llvm*/bin /usr/lib/llvm*/bin /usr/local/lib/llvm*/bin; do
        [ -d "$d" ] && { printf '%s\n' "$d"; return 0; }
    done
    return 1
}

copy_if_exists() {
    src="$1"
    dest_dir="$2"
    [ -e "$src" ] || [ -L "$src" ] || return 0
    mkdir -p "$dest_dir"
    cp -a "$src" "$dest_dir/"
}

llvm_bin=$(find_llvm_bin || true)
[ -n "$llvm_bin" ] || {
    echo "repack-installed-package: no llvm bin directory found under /lib/llvm*/bin" >&2
    exit 1
}

mkdir -p "$DESTDIR/bin" "$DESTDIR/lib" "$DESTDIR/etc"
cp -a "$llvm_bin/." "$DESTDIR/bin/"

if [ ! -e "$DESTDIR/bin/clang-21" ] && [ -e "$DESTDIR/bin/clang" ]; then
    ln -sf clang "$DESTDIR/bin/clang-21"
fi
if [ ! -e "$DESTDIR/bin/clang++-21" ] && [ -e "$DESTDIR/bin/clang++" ]; then
    ln -sf clang++ "$DESTDIR/bin/clang++-21"
fi
if [ ! -e "$DESTDIR/bin/ld.lld" ] && [ -e "$DESTDIR/bin/lld" ]; then
    ln -sf lld "$DESTDIR/bin/ld.lld"
fi

clang_real="$llvm_bin/clang-21"
[ -x "$clang_real" ] || clang_real="$llvm_bin/clang"
[ -x "$clang_real" ] || {
    echo "repack-installed-package: no working clang binary found in $llvm_bin" >&2
    exit 1
}

triple=$("$clang_real" -dumpmachine 2>/dev/null || echo "$(uname -m)-jonerix-linux-musl")
mkdir -p "$DESTDIR/etc/clang"
if [ -d /etc/clang ]; then
    cp -a /etc/clang/. "$DESTDIR/etc/clang/"
fi
if [ ! -f "$DESTDIR/etc/clang/$triple.cfg" ]; then
    printf -- '--rtlib=compiler-rt\n--unwindlib=libunwind\n-fuse-ld=lld\n' > "$DESTDIR/etc/clang/$triple.cfg"
fi

cat > "$DESTDIR/bin/clang" <<CLANGWRAP
#!/bin/sh
exec /bin/clang-21 --config="/etc/clang/$triple.cfg" "\$@"
CLANGWRAP
cat > "$DESTDIR/bin/clang++" <<CLANGXXWRAP
#!/bin/sh
exec /bin/clang++-21 --config="/etc/clang/$triple.cfg" --unwindlib=libunwind -stdlib=libc++ -lc++ -lc++abi "\$@"
CLANGXXWRAP
chmod 755 "$DESTDIR/bin/clang" "$DESTDIR/bin/clang++"
ln -sf clang "$DESTDIR/bin/cc"
ln -sf clang++ "$DESTDIR/bin/c++"
ln -sf ld.lld "$DESTDIR/bin/ld" 2>/dev/null || true

if [ -d /lib/clang ]; then
    cp -a /lib/clang "$DESTDIR/lib/"
fi
copy_if_exists /lib/libssp_nonshared.a "$DESTDIR/lib"
copy_if_exists /lib/libRemarks.so "$DESTDIR/lib"
copy_if_exists /lib/libRemarks.so.21.1 "$DESTDIR/lib"

for pattern in /lib/libLLVM*.so* /lib/libclang*.so* /lib/liblld*.so*; do
    for f in $pattern; do
        [ -e "$f" ] || [ -L "$f" ] || continue
        cp -a "$f" "$DESTDIR/lib/"
    done
done

if [ -f /workspace/scripts/normalize-lib64.py ]; then
    python3 /workspace/scripts/normalize-lib64.py \
        "$DESTDIR/bin" "$DESTDIR/lib" "$DESTDIR/etc"
fi
EOF
    chmod 755 "$tmp_recipe/install-llvm-live.sh"
else
    echo "repack-installed-package: installed file manifest not found: $files_manifest" >&2
    exit 1
fi
mkdir -p "$output_dir"

exec jpkg build "$tmp_recipe" --build-jpkg --output "$output_dir"
