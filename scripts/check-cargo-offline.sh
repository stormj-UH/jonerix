#!/bin/sh
# Verify Cargo package builds are locked to vendored/offline inputs.
#
# SPDX-License-Identifier: MIT

set -eu

ROOT=${1:-.}
RECIPES=${ROOT}/packages
SOURCES=${ROOT}/sources

failures=0

fail() {
    printf '%s\n' "$*" >&2
    failures=$((failures + 1))
}

check_cargo_flags() {
    recipe=$1

    grep -n 'cargo ' "$recipe" 2>/dev/null | while IFS= read -r row; do
        line=${row%%:*}
        text=${row#*:}
        trimmed=$(printf '%s\n' "$text" | sed 's/^[	 ]*//')

        case "$trimmed" in
            \#*) continue ;;
        esac

        case "$text" in
            *'cargo fetch'*)
                printf 'CARGO: %s:%s uses cargo fetch\n' "$recipe" "$line" >&2
                exit 2
                ;;
            *'cargo build'*|*'cargo test'*|*'cargo rustc'*)
                case "$text" in
                    *'--offline'*|*'--frozen'*) ;;
                    *)
                        printf 'CARGO: %s:%s missing --offline/--frozen: %s\n' "$recipe" "$line" "$text" >&2
                        exit 2
                        ;;
                esac
                ;;
        esac
    done
}

# tar invocations below use `2>/dev/null` to drop stderr noise — chiefly the
# GNU-tar warning flood that fires once per file when reading tarballs created
# on macOS, which leak `LIBARCHIVE.xattr.com.apple.provenance` PAX headers.
# A vendor tarball can have tens of thousands of entries and the warnings
# would otherwise dominate the CI logs and time the job out. Existence of the
# tarball is checked separately via `[ -f "$src" ]`, so silencing tar's
# stderr here does not hide real "file missing" errors.
check_vendor_tarball() {
    label=$1
    file=$2
    src=${SOURCES}/${file}

    if [ ! -f "$src" ]; then
        fail "CARGO: missing vendored tarball for $label: $file"
        return
    fi

    if is_lfs_pointer "$src" && [ "${ALLOW_LFS_POINTER_SOURCES:-0}" = 1 ]; then
        printf 'CARGO: skipping LFS pointer tarball content check for %s: %s\n' "$label" "$file" >&2
        return
    fi

    if ! tar tf "$src" 2>/dev/null | grep '/Cargo.lock$' >/dev/null 2>&1; then
        fail "CARGO: $label tarball has no Cargo.lock: $file"
    fi
    if ! tar tf "$src" 2>/dev/null | grep '/vendor/' >/dev/null 2>&1; then
        fail "CARGO: $label tarball has no vendor/: $file"
    fi
    if ! tar tf "$src" 2>/dev/null | grep '/\.cargo/config\.toml$' >/dev/null 2>&1; then
        fail "CARGO: $label tarball has no .cargo/config.toml: $file"
    fi
}

check_path_only_tarball() {
    label=$1
    file=$2
    src=${SOURCES}/${file}

    if [ ! -f "$src" ]; then
        fail "CARGO: missing source tarball for $label: $file"
        return
    fi

    if is_lfs_pointer "$src" && [ "${ALLOW_LFS_POINTER_SOURCES:-0}" = 1 ]; then
        printf 'CARGO: skipping LFS pointer tarball content check for %s: %s\n' "$label" "$file" >&2
        return
    fi

    lock_path=$(tar tf "$src" 2>/dev/null | grep '/Cargo\.lock$' | head -n 1 || true)
    [ -n "$lock_path" ] || return 0

    if tar xOf "$src" "$lock_path" 2>/dev/null | grep '^source = ' >/dev/null 2>&1; then
        fail "CARGO: $label has external Cargo.lock sources but no vendor/: $file"
    fi
}

check_vendor_dir() {
    label=$1
    vendor_dir=$2
    config_file=$3

    if [ ! -d "${ROOT}/${vendor_dir}" ]; then
        fail "CARGO: missing vendored directory for $label: $vendor_dir"
    fi
    if [ ! -f "${ROOT}/${config_file}" ]; then
        fail "CARGO: missing source replacement config for $label: $config_file"
    fi
}

check_local_cargo_dir() {
    label=$1
    dir=$2
    cargo_dir=${ROOT}/${dir}
    lock_file=${cargo_dir}/Cargo.lock

    if [ ! -f "${cargo_dir}/Cargo.toml" ]; then
        fail "CARGO: missing Cargo.toml for $label: $dir"
        return
    fi

    if [ -f "$lock_file" ] && grep '^source = ' "$lock_file" >/dev/null 2>&1; then
        check_vendor_dir "$label" "${dir}/vendor" "${dir}/.cargo/config.toml"
    fi
}

