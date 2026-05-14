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

# --- Rebuild jpkg from source so bootstrap tests the IN-TREE recipe parser
# rather than whatever was baked into the builder image at last publish-images.
# Without this, recipe-parser fixes in main can't surface in the bootstrap
# because the running /bin/jpkg lags behind.  Mirrors ci-build-{x86_64,aarch64}.sh
# but unconditional (no /jpkg-bin cache) — this script runs in jonerix:builder
# which already has rust/cargo, so the cost is one cargo build (~30s).
if [ -f /workspace/packages/core/jpkg/Cargo.toml ]; then
    echo "=== Rebuilding /bin/jpkg from /workspace/packages/core/jpkg ==="
    (
        cd /workspace/packages/core/jpkg
        TRIPLE=$(rustc -vV | sed -n 's/^host: //p')
        RUSTFLAGS="-C strip=symbols -C target-feature=+crt-static" \
            cargo build --release --frozen --target "$TRIPLE" --bin jpkg --bin jpkg-local
        for b in "target/$TRIPLE/release/jpkg" "target/$TRIPLE/release/jpkg-local"; do
            python3 -c "
import sys
p = sys.argv[1]
d = open(p,'rb').read()
n = d.count(b'/lib64')
if n:
    open(p,'wb').write(d.replace(b'/lib64', b'/lib\\x00\\x00'))
" "$b" || true
        done
        install -m 755 "target/$TRIPLE/release/jpkg" /bin/jpkg
        install -m 755 "target/$TRIPLE/release/jpkg-local" /bin/jpkg-local
    )
    echo "=== /bin/jpkg version after rebuild: $(/bin/jpkg --version 2>&1) ==="
fi

# Point jpkg at the vendored source cache. /workspace/sources/ contains
# pre-fetched tarballs for ~88 packages (musl, pico, ifupdown-ng, etc).
# Without this, jpkg falls through to network downloads which fail or
# stall on slow upstreams (musl.libc.org timed out at 134s in earlier
# CI runs). Same pattern as scripts/ci-build-{x86_64,aarch64}.sh.
#
# full-bootstrap.yml now checks out with lfs:false (the repo's LFS-
# tracked tarballs blow GitHub's free-tier bandwidth budget within a
# handful of runs and the whole checkout step ##error's). The LFS-
# tracked archives therefore come down as ~130-byte pointer files
# that jpkg can't distinguish from real archives until the hash check
# explodes. Purge any matching the LFS pointer signature so jpkg
# falls through to the recipe's source.url instead. Mirrors the
# workaround block in scripts/ci-build-{x86_64,aarch64}.sh.
if [ -d /workspace/sources ]; then
    pointers_purged=0
    for src in /workspace/sources/*; do
        [ -f "$src" ] || continue
        IFS= read -r first_line < "$src" 2>/dev/null || continue
        if [ "$first_line" = "version https://git-lfs.github.com/spec/v1" ]; then
            rm -f "$src"
            pointers_purged=$((pointers_purged + 1))
        fi
    done
    if [ "$pointers_purged" -gt 0 ]; then
        echo "ci-full-bootstrap: purged $pointers_purged LFS pointer(s) from /workspace/sources/ (jpkg will fetch upstream)"
    fi
fi
if [ -z "${JPKG_SOURCE_CACHE:-}" ] && [ -d /workspace/sources ]; then
    export JPKG_SOURCE_CACHE=/workspace/sources
    echo ">>> JPKG_SOURCE_CACHE=$JPKG_SOURCE_CACHE ($(ls /workspace/sources | wc -l) tarballs)"
fi

# Heavy packages whose build dominates wall time. is_heavy() does an exact
# word match against this space-separated list — `*" $1 "*` — so each
# package has to appear literally (no substring magic). New entries that
# pulled the heavy-package filter in were the LLVM splits (LLVM 21 plus
# LLVM 22; all live on llvm-project tarballs) and
# gitredoxide (its vendored archive is on LFS too). Without those entries
# SKIP_HEAVIES=true left them building, which (a) burned hours of CI time
# and (b) burned LFS bandwidth on every full-bootstrap run.
#
# `python3` / `ruby` / `perl` are slow interpreter builds — exercised by
# publish-packages.yml separately; this CI validates the lightweight
# package set.
#
# `cmake` is heavy because of its compile time, even though its source is
# not on LFS.
HEAVIES="rust rustdoc rustfmt rustup llvm llvm-all libllvm clang lld llvm-extra zig libcxx libcxx22 libllvm22 clang22 lld22 llvm22 llvm22-extra nodejs go go-bootstrap go-current cmake python3 ruby perl linux lldb tmux gitredoxide"
# hostapd / wpa_supplicant used to live in HEAVIES because their hostap.git
# Makefiles tripped the jmake MAKEFLAGS escape bug fixed in jmake 1.1.14.
# They're regular recipes now: build under jmake against nloxide (the in-house
# Rust libnl-3 / libnl-genl-3 drop-in) plus libressl/jonerix-headers, and
# finish in well under a minute each on CI hardware. nloxide is also listed
# explicitly in build-order.txt so it lands in /out/jpkgs/ before either
# consumer's install_target_build_deps phase looks for it.

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

# --- helper: toml_string_field <field> <recipe.toml> ----------------------
toml_string_field() {
    _field="$1"
    _file="$2"
    sed -n \
        "s/^[[:space:]]*${_field}[[:space:]]*=[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" \
        "$_file" | sed -n '1p'
}

# --- helper: package_name_from_recipe <recipe.toml> -----------------------
package_name() {
    toml_string_field name "$1"
}

# --- helper: package_version_from_recipe <recipe.toml> --------------------
package_version() {
    toml_string_field version "$1"
}

# --- helper: build_deps_from_recipe <recipe.toml> -------------------------
build_deps() {
    _file="$1"
    _in_dep=0
    _collect=0
    _buf=
    while IFS= read -r _line; do
        if [ "$_line" = "[depends]" ]; then
            _in_dep=1
            continue
        fi
        case "$_line" in
            \[*)
                [ "$_in_dep" -eq 1 ] && break
                ;;
        esac
        [ "$_in_dep" -eq 1 ] || continue
        if [ "$_collect" -eq 0 ]; then
            case "$_line" in
                *"build"*"="*\[*)
                    _collect=1
                    _buf=$_line
                    ;;
            esac
        else
            _buf="${_buf} ${_line}"
        fi
        case "$_collect:$_line" in
            1:*\]*)
                printf '%s\n' "$_buf" \
                    | sed -E 's/.*\[(.*)\].*/\1/' \
                    | sed 's/"//g' \
                    | sed 's/,/ /g'
                return 0
                ;;
        esac
    done < "$_file"
}

