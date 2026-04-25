#!/bin/sh
# ci-full-bootstrap.sh — runs INSIDE the jonerix:builder container.
#
# Invoked by .github/workflows/full-bootstrap.yml. Does the heavy lifting for
# steps 2–4 of that workflow: build every recipe in the tree from source
# (with optional heavy-package filter), then install all freshly-built
# .jpkgs into a clean minimal rootfs at /out/rootfs/.
#
# Outputs (all under /out/):
#   build-log/<pkg>.log       per-package build stdout+stderr
#   install-log/install.log   `jpkg install` output for the rootfs assembly
#   built-packages.txt        names+versions that built successfully
#   skipped-packages.txt      names skipped by the heavies filter
#   failed-packages.txt       names that failed to build
#   binaries.txt              every /bin/<name> that lands in the final rootfs
#   rootfs.tar                portable rootfs (consumed by the smoke-test step)
#   report.md                 markdown summary appended to the workflow run

set -eu

# --- inputs (env from workflow) -------------------------------------------
SKIP_HEAVIES="${SKIP_HEAVIES:-true}"
STOP_ON_FAIL="${STOP_ON_FAIL:-false}"
ARCH="${ARCH:-$(uname -m)}"

# Heavy packages whose build dominates wall time. Filter applied as substring
# match against the recipe directory name. `cmake` is included because it
# pulls in a lot of compile time; trim if you ever want a tighter set.
HEAVIES="rust llvm llvm-all nodejs go go-bootstrap go-current cmake"

OUT=/out
mkdir -p "$OUT/build-log" "$OUT/install-log" "$OUT/jpkgs" "$OUT/rootfs"
: > "$OUT/built-packages.txt"
: > "$OUT/skipped-packages.txt"
: > "$OUT/failed-packages.txt"

start_epoch=$(date +%s)

# --- helper: is_heavy <name> ----------------------------------------------
is_heavy() {
    case " $HEAVIES " in
        *" $1 "*) return 0 ;;
    esac
    return 1
}

# --- helper: package_name_from_recipe <recipe.toml> -----------------------
package_name() {
    awk -F'"' '/^name *= *"/ {print $2; exit}' "$1"
}

# --- step A: figure out the build order -----------------------------------
# scripts/build-order.txt has the canonical dependency-respecting order.
# Filter blanks/comments and (optionally) heavies.
ORDER_FILE="$OUT/order.txt"
awk '
    /^[[:space:]]*#/ { next }
    /^[[:space:]]*$/ { next }
    { print }
' /workspace/scripts/build-order.txt > "$ORDER_FILE"

