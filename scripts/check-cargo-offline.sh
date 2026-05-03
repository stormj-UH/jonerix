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

check_vendor_tarball() {
    label=$1
    file=$2
    src=${SOURCES}/${file}

    if [ ! -f "$src" ]; then
        fail "CARGO: missing vendored tarball for $label: $file"
        return
    fi

    if ! tar tf "$src" | grep '/Cargo.lock$' >/dev/null 2>&1; then
        fail "CARGO: $label tarball has no Cargo.lock: $file"
    fi
    if ! tar tf "$src" | grep '/vendor/' >/dev/null 2>&1; then
        fail "CARGO: $label tarball has no vendor/: $file"
    fi
    if ! tar tf "$src" | grep '/\.cargo/config\.toml$' >/dev/null 2>&1; then
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

    lock_path=$(tar tf "$src" | grep '/Cargo\.lock$' | head -n 1 || true)
    [ -n "$lock_path" ] || return 0

    if tar xOf "$src" "$lock_path" | grep '^source = ' >/dev/null 2>&1; then
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

# Tarball packages with registry dependencies must carry Cargo.lock,
# vendor/, and source replacement config.
check_vendor_tarball brash brash-1.0.10.tar.gz
check_vendor_tarball exproxide exproxide-0.1.0.tar.gz
check_vendor_tarball gitoxide gitoxide-0.52.0.tar.gz
check_vendor_tarball jmake jmake-v1.1.14.tar.gz
check_vendor_tarball ripgrep ripgrep-15.1.0.tar.gz
check_vendor_tarball stormwall stormwall-1.0.4.tar.gz
check_vendor_tarball uutils uutils-0.7.0-r1.tar.gz

check_path_only_tarball anvil anvil-0.2.1-r1.tar.gz
check_path_only_tarball jfsck jfsck-0.1.0.tar.gz
check_path_only_tarball lsusb-rs lsusb-rs-0.1.0-r0.tar.gz
check_path_only_tarball m4oxide m4oxide-0.1.0-r0.tar.gz

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