# --- helper: recipe_dir_for_name <name> -----------------------------------
recipe_dir_for_name() {
    _want="$1"
    while IFS=$(printf '\t') read -r _name _dir; do
        if [ "$_name" = "$_want" ]; then
            printf '%s\n' "$_dir"
            return 0
        fi
    done < "$RECIPE_MAP"
    return 1
}

# --- helper: extract a .jpkg directly into / -----------------------------
# jpkg format: 8B magic + 4B LE header_len + TOML metadata + zstd-tar payload.
# Bypasses jpkg's database AND post_install hooks. Safe for library deps
# (libressl, libevent, pcre2, nloxide, jonerix-headers etc.) because they
# ship /lib + /include only — no /bin symlink farm to corrupt the live
# builder. NOT used for multicall packages (toybox/mksh/openrc) — those
# never appear in [depends].build for the recipes we build here.
install_local_jpkg() {
    f="$1"
    hdr_len=$(od -An -v -tu4 -N4 -j8 "$f" | tr -d ' ')
    skip=$((12 + hdr_len))
    tail -c +$((skip + 1)) "$f" | zstd -dc | tar xf - -C /
}

# --- helper: ensure awk exists for autoconf -------------------------------
ensure_bootstrap_awk() {
    if command -v awk >/dev/null 2>&1 &&
            awk 'BEGIN { exit 0 }' </dev/null >/dev/null 2>&1; then
        return 0
    fi

    echo ">>> installing onetrueawk bootstrap tool"
    if ! jpkg install --force onetrueawk >/dev/null 2>&1; then
        echo "FATAL: could not install onetrueawk, required by autoconf recipes" >&2
        return 1
    fi
    if ! command -v awk >/dev/null 2>&1 ||
            ! awk 'BEGIN { exit 0 }' </dev/null >/dev/null 2>&1; then
        echo "FATAL: onetrueawk installed but awk is not runnable" >&2
        return 1
    fi
}

# --- helper: patch old builder CONFIG_SITE for this run -------------------
ensure_config_site_target() {
    [ -n "${CONFIG_SITE:-}" ] || return 0
    [ -f "$CONFIG_SITE" ] || return 0
    grep 'ac_cv_target' "$CONFIG_SITE" >/dev/null 2>&1 && return 0

    _site="$OUT/jonerix-config.site"
    cp "$CONFIG_SITE" "$_site"
    cat >> "$_site" <<'EOF'

# Older builder images set only ac_cv_build/ac_cv_host. Some configure
# scripts also run AC_CANONICAL_TARGET; keep target aligned with host.
[ -n "${ac_cv_host-}" ] && ac_cv_target=$ac_cv_host
EOF
    export CONFIG_SITE="$_site"
    echo ">>> using patched CONFIG_SITE=$CONFIG_SITE"
}

