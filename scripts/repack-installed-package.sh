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

copy_if_exists() {
    src="$1"
    dest_dir="$2"
    [ -e "$src" ] || [ -L "$src" ] || return 0
    mkdir -p "$dest_dir"
    cp -a "$src" "$dest_dir/"
}

resolve_existing_path() {
    path="$1"
    [ -n "$path" ] || return 1
    if resolved=$(readlink -f "$path" 2>/dev/null) && [ -e "$resolved" ]; then
        printf '%s\n' "$resolved"
        return 0
    fi
    if [ -e "$path" ] || [ -L "$path" ]; then
        printf '%s\n' "$path"
        return 0
    fi
    return 1
}

install_tool() {
    src="$1"
    name="$2"
    [ -n "$src" ] || return 0
    resolved=$(resolve_existing_path "$src" || true)
    [ -n "$resolved" ] || return 0
    mkdir -p "$DESTDIR/bin"
    cp -f "$resolved" "$DESTDIR/bin/$name"
    chmod 755 "$DESTDIR/bin/$name" 2>/dev/null || true
}

find_working_tool() {
    for candidate in "$@"; do
        [ -x "$candidate" ] || continue
        if "$candidate" --version >/dev/null 2>&1 || "$candidate" -dumpmachine >/dev/null 2>&1; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    return 1
}

clang_real=$(
    find_working_tool \
        /lib/llvm*/bin/clang-21 /lib/llvm*/bin/clang \
        /usr/lib/llvm*/bin/clang-21 /usr/lib/llvm*/bin/clang \
        /usr/local/lib/llvm*/bin/clang-21 /usr/local/lib/llvm*/bin/clang \
        /usr/bin/clang-21 /usr/bin/clang \
        /bin/clang-21 /bin/clang \
        /usr/local/bin/clang-21 /usr/local/bin/clang || true
)
[ -n "$clang_real" ] || {
    echo "repack-installed-package: no working clang binary found" >&2
    ls -l /bin/clang /bin/clang-21 /usr/bin/clang /usr/bin/clang-21 2>/dev/null || true
    find /bin /usr/bin /usr/local/bin /lib /usr/lib /usr/local/lib \
        -maxdepth 4 \
        \( -name 'clang*' -o -name 'ld.lld*' -o -name 'lld' -o -name 'llvm-*' \) \
        2>/dev/null | sort >&2 || true
    exit 1
}

clang_resolved=$(resolve_existing_path "$clang_real" || true)
[ -n "$clang_resolved" ] || {
    echo "repack-installed-package: failed to resolve clang path: $clang_real" >&2
    exit 1
}

mkdir -p "$DESTDIR/bin" "$DESTDIR/lib" "$DESTDIR/etc"
case "$(dirname "$clang_resolved")" in
    /lib/llvm*/bin|/usr/lib/llvm*/bin|/usr/local/lib/llvm*/bin)
        cp -a "$(dirname "$clang_resolved")/." "$DESTDIR/bin/"
        ;;
    *)
        install_tool "$clang_resolved" "clang-21"
        clangxx_real=$(
            find_working_tool \
                /lib/llvm*/bin/clang++-21 /lib/llvm*/bin/clang++ \
                /usr/lib/llvm*/bin/clang++-21 /usr/lib/llvm*/bin/clang++ \
                /usr/local/lib/llvm*/bin/clang++-21 /usr/local/lib/llvm*/bin/clang++ \
                /usr/bin/clang++-21 /usr/bin/clang++ \
                /bin/clang++-21 /bin/clang++ \
                /usr/local/bin/clang++-21 /usr/local/bin/clang++ || true
        )
        ld_real=$(
            find_working_tool \
                /lib/llvm*/bin/ld.lld /lib/llvm*/bin/ld.lld-21 /lib/llvm*/bin/lld \
                /usr/lib/llvm*/bin/ld.lld /usr/lib/llvm*/bin/ld.lld-21 /usr/lib/llvm*/bin/lld \
                /usr/local/lib/llvm*/bin/ld.lld /usr/local/lib/llvm*/bin/ld.lld-21 /usr/local/lib/llvm*/bin/lld \
                /usr/bin/ld.lld /usr/bin/ld.lld-21 /usr/bin/lld \
                /bin/ld.lld /bin/ld.lld-21 /bin/lld \
                /usr/local/bin/ld.lld /usr/local/bin/ld.lld-21 /usr/local/bin/lld || true
        )
        install_tool "$clangxx_real" "clang++-21"
        install_tool "$ld_real" "ld.lld"
        for tool in llvm-ar llvm-nm llvm-ranlib llvm-strip llvm-objcopy llvm-objdump llvm-readelf llvm-config; do
            for base in /lib/llvm*/bin /usr/lib/llvm*/bin /usr/local/lib/llvm*/bin /bin /usr/bin /usr/local/bin; do
                if [ -x "$base/$tool" ]; then
                    install_tool "$base/$tool" "$tool"
                    break
                fi
            done
        done
        ;;
esac

if [ ! -e "$DESTDIR/bin/clang-21" ]; then
    if [ -e "$DESTDIR/bin/clang" ]; then
        install_tool "$DESTDIR/bin/clang" "clang-21"
    fi
fi
if [ ! -e "$DESTDIR/bin/clang++-21" ]; then
    if [ -e "$DESTDIR/bin/clang++" ]; then
        install_tool "$DESTDIR/bin/clang++" "clang++-21"
    else
        ln -sf clang-21 "$DESTDIR/bin/clang++-21"
    fi
fi
if [ ! -e "$DESTDIR/bin/ld.lld" ] && [ -e "$DESTDIR/bin/lld" ]; then
    ln -sf lld "$DESTDIR/bin/ld.lld"
fi

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