# Pre-build a name -> recipe.toml dir map once. Avoids running a find with
# -exec sh -c per package (O(n^2) shell forks). Each line is "<name>\t<dir>".
RECIPE_MAP="$OUT/recipe-map.tsv"
: > "$RECIPE_MAP"
for r in /workspace/packages/*/*/recipe.toml /workspace/packages/*/recipe.toml; do
    [ -f "$r" ] || continue
    name=$(package_name "$r")
    [ -n "$name" ] || continue
    printf '%s\t%s\n' "$name" "$(dirname "$r")" >> "$RECIPE_MAP"
done

# Append every recipe NOT already in the build order so we cover the tail.
# build-order.txt covers ~46 packages; the repo has ~95 recipes total.
TRACKED=$(cat "$ORDER_FILE")
while IFS=$(printf '\t') read -r name _; do
    case " $TRACKED " in
        *" $name "*) continue ;;
    esac
    echo "$name" >> "$ORDER_FILE"
done < "$RECIPE_MAP"

# --- step B: walk the order, build each package ---------------------------
build_count=0
fail_count=0
skip_count=0

for name in $(cat "$ORDER_FILE"); do
    # Look up the recipe in the pre-built map (one tab-separated line each).
    recipe_dir=$(awk -F'\t' -v n="$name" '$1 == n { print $2; exit }' \
        "$RECIPE_MAP")
    if [ -z "$recipe_dir" ]; then
        echo "WARN: no recipe found for $name" >&2
        continue
    fi

    if [ "$SKIP_HEAVIES" = "true" ] && is_heavy "$name"; then
        printf '%s\n' "$name" >> "$OUT/skipped-packages.txt"
        skip_count=$((skip_count + 1))
        continue
    fi

    log="$OUT/build-log/${name}.log"
    echo ">>> building $name from $recipe_dir"
    if jpkg build "$recipe_dir" --output "$OUT/jpkgs" >"$log" 2>&1; then
        version=$(awk -F'"' '/^version *= *"/ {print $2; exit}' \
            "$recipe_dir/recipe.toml")
        printf '%s\t%s\n' "$name" "$version" >> "$OUT/built-packages.txt"
        build_count=$((build_count + 1))
    else
        echo "FAIL: $name (see build-log/${name}.log)" >&2
        printf '%s\n' "$name" >> "$OUT/failed-packages.txt"
        fail_count=$((fail_count + 1))
        if [ "$STOP_ON_FAIL" = "true" ]; then
            echo "STOP_ON_FAIL=true — aborting after $name" >&2
            break
        fi
    fi
done

# --- step C: assemble a fresh minimal rootfs ------------------------------
# Mirror Dockerfile.minimal's directory skeleton.
ROOTFS="$OUT/rootfs"
mkdir -p \
    "$ROOTFS/etc/jpkg/keys" \
    "$ROOTFS/etc/ssl/certs" \
    "$ROOTFS/etc/init.d" \
    "$ROOTFS/etc/conf.d" \
    "$ROOTFS/etc/network" \
    "$ROOTFS/bin" \
    "$ROOTFS/lib" \
    "$ROOTFS/var/log" \
    "$ROOTFS/var/cache/jpkg" \
    "$ROOTFS/var/db/jpkg/installed" \
    "$ROOTFS/home" \
    "$ROOTFS/root" \
    "$ROOTFS/dev" \
    "$ROOTFS/proc" \
    "$ROOTFS/sys" \
    "$ROOTFS/run" \
    "$ROOTFS/tmp"
chmod 1777 "$ROOTFS/tmp"

# Local-mirror config so jpkg --root resolves the just-built jpkgs.
cat > "$ROOTFS/etc/jpkg/repos.conf" <<EOF
[repo]
url = "file://$OUT/jpkgs"
EOF

# Generate a local INDEX so `jpkg --root install` can resolve names+deps.
PKG_DIR="$OUT/jpkgs" \
OUT_INDEX="$OUT/jpkgs/INDEX" \
RECIPES_ROOT=/workspace/packages \
STALE_LIST=/dev/null \
    /workspace/scripts/gen-index.sh
zstd -19 -f -o "$OUT/jpkgs/INDEX.zst" "$OUT/jpkgs/INDEX"

# Step C.1 — install the minimal-image set first.
MINIMAL_PKGS="musl zlib toybox dropbear openrc libressl curl zstd jpkg"
echo ">>> installing minimal package set into $ROOTFS"
for pkg in $MINIMAL_PKGS; do
    j=$(ls "$OUT/jpkgs/${pkg}-"*-"${ARCH}.jpkg" 2>/dev/null | head -1)
    [ -n "$j" ] || continue
    jpkg --root "$ROOTFS" local install "$j" \
        >> "$OUT/install-log/install.log" 2>&1 || true
done

# Step C.2 — install everything else we built. Best-effort; failures are
# logged but don't abort the run because the smoke test should still cover
# whatever DID install.
echo ">>> installing every other built package"
while IFS=$(printf '\t') read -r name version; do
    case " $MINIMAL_PKGS " in
        *" $name "*) continue ;;
    esac
    j=$(ls "$OUT/jpkgs/${name}-${version}-${ARCH}.jpkg" 2>/dev/null | head -1)
    [ -n "$j" ] || continue
    jpkg --root "$ROOTFS" local install "$j" \
        >> "$OUT/install-log/install.log" 2>&1 || true
done < "$OUT/built-packages.txt"

# --- step D: enumerate every shipped /bin binary --------------------------
( cd "$ROOTFS" && find bin/ -maxdepth 1 -type f -o -type l 2>/dev/null \
    | sort -u ) > "$OUT/binaries.txt"

# --- step E: tar up the rootfs for the smoke-test step --------------------
tar -C "$ROOTFS" -cf "$OUT/rootfs.tar" .
ls -lh "$OUT/rootfs.tar" >&2

# --- step F: write the report --------------------------------------------
end_epoch=$(date +%s)
elapsed=$((end_epoch - start_epoch))
total_ordered=$(wc -l < "$ORDER_FILE")
binaries=$(wc -l < "$OUT/binaries.txt")

{
    echo "# Bootstrap report — ${ARCH}"
    echo
    echo "_${elapsed}s in the build container_"
    echo
    echo '## Counts'
    echo
    echo "| | count |"
    echo "|--|--|"
    echo "| recipes considered | $total_ordered |"
    echo "| built successfully | $build_count |"
    echo "| skipped (heavies)  | $skip_count |"
    echo "| failed             | $fail_count |"
    echo "| /bin binaries shipped | $binaries |"
    echo
    if [ "$fail_count" -gt 0 ]; then
        echo '## Failed packages'
        echo
        while read -r f; do
            echo "- \`$f\` — see \`build-log/${f}.log\`"
        done < "$OUT/failed-packages.txt"
        echo
    fi
    if [ "$skip_count" -gt 0 ]; then
        echo '<details><summary>Skipped (heavies)</summary>'
        echo
        echo '```'
        cat "$OUT/skipped-packages.txt"
        echo '```'
        echo
        echo '</details>'
        echo
    fi
} > "$OUT/report.md"

echo "=== bootstrap done: built=$build_count failed=$fail_count skipped=$skip_count ==="
exit 0