# --- helper: install a recipe's [depends].build into the live / ----------
# Mirrors scripts/ci-build-x86_64.sh's install_target_build_deps. For each
# dep, prefer a freshly-built jpkg in $OUT/jpkgs/ (raw extract, no hooks)
# over `jpkg install` from the rolling release. Library packages are
# ALWAYS installed even if a binary of the same name is on PATH because
# their headers may not be on the builder. Tool packages (clang, byacc)
# skip the install if a binary is already on PATH.
install_target_build_deps() {
    _rdir="$1"
    _deps=$(build_deps "$_rdir/recipe.toml")
    [ -z "$_deps" ] && return 0
    for _dep in $_deps; do
        [ -n "$_dep" ] || continue
        # Heavy toolchains: trust whatever the builder image already
        # has and DO NOT reinstall. `jpkg install --force rust` over
        # the live builder corrupts cargo's sysroot mid-bootstrap and
        # every Rust recipe (jmake, nloxide, stormwall, ...) then fails
        # with "could not compile clap_derive (lib)" or similar build-
        # script errors. clang is similarly hot-cached.
        case "$_dep" in
            rust)
                command -v cargo >/dev/null 2>&1 && continue
                # cargo not on PATH and rust not in /out/jpkgs (heavy is
                # in HEAVIES) — silently skip; downstream builds will
                # fail on their own with a clear "cargo not found".
                continue ;;
            go|go-bootstrap|go-current)
                command -v go >/dev/null 2>&1 && continue
                continue ;;
            cmake)
                command -v cmake >/dev/null 2>&1 && continue ;;
            llvm|llvm-all)
                command -v clang >/dev/null 2>&1 && continue ;;
        esac
        # Map binary names to package names first (clang -> llvm, etc).
        _dep_pkg="$_dep"
        case "$_dep" in
            clang|clang++|ld.lld|llvm-ar|llvm-ranlib|llvm-nm|llvm-strip)
                _dep_pkg=llvm ;;
            make)
                _dep_pkg=jmake ;;
            python)
                _dep_pkg=python3 ;;
        esac
        # Library packages always need the install (their headers may be
        # missing in the builder even if a namesake binary is on PATH).
        case "$_dep" in
            xz|bzip2|zstd|zlib|lz4|ncurses|pcre2|libffi|sqlite|\
            libressl|libarchive|libevent|libcxx|nloxide|curl|expat|\
            jonerix-headers)
                _is_lib=1 ;;
            *)
                _is_lib=0 ;;
        esac
        # Priority order:
        #   1. If a local jpkg in $OUT/jpkgs/ for this dep exists, ALWAYS
        #      install (raw zstd+tar, no hooks). Local builds are fresh and
        #      supersede whatever stale state is in the builder image
        #      (e.g. the byacc symlink loop in older builder images).
        #   2. Else if it's a library package, jpkg install from network.
        #   3. Else if `command -v` finds an EXECUTABLE binary on PATH
        #      (broken symlinks fail this check), skip.
        #   4. Else, jpkg install from network.
        _local=$(ls "$OUT/jpkgs/${_dep_pkg}-"*-"${ARCH}.jpkg" 2>/dev/null \
            | sort -V | tail -1)
        if [ -n "$_local" ] && [ -f "$_local" ]; then
            echo "  dep: ${_dep_pkg} (local $(basename "$_local"))"
            case "$_dep_pkg" in
                byacc)
                    # Older builder images shipped /bin/byacc -> yacc and
                    # /bin/yacc -> byacc. tar follows the destination path
                    # poorly through that loop, so clear it before overlaying
                    # the freshly-built package.
                    rm -f /bin/byacc /bin/yacc 2>/dev/null || true
                    ;;
            esac
            install_local_jpkg "$_local" 2>/dev/null || \
                echo "    WARN: extract failed for ${_dep_pkg}" >&2
            continue
        fi
        if [ "$_is_lib" = 1 ]; then
            echo "  dep: ${_dep_pkg} (lib, jpkg install)"
            jpkg install --force "$_dep_pkg" >/dev/null 2>&1 || \
                echo "    WARN: jpkg install failed for ${_dep_pkg}" >&2
            continue
        fi
        # Tool package — accept builder's pre-installed binary IF it is
        # actually executable (broken symlink loops fail `[ -x ]`).
        _resolved=$(command -v "$_dep" 2>/dev/null || true)
        if [ -n "$_resolved" ] && [ -x "$_resolved" ]; then
            continue
        fi
        # Tool not on PATH or broken — pull from network.
        echo "  dep: ${_dep_pkg} (tool, jpkg install)"
        jpkg install --force "$_dep_pkg" >/dev/null 2>&1 || \
            echo "    WARN: jpkg install failed for ${_dep_pkg}" >&2
    done
}

