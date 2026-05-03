#!/bin/sh
# Verify Go package builds are locked to vendored module inputs.
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

check_go_flags() {
    recipe=$1

    grep -n 'go ' "$recipe" 2>/dev/null | while IFS= read -r row; do
        line=${row%%:*}
        text=${row#*:}
        trimmed=$(printf '%s\n' "$text" | sed 's/^[	 ]*//')

        case "$trimmed" in
            \#*|echo\ *|printf\ *) continue ;;
        esac

        with_prefix=" $text"

        case "$with_prefix" in
            *[!A-Za-z0-9_]go\ mod\ download*|*[!A-Za-z0-9_]go\ get\ *|*[!A-Za-z0-9_]go\ install\ *)
                printf 'GO: %s:%s uses network-prone command: %s\n' "$recipe" "$line" "$text" >&2
                exit 2
                ;;
            *[!A-Za-z0-9_]go\ build*|*[!A-Za-z0-9_]go\ test*)
                case "$text" in
                    *'-mod=vendor'*) ;;
                    *)
                        printf 'GO: %s:%s missing -mod=vendor: %s\n' "$recipe" "$line" "$text" >&2
                        exit 2
                        ;;
                esac
                ;;
        esac
    done
}

check_vendor_tarball() {
    label=$1
    file=$2
    src=${SOURCES}/${file}

    if [ ! -f "$src" ]; then
        fail "GO: missing vendored tarball for $label: $file"
        return
    fi

    if ! tar tf "$src" | grep '/go\.mod$' >/dev/null 2>&1; then
        fail "GO: $label tarball has no go.mod: $file"
    fi
    if ! tar tf "$src" | grep '/vendor/modules\.txt$' >/dev/null 2>&1; then
        fail "GO: $label tarball has no vendor/modules.txt: $file"
    fi
}

for recipe in "${RECIPES}"/core/*/recipe.toml \
              "${RECIPES}"/develop/*/recipe.toml \
              "${RECIPES}"/extra/*/recipe.toml; do
    [ -f "$recipe" ] || continue
    if ! check_go_flags "$recipe"; then
        failures=$((failures + 1))
    fi
done

check_vendor_tarball iproute-go iproute-go-0.16.0.tar.gz
check_vendor_tarball micro micro-2.0.15.tar.gz
check_vendor_tarball containerd containerd-2.2.2.tar.gz
check_vendor_tarball cni-plugins cni-plugins-1.9.1.tar.gz
check_vendor_tarball derper derper-1.96.5.tar.gz
check_vendor_tarball headscale headscale-0.28.0.tar.gz
check_vendor_tarball nerdctl nerdctl-2.2.1.tar.gz
check_vendor_tarball runc runc-1.4.1.tar.gz

if [ "$failures" -ne 0 ]; then
    printf 'go offline check failed: %s issue(s)\n' "$failures" >&2
    exit 1
fi

printf 'go offline check passed\n'
