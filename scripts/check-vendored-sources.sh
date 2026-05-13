#!/bin/sh
# Verify every remote recipe source has a matching file in sources/.
#
# Match rules intentionally mirror jpkg's JPKG_SOURCE_CACHE lookup:
#   1. exact URL basename
#   2. package-version.*
#   3. package-upstream_version.* where upstream_version strips a trailing -rN
#
# SPDX-License-Identifier: MIT

set -eu

ROOT=${1:-.}
RECIPES=${ROOT}/packages
SOURCES=${ROOT}/sources

failures=0

# When running under a checkout that didn't fetch LFS objects (e.g. the LFS
# bandwidth quota is exhausted, or the runner deliberately skipped `lfs:
# true`), the LFS-tracked source tarballs are present only as small pointer
# files.  Their sha256 doesn't match the recipe-pinned tarball hash, but the
# file IS still version-pinned via the LFS oid in the pointer.  When
# ALLOW_LFS_POINTER_SOURCES=1, treat the pointer file as an exemption: skip
# the content-side check for that one entry rather than failing the whole
# gate. Mirrors the same pattern in check-cargo-offline.sh.
is_lfs_pointer() {
    file=$1
    IFS= read -r first < "$file" || return 1
    [ "$first" = "version https://git-lfs.github.com/spec/v1" ]
}

strip_release_suffix() {
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

first_match() {
    pkg=$1
    version=$2
    url=$3

    url_no_query=${url%%\?*}
    url_base=${url_no_query##*/}
    base_version=$(strip_release_suffix "$version")

    if [ -n "$url_base" ] && [ -f "${SOURCES}/${url_base}" ]; then
        printf '%s\n' "${SOURCES}/${url_base}"
        return 0
    fi

    for f in "${SOURCES}/${pkg}-${version}."*; do
        [ -f "$f" ] || continue
        printf '%s\n' "$f"
        return 0
    done

    if [ "$base_version" != "$version" ]; then
        for f in "${SOURCES}/${pkg}-${base_version}."*; do
            [ -f "$f" ] || continue
            printf '%s\n' "$f"
            return 0
        done
    fi

    return 1
}

recipe_value() {
    key=$1
    file=$2
    grep "^${key}[ 	]*=" "$file" 2>/dev/null |
        head -n 1 |
        sed 's/^[^=]*=[ 	]*"//; s/".*$//'
}

check_cached_file() {
    label=$1
    file=$2
    expected=$3
    src=${SOURCES}/${file}

    if [ ! -f "$src" ]; then
        printf 'MISSING: %s vendored file %s\n' "$label" "$file" >&2
        failures=$((failures + 1))
        return
    fi

    if is_lfs_pointer "$src" && [ "${ALLOW_LFS_POINTER_SOURCES:-0}" = 1 ]; then
        printf 'SKIP: %s is an LFS pointer; sha256 check exempt (%s)\n' "$label" "$file" >&2
        return
    fi

    if [ -n "$expected" ]; then
        got=$(sha256sum "$src" | awk '{print $1}')
        if [ "$got" != "$expected" ]; then
            printf 'HASH: %s expected %s got %s (%s)\n' "$label" "$expected" "$got" "$src" >&2
            failures=$((failures + 1))
        fi
    fi
}

for recipe in "${RECIPES}"/core/*/recipe.toml \
              "${RECIPES}"/develop/*/recipe.toml \
              "${RECIPES}"/extra/*/recipe.toml; do
    [ -f "$recipe" ] || continue

    pkg=$(recipe_value name "$recipe")
    version=$(recipe_value version "$recipe")
    url=$(recipe_value url "$recipe")
    sha256=$(recipe_value sha256 "$recipe")

    [ -n "$pkg" ] || pkg=$(basename "$(dirname "$recipe")")
    [ -n "$version" ] || version=unknown
    [ -n "$url" ] || continue
    [ "$url" = "local" ] && continue

    if ! src=$(first_match "$pkg" "$version" "$url"); then
        printf 'MISSING: %s %s source for %s\n' "$pkg" "$version" "$url" >&2
        failures=$((failures + 1))
        continue
    fi

    if is_lfs_pointer "$src" && [ "${ALLOW_LFS_POINTER_SOURCES:-0}" = 1 ]; then
        printf 'SKIP: %s is an LFS pointer; sha256 check exempt (%s)\n' "$pkg" "$src" >&2
        continue
    fi

    if [ -n "$sha256" ]; then
        got=$(sha256sum "$src" | awk '{print $1}')
        if [ "$got" != "$sha256" ]; then
            printf 'HASH: %s expected %s got %s (%s)\n' "$pkg" "$sha256" "$got" "$src" >&2
            failures=$((failures + 1))
        fi
    fi
done

# Non-[source] vendored inputs used directly by package build scripts.
check_cached_file ca-certificates cacert.pem b6e66569cc3d438dd5abe514d0df50005d570bfc96c14dca8f768d020cb96171
check_cached_file tzdata tzdata2026a.tar.gz 77b541725937bb53bd92bd484c0b43bec8545e2d3431ee01f04ef8f2203ba2b7
check_cached_file go go1.26.1.linux-arm64.tar.gz a290581cfe4fe28ddd737dde3095f3dbeb7f2e4065cab4eae44dfc53b760c2f7
check_cached_file go go1.26.1.linux-amd64.tar.gz 031f088e5d955bab8657ede27ad4e3bc5b7c1ba281f05f245bcc304f327c987a
check_cached_file rust rust-1.95.0-aarch64-jonerix-linux-musl.tar.gz 622c8491278486889db2a52a58a19d1a477f1bd101c0a630e416f1cc3922ae8a
check_cached_file rust rust-1.95.0-x86_64-jonerix-linux-musl.tar.gz 3eec54cf7cf145fd6a7a40aee605144da82dd48192380c4f86f7947abf8c06dc
check_cached_file rustup rustup-init-1.29.0-aarch64-unknown-linux-musl 88761caacddb92cd79b0b1f939f3990ba1997d701a38b3e8dd6746a562f2a759
check_cached_file rustup rustup-init-1.29.0-x86_64-unknown-linux-musl 9cd3fda5fd293890e36ab271af6a786ee22084b5f6c2b83fd8323cec6f0992c1

if [ "$failures" -ne 0 ]; then
    printf 'vendored source check failed: %s issue(s)\n' "$failures" >&2
    exit 1
fi

printf 'vendored source check passed\n'
