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

if [ ! -f "$files_manifest" ]; then
    echo "repack-installed-package: installed file manifest not found: $files_manifest" >&2
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
install = "sh \"$RECIPE_DIR/install-from-manifest.sh\""
EOF

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
mkdir -p "$output_dir"

exec jpkg build "$tmp_recipe" --build-jpkg --output "$output_dir"