# --- step A: figure out the build order -----------------------------------
# scripts/build-order.txt has the canonical dependency-respecting order.
# Filter blanks/comments and (optionally) heavies.
ORDER_FILE="$OUT/order.txt"
sed -n '
    /^[[:space:]]*#/d
    /^[[:space:]]*$/d
    s/^[[:space:]]*//
    s/[[:space:]]*$//
    p
' /workspace/scripts/build-order.txt > "$ORDER_FILE"

# Pre-build a name -> recipe.toml dir map once. Avoids running a find with
# -exec sh -c per package (O(n^2) shell forks). Each line is "<name>\t<dir>".
RECIPE_MAP="$OUT/recipe-map.tsv"
: > "$RECIPE_MAP"
# REVIEW: packages/jpkg/recipe.toml and packages/core/jpkg/recipe.toml both
# declare name = "jpkg".  The glob below adds both to the map; the lookup in
# the build loop returns whichever line appears first (i.e. glob order).
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
# Refresh jpkg's INDEX once before the build loop so install_target_build_deps
# can `jpkg install --force <dep>` for build deps that aren't yet in /out/jpkgs/
# (e.g. m4oxide via flex when flex builds before m4oxide can in this run's
# build order). Without this, every `jpkg install` errors with
# "no cached INDEX found. Run 'jpkg update' first." (reproduced 2026-04-26).
echo ">>> running jpkg update so install_target_build_deps can fall back to network"
jpkg update >/dev/null 2>&1 || echo "WARN: jpkg update failed; network-fallback installs will not work" >&2
ensure_bootstrap_awk
ensure_config_site_target

build_count=0
fail_count=0
skip_count=0

for name in $(cat "$ORDER_FILE"); do
    # Look up the recipe in the pre-built map (one tab-separated line each).
    recipe_dir=$(recipe_dir_for_name "$name" || true)
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
    # Install declared build deps before invoking jpkg build. Both this
    # call and `jpkg build` below APPEND to $log so the dep-install
    # output is preserved (was being clobbered by jpkg build's `>` until
    # round 5). Recipes that declare `build = ["jonerix-headers", ...]`
    # otherwise only get a `not found via jpkg` warning and hit the
    # actual error at compile time.
    : > "$log"
    {
        echo "=== install_target_build_deps for $name ==="
        install_target_build_deps "$recipe_dir"
        echo "=== jpkg build ==="
    } >> "$log" 2>&1
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
    # producing zero .jpkg files.
    if timeout -k 30 1200 jpkg build "$recipe_dir" \
            --build-jpkg --output "$OUT/jpkgs" >>"$log" 2>&1; then
        version=$(package_version "$recipe_dir/recipe.toml")
        printf '%s\t%s\n' "$name" "$version" >> "$OUT/built-packages.txt"
        build_count=$((build_count + 1))
        pkg_elapsed=$(( $(date +%s) - pkg_start ))
        echo ">>> built $name in ${pkg_elapsed}s"
        # NOTE: previous versions of this script installed each freshly-
        # built .jpkg into the builder's live / via `jpkg-local install`
        # so downstream recipes could find the new versions of their
        # build-time deps. That broke too many things: every install
        # touched /bin (multicall symlinks), and some package's install
        # rewrote /bin/tar -> hwclock somehow, after which `tar -xf` (used
        # by jpkg's own source-extract step) emitted `hwclock: unknown
        # option 'tar'` and EVERY subsequent recipe failed to extract its
        # source. The builder image already ships the deps every recipe
        # needs (musl, libressl, libevent, pcre2, nloxide, etc); its
        # versions may lag the rolling `packages` release by a few
        # hours, but the bootstrap CI is verifying that the RECIPES
        # build from source, not that recipes link against newer-than-
        # builder-image deps. If we ever want self-consistent
        # recipe-against-just-built-recipe builds, the right pattern is
        # a side-load prefix (/tmp/orch-libs) added to LIBRARY_PATH /
        # C_INCLUDE_PATH, NOT a destructive install over /.
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
        while IFS=$(printf '\t') read -r failed_name failed_status; do
            [ -n "$failed_name" ] || continue
            echo "- \`$failed_name\` — ${failed_status:-failed}; see \`build-log/${failed_name}.log\`"
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
if [ "$fail_count" -gt 0 ]; then
    exit 1
fi
exit 0
