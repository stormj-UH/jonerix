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

# Point jpkg at the vendored source cache. /workspace/sources/ contains
# pre-fetched tarballs for ~88 packages (musl, micro, ifupdown-ng, etc).
# Without this, jpkg falls through to network downloads which fail or
# stall on slow upstreams (musl.libc.org timed out at 134s in earlier
# CI runs). Same pattern as scripts/ci-build-{x86_64,aarch64}.sh.
if [ -z "${JPKG_SOURCE_CACHE:-}" ] && [ -d /workspace/sources ]; then
    export JPKG_SOURCE_CACHE=/workspace/sources
    echo ">>> JPKG_SOURCE_CACHE=$JPKG_SOURCE_CACHE ($(ls /workspace/sources | wc -l) tarballs)"
fi

# Heavy packages whose build dominates wall time. Filter applied as substring
# match against the recipe directory name. `cmake` is included because it
# pulls in a lot of compile time; trim if you ever want a tighter set.
# `python3` and `ruby` and `perl` are slow interpreter builds — added because
# we mostly want this CI to validate the lightweight package set, not the
# language-runtime tail (which is exercised by publish-packages.yml anyway).
HEAVIES="rust llvm llvm-all nodejs go go-bootstrap go-current cmake python3 ruby perl"

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
# REVIEW: packages/jpkg/recipe.toml and packages/core/jpkg/recipe.toml both
# declare name = "jpkg".  The glob below adds both to the map; the awk lookup
# in the build loop returns whichever line appears first (i.e. glob order).
# One of the two entries will be silently ignored.  Long-term fix: ensure only
# one canonical location for each package, or deduplicate the map by name.
for r in /workspace/packages/*/*/recipe.toml /workspace/packages/*/recipe.toml; do
    [ -f "$r" ] || continue
    name=$(package_name "$r")
    [ -n "$name" ] || continue
    printf '%s\t%s\n' "$name" "$(dirname "$r")" >> "$RECIPE_MAP"
done

# Append every recipe NOT already in the build order so we cover the tail.
# build-order.txt covers ~46 packages; the repo has ~95 recipes total.
#
# NOTE: TRACKED was previously built with `TRACKED=$(cat "$ORDER_FILE")` and
# tested via `case " $TRACKED " in *" $name "*) ...`.  That pattern never
# matched interior lines of the multi-line string (each name is delimited by
# \n, not a space), so every package in build-order.txt was re-appended,
# causing each package to be built twice.  Use grep -qxF against the file
# directly — one syscall, no newline quoting issues.
while IFS=$(printf '\t') read -r name _; do
    grep -qxF "$name" "$ORDER_FILE" && continue
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
    # REVIEW: if jpkg build honours a JPKG_SOURCE_CACHE env var for a
    # pre-populated tarball cache, set it here (e.g. via the workflow's
    # actions/cache step) to avoid re-downloading sources on every run.
    # Currently every build fetches from the upstream URL; not a correctness
    # bug but adds significant latency and network dependency.
    #
    # Per-package timeout: 20 min covers everything except the heavies
    # (which are skipped when SKIP_HEAVIES=true). Without this, a single
    # hung configure / interactive prompt / recursive-make deadlock would
    # stall the entire bootstrap until the 6h workflow timeout.
    # `timeout` returns 124 on TERM, 137 on KILL — both treated as failure.
    pkg_start=$(date +%s)
    # toybox timeout uses short flags only: -k DURATION instead of GNU's
    # --kill-after=DURATION. Confirmed in builder via `timeout --help`.
    # CRITICAL: --build-jpkg makes jpkg actually produce a .jpkg in
    # --output. Without this flag, `jpkg build` installs directly to the
    # builder's live / filesystem (DESTDIR=/), corrupting the host and
    # producing zero .jpkg files. See cmd_build.c line 894.
    if timeout -k 30 1200 jpkg build "$recipe_dir" \
            --build-jpkg --output "$OUT/jpkgs" >"$log" 2>&1; then
        version=$(awk -F'"' '/^version *= *"/ {print $2; exit}' \
            "$recipe_dir/recipe.toml")
        printf '%s\t%s\n' "$name" "$version" >> "$OUT/built-packages.txt"
        build_count=$((build_count + 1))
        pkg_elapsed=$(( $(date +%s) - pkg_start ))
        echo ">>> built $name in ${pkg_elapsed}s"
        # Install the freshly-built jpkg into the builder's live / so
        # subsequent recipes can find it as a build dep. The builder
        # image already has older versions of these packages; --force
        # overwrites with the just-built variant. Without this step,
        # downstream recipes that expect headers/.so files from a freshly-
        # built dep get "not found via jpkg" warnings and fail at the
        # configure / link stage (e.g. hostapd needs nloxide; tmux needs
        # libevent; curl needs libressl). Best-effort — a failure here
        # doesn't fail the bootstrap (the .jpkg still ships into the
        # rootfs in step C, which is what counts for the smoke test).
        new_jpkg=$(ls "$OUT/jpkgs/${name}-${version}-${ARCH}.jpkg" 2>/dev/null | head -1)
        if [ -n "$new_jpkg" ]; then
            jpkg-local install "$new_jpkg" >> "$OUT/install-log/builder-install.log" 2>&1 \
                || echo ">>>   (warn: builder-install of $name failed)" >&2
        fi
    else
        rc=$?
        pkg_elapsed=$(( $(date +%s) - pkg_start ))
        if [ "$rc" -eq 124 ] || [ "$rc" -eq 137 ]; then
            echo "TIMEOUT: $name killed after ${pkg_elapsed}s (20 min cap)" >&2
            printf '%s\ttimeout\n' "$name" >> "$OUT/failed-packages.txt"
        else
            echo "FAIL: $name (rc=$rc, ${pkg_elapsed}s, see build-log/${name}.log)" >&2
            printf '%s\trc=%s\n' "$name" "$rc" >> "$OUT/failed-packages.txt"
        fi
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
#
# IMPORTANT: call jpkg-local directly, not `jpkg --root $ROOTFS local install`.
# jpkg(1) parses --root before dispatching `local` as an external subcommand via
# execvp("jpkg-local", ...).  execvp replaces the process image, so the --root
# value that was stored in g_rootfs is lost — jpkg-local never receives it and
# installs files into the builder's own / instead of $ROOTFS.  Calling jpkg-local
# with an explicit --root argument bypasses the dispatch and is the correct form.
MINIMAL_PKGS="musl zlib toybox dropbear openrc libressl curl zstd jpkg"
echo ">>> installing minimal package set into $ROOTFS"
for pkg in $MINIMAL_PKGS; do
    j=$(ls "$OUT/jpkgs/${pkg}-"*-"${ARCH}.jpkg" 2>/dev/null | head -1)
    [ -n "$j" ] || continue
    jpkg-local install --root "$ROOTFS" "$j" \
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
    jpkg-local install --root "$ROOTFS" "$j" \
        >> "$OUT/install-log/install.log" 2>&1 || true
done < "$OUT/built-packages.txt"

# --- step D: enumerate every shipped /bin binary --------------------------
# Parentheses are required: without them, POSIX operator precedence binds
# -maxdepth 1 only to the -type f branch, leaving -type l unconstrained and
# potentially matching symlinks deeper than bin/.
( cd "$ROOTFS" && find bin/ -maxdepth 1 \( -type f -o -type l \) 2>/dev/null \
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