for recipe in "${RECIPES}"/core/*/recipe.toml \
              "${RECIPES}"/develop/*/recipe.toml \
              "${RECIPES}"/extra/*/recipe.toml; do
    [ -f "$recipe" ] || continue
    if ! check_cargo_flags "$recipe"; then
        failures=$((failures + 1))
    fi
done

# Derive tarball filename from the recipe's version field: $pkg-$version.tar.gz.
# Avoids hardcoded version strings that break on bumps.
tarball_from_recipe() {
    pkg=$1
    recipe=$2
    ver=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$recipe" | head -n 1)
    [ -n "$ver" ] || return 1
    printf '%s\n' "${pkg}-${ver}.tar.gz"
}

# Strip trailing -rN package revision from version (e.g. 0.7.0-r1 -> 0.7.0).
strip_revision() {
    case "$1" in
        *-r[0-9]*)
            base=${1%-r*}
            suffix=${1##*-r}
            case "$suffix" in
                ''|*[!0-9]*) printf '%s\n' "$1" ;;
                *) printf '%s\n' "$base" ;;
            esac
            ;;
        *) printf '%s\n' "$1" ;;
    esac
}

is_lfs_pointer() {
    file=$1
    IFS= read -r first < "$file" || return 1
    [ "$first" = "version https://git-lfs.github.com/spec/v1" ]
}

# Locate the source tarball for a package.  Tries, in order:
#   $pkg-$version.tar.gz          (with -rN)
#   $pkg-$base_version.tar.gz     (without -rN)
#   $pkg-v$base_version.tar.gz    (v-prefixed, e.g. jmake)
#   <recipe url basename>         (commit-hash-pinned, e.g.
#                                  m4oxide-40573c872ea5f076a70a78c51946d938ed80ae9c.tar.gz)
find_source_tarball() {
    pkg=$1
    recipe=$2
    ver=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$recipe" | head -n 1)
    [ -n "$ver" ] || return 1
    base=$(strip_revision "$ver")
    if [ -f "${SOURCES}/${pkg}-${ver}.tar.gz" ]; then
        printf '%s\n' "${pkg}-${ver}.tar.gz"
        return 0
    fi
    if [ -f "${SOURCES}/${pkg}-${base}.tar.gz" ]; then
        printf '%s\n' "${pkg}-${base}.tar.gz"
        return 0
    fi
    if [ -f "${SOURCES}/${pkg}-v${ver}.tar.gz" ]; then
        printf '%s\n' "${pkg}-v${ver}.tar.gz"
        return 0
    fi
    if [ -f "${SOURCES}/${pkg}-v${base}.tar.gz" ]; then
        printf '%s\n' "${pkg}-v${base}.tar.gz"
        return 0
    fi
    # Fall back to the recipe URL's basename. Catches commit-hash-pinned
    # tarballs whose filename doesn't follow $pkg-$version naming.
    url=$(sed -n 's/^url *= *"\(.*\)"/\1/p' "$recipe" | head -n 1)
    if [ -n "$url" ]; then
        url_no_query=${url%%\?*}
        url_base=${url_no_query##*/}
        if [ -n "$url_base" ] && [ -f "${SOURCES}/${url_base}" ]; then
            printf '%s\n' "${url_base}"
            return 0
        fi
    fi
    printf '%s\n' "${pkg}-${ver}.tar.gz"
}

# Tarball packages with registry dependencies must carry Cargo.lock,
# vendor/, and source replacement config.  Filenames are derived from
# each recipe's version field so version bumps propagate automatically.
for pkg in brash exproxide gitoxide jmake ripgrep stormwall uutils; do
    recipe=$(find "${RECIPES}" -path "*/${pkg}/recipe.toml" | head -n 1)
    [ -f "$recipe" ] || continue
    file=$(find_source_tarball "$pkg" "$recipe") || continue
    check_vendor_tarball "$pkg" "$file"
done

# Path-only tarballs: no vendor/ required but Cargo.lock must not
# reference external registries.
for pkg in anvil jfsck lsusb-rs m4oxide readlineoxide; do
    recipe=$(find "${RECIPES}" -path "*/${pkg}/recipe.toml" | head -n 1)
    [ -f "$recipe" ] || continue
    file=$(find_source_tarball "$pkg" "$recipe") || continue
    check_path_only_tarball "$pkg" "$file"
done

# Local-only Cargo packages: path-only crates need no vendor tree; crates with
# registry sources must carry vendor/ plus .cargo/config.toml.
check_local_cargo_dir jcarp packages/extra/jcarp/src
check_local_cargo_dir jonerix-util packages/extra/jonerix-util/src
check_local_cargo_dir jpkg packages/core/jpkg
check_local_cargo_dir nloxide packages/extra/nloxide/src

if [ "$failures" -ne 0 ]; then
    printf 'cargo offline check failed: %s issue(s)\n' "$failures" >&2
    exit 1
fi

printf 'cargo offline check passed\n'
