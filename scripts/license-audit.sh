#!/bin/sh
# license-audit.sh — Verify all installed packages have permissive licenses
#
# Scans the jonerix package database and/or package recipe files to verify
# that every component uses a permissive license. Flags any GPL, LGPL, or
# AGPL-licensed packages and exits non-zero if violations are found.
#
# Accepted licenses: the PERMISSIVE_LICENSES list below, which mirrors
# PERMISSIVE_LICENSES in packages/core/jpkg/src/util.rs (the gate
# `jpkg build` applies). SPDX expressions are evaluated like jpkg does:
# `A OR B` passes when either side passes, `A AND B` only when both do.
# So perl's "Artistic-1.0-Perl OR GPL-1.0-or-later" passes on its
# Artistic side, while "MIT AND GPL-2.0-only" fails.
#
# Exceptions:
#   Linux kernel (GPLv2) is the documented sole package exception.
#   ca-certificates may carry MPL-2.0 (the Mozilla CA bundle is data);
#   MPL-2.0 is not accepted for any other package (LICENSE_EXCEPTIONS).
#
# Usage:
#   license-audit.sh                     # audit installed packages (jpkg DB)
#   license-audit.sh --recipes           # audit package recipe files
#   license-audit.sh --rootfs <path>     # audit a rootfs directory
#   license-audit.sh --verbose           # show all packages, not just violations
#
# Exit codes:
#   0 — all licenses are permissive (or only known exceptions)
#   1 — license violations found
#   2 — usage error or missing dependencies
#
# Part of jonerix — MIT License

set -eu

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

JPKG_DB="/var/db/jpkg"
RECIPES_DIR=""
ROOTFS_DIR=""
VERBOSE=0
MODE="installed"

# Known permissive license identifiers (case-insensitive matching).
# Keep in sync with PERMISSIVE_LICENSES in packages/core/jpkg/src/util.rs.
PERMISSIVE_LICENSES="
MIT
BSD-2-Clause
BSD-3-Clause
BSD-3-Clause-flex
BSD-2-Clause-Patent
BSD-2
BSD-3
BSD
ISC
Apache-2.0
Apache
0BSD
CC0-1.0
CC0
public-domain
Public-Domain
public domain
Zlib
zlib
curl
MirOS
OpenSSL
SSLeay
Unlicense
WTFPL
PSF-2.0
BSL-1.0
Artistic-2.0
Artistic-1.0-Perl
Ruby
Info-ZIP
bzip2-1.0.6
FTL
HPND
Unicode-DFS-2016
Unicode-3.0
libpng-2.0
"

# Known exceptions (packages allowed to be non-permissive)
EXCEPTIONS="
linux
"

# Per-package license exceptions, "package:license" (exact license match,
# case-insensitive). Keep in sync with LICENSE_EXCEPTIONS in
# packages/core/jpkg/src/util.rs.
LICENSE_EXCEPTIONS="
ca-certificates:MPL-2.0
"

# Forbidden license patterns (substring match, checked after the permissive
# list and the per-package exceptions). MPL is file-level copyleft.
FORBIDDEN_PATTERNS="GPL LGPL AGPL SSPL EUPL MPL"

# Counters
TOTAL=0
PASS=0
FAIL=0
EXCEPTIONS_FOUND=0
UNKNOWN=0

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

die() {
    printf "license-audit: error: %s\n" "$1" >&2
    exit 2
}

info() {
    printf "license-audit: %s\n" "$1"
}

warn() {
    printf "license-audit: WARNING: %s\n" "$1" >&2
}

# Print colored status
status_ok() {
    printf "  \033[32mOK\033[0m    %-30s %s\n" "$1" "$2"
}

status_fail() {
    printf "  \033[31mFAIL\033[0m  %-30s %s\n" "$1" "$2"
}

status_warn() {
    printf "  \033[33mWARN\033[0m  %-30s %s\n" "$1" "$2"
}

status_skip() {
    printf "  \033[36mSKIP\033[0m  %-30s %s (exception)\n" "$1" "$2"
}

# Trim leading/trailing whitespace.
trim_license() {
    printf '%s' "$1" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//'
}

# Print "yes" when a single license identifier is on the permissive list.
# A parenthesised single identifier, "(MIT)", is accepted too.
license_atom_is_permissive() {
    atom_normalized="$(trim_license "$1" | sed 's/^(//; s/)$//' | tr '[:upper:]' '[:lower:]')"
    [ -n "$atom_normalized" ] || return 0

    printf '%s\n' "$PERMISSIVE_LICENSES" | while IFS= read -r perm; do
        [ -z "$perm" ] && continue
        perm_lower="$(printf '%s' "$perm" | tr '[:upper:]' '[:lower:]')"
        if [ "$atom_normalized" = "$perm_lower" ]; then
            printf 'yes'
            return 0
        fi
    done
    return 0
}

# Print "yes" when every AND-separated identifier is permissive.
license_and_is_permissive() {
    la_rest="$(trim_license "$1")"

    while :; do
        case "$la_rest" in
            *" AND "*)
                la_part="${la_rest%% AND *}"
                la_rest="${la_rest#* AND }"
                ;;
            *)
                la_part="$la_rest"
                la_rest=""
                ;;
        esac

        if [ "$(license_atom_is_permissive "$la_part")" != "yes" ]; then
            return 0
        fi

        [ -z "$la_rest" ] && break
    done

    printf 'yes'
    return 0
}

# Print "yes" when a license expression is permissive: any OR alternative
# whose AND terms are all permissive. Mirrors jpkg's license gate for the
# flat expressions recipes use; grouped sub-expressions such as
# "(A OR B) AND C" are not parsed here (jpkg build still checks them).
is_permissive() {
    li_rest="$(trim_license "$1")"

    while :; do
        case "$li_rest" in
            *" OR "*)
                li_part="${li_rest%% OR *}"
                li_rest="${li_rest#* OR }"
                ;;
            *)
                li_part="$li_rest"
                li_rest=""
                ;;
        esac

        if [ "$(license_and_is_permissive "$li_part")" = "yes" ]; then
            printf 'yes'
            return 0
        fi

        [ -z "$li_rest" ] && break
    done

    return 0
}

# Succeed when package $1 has a per-package exception for license $2.
is_license_exception() {
    le_pkg="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')"
    le_lic="$(trim_license "$2" | tr '[:upper:]' '[:lower:]')"

    for le_entry in $LICENSE_EXCEPTIONS; do
        le_entry="$(printf '%s' "$le_entry" | tr '[:upper:]' '[:lower:]')"
        if [ "$le_entry" = "$le_pkg:$le_lic" ]; then
            return 0
        fi
    done

    return 1
}

# Check if a license string contains forbidden patterns
is_forbidden() {
    local license="$1"
    local normalized
    normalized="$(printf '%s' "$license" | tr '[:lower:]' '[:upper:]')"

    for pattern in $FORBIDDEN_PATTERNS; do
        case "$normalized" in
            *"$pattern"*)
                # Exception: "LGPL" inside "MIT AND LGPL" would still match
                # But that's correct — any GPL contamination is a violation
                return 0
                ;;
        esac
    done

    return 1
}

# Check if a package is a known exception
is_exception() {
    local pkg_name="$1"
    local normalized
    normalized="$(printf '%s' "$pkg_name" | tr '[:upper:]' '[:lower:]')"

    for exc in $EXCEPTIONS; do
        [ -z "$exc" ] && continue
        if [ "$normalized" = "$exc" ]; then
            return 0
        fi
    done

    return 1
}

# Audit a single package
audit_package() {
    local name="$1"
    local license="$2"

    TOTAL=$((TOTAL + 1))

    # Check if this is a known exception
    if is_exception "$name"; then
        EXCEPTIONS_FOUND=$((EXCEPTIONS_FOUND + 1))
        if [ "$VERBOSE" -eq 1 ]; then
            status_skip "$name" "$license"
        fi
        return 0
    fi

    # Per-package license exception (ca-certificates' MPL-2.0 data bundle)
    if is_license_exception "$name" "$license"; then
        PASS=$((PASS + 1))
        if [ "$VERBOSE" -eq 1 ]; then
            status_ok "$name" "$license (per-package exception)"
        fi
        return 0
    fi

    # Check against the permissive list first, so a permissive OR
    # alternative (perl's "Artistic-1.0-Perl OR GPL-1.0-or-later") passes
    # before the substring match below sees "GPL" in it.
    local result
    result="$(is_permissive "$license")"
    if [ "$result" = "yes" ]; then
        PASS=$((PASS + 1))
        if [ "$VERBOSE" -eq 1 ]; then
            status_ok "$name" "$license"
        fi
        return 0
    fi

    # Check for forbidden license patterns
    if is_forbidden "$license"; then
        FAIL=$((FAIL + 1))
        status_fail "$name" "$license"
        return 1
    fi

    # Unknown license — warn but don't fail
    UNKNOWN=$((UNKNOWN + 1))
    status_warn "$name" "$license (unrecognized — verify manually)"
    return 0
}

# ---------------------------------------------------------------------------
# Audit modes
# ---------------------------------------------------------------------------

audit_installed() {
    local db_path="${ROOTFS_DIR:+${ROOTFS_DIR}}${JPKG_DB}"

    if [ ! -d "$db_path" ]; then
        warn "Package database not found at $db_path"
        warn "Falling back to recipe scan"
        audit_recipes
        return $?
    fi

    info "Auditing installed packages from $db_path"
    printf "\n"

    local had_failure=0

    # jpkg 2.x keeps one directory per package under installed/, holding
    # metadata.toml; older layouts kept <pkg>/metadata or <pkg>/PKG directly
    # under the DB root.
    local pkg_root="$db_path"
    [ -d "$db_path/installed" ] && pkg_root="$db_path/installed"

    # Each installed package has a metadata file in the DB
    for pkg_dir in "$pkg_root"/*/; do
        [ -d "$pkg_dir" ] || continue
        local pkg_name
        pkg_name="$(basename "$pkg_dir")"

        local license=""

        # Read the license from the package metadata (TOML-like)
        for meta_file in metadata.toml metadata PKG; do
            if [ -f "$pkg_dir/$meta_file" ]; then
                license="$(grep -i '^license' "$pkg_dir/$meta_file" | head -1 | sed 's/^[^=]*=[[:space:]]*//' | tr -d '"'"'")"
                break
            fi
        done

        if [ -z "$license" ]; then
            license="UNKNOWN"
        fi

        audit_package "$pkg_name" "$license" || had_failure=1
    done

    return $had_failure
}

audit_recipes() {
    local script_dir
    script_dir="$(cd "$(dirname "$0")" && pwd)"
    local project_root="$(dirname "$script_dir")"

    # Search all recipe directories: core, develop, extra
    local recipes_paths=""
    for subdir in core develop extra; do
        local candidate="${RECIPES_DIR:-$project_root/packages}/$subdir"
        [ -d "$candidate" ] && recipes_paths="$recipes_paths $candidate"
    done

    if [ -z "$recipes_paths" ]; then
        die "No package recipe directories found. Specify with --recipes <dir>"
    fi

    info "Auditing package recipes in packages/{core,develop,extra}"
    printf "\n"

    local had_failure=0

    for recipes_path in $recipes_paths; do
    for recipe_dir in "$recipes_path"/*/; do
        [ -d "$recipe_dir" ] || continue
        local pkg_name
        pkg_name="$(basename "$recipe_dir")"

        local license=""

        # Look for license in Makefile
        if [ -f "$recipe_dir/Makefile" ]; then
            license="$(grep '^PKG_LICENSE' "$recipe_dir/Makefile" | head -1 | sed 's/^[^=]*=[[:space:]]*//')"
        fi

        # Look for license in build.toml or recipe.toml
        for f in build.toml recipe.toml package.toml; do
            if [ -f "$recipe_dir/$f" ] && [ -z "$license" ]; then
                license="$(grep -i '^license' "$recipe_dir/$f" | head -1 | sed 's/^[^=]*=[[:space:]]*//' | tr -d '"'"'")"
            fi
        done

        if [ -z "$license" ]; then
            license="UNKNOWN"
        fi

        audit_package "$pkg_name" "$license" || had_failure=1
    done
    done

    return $had_failure
}

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

print_summary() {
    printf "\n"
    printf "=== License Audit Summary ===\n"
    printf "  Total packages:    %d\n" "$TOTAL"
    printf "  Permissive (OK):   %d\n" "$PASS"
    printf "  Violations (FAIL): %d\n" "$FAIL"
    printf "  Exceptions:        %d\n" "$EXCEPTIONS_FOUND"
    printf "  Unknown:           %d\n" "$UNKNOWN"
    printf "\n"

    if [ "$FAIL" -gt 0 ]; then
        printf "\033[31mFAILED: %d package(s) have non-permissive licenses.\033[0m\n" "$FAIL"
        printf "These packages MUST be replaced or removed from the jonerix image.\n"
        return 1
    elif [ "$UNKNOWN" -gt 0 ]; then
        printf "\033[33mWARNING: %d package(s) have unrecognized licenses.\033[0m\n" "$UNKNOWN"
        printf "Please verify these manually.\n"
        return 0
    else
        printf "\033[32mPASSED: All packages have permissive licenses.\033[0m\n"
        return 0
    fi
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    # Parse arguments
    while [ $# -gt 0 ]; do
        case "$1" in
            --recipes)
                MODE="recipes"
                if [ $# -gt 1 ] && [ "${2#-}" = "$2" ]; then
                    shift
                    RECIPES_DIR="$1"
                fi
                ;;
            --rootfs)
                shift
                ROOTFS_DIR="${1:?--rootfs requires a path}"
                ;;
            --verbose|-v)
                VERBOSE=1
                ;;
            --help|-h)
                printf 'Usage: license-audit.sh [--recipes [dir]] [--rootfs <path>] [--verbose]\n'
                printf '\n'
                printf 'Modes:\n'
                printf '  (default)        Audit installed packages from jpkg database\n'
                printf '  --recipes [dir]  Audit package recipe Makefiles\n'
                printf '  --rootfs <path>  Audit packages in a rootfs directory\n'
                printf '\n'
                printf 'Options:\n'
                printf '  --verbose, -v    Show all packages, not just violations\n'
                exit 0
                ;;
            *)
                die "Unknown argument: $1"
                ;;
        esac
        shift
    done

    info "jonerix license audit"
    info "Policy: all runtime packages must be permissively licensed"
    info "Allowed: MIT, BSD, ISC, Apache-2.0, 0BSD, CC0, Zlib, public domain, PSF-2.0, Artistic (Perl, 2.0) and the rest of jpkg's list; SPDX AND/OR evaluated"
    info "Forbidden: GPL, LGPL, AGPL (unless a permissive OR alternative exists)"
    info "Exceptions: Linux kernel (GPLv2); ca-certificates may carry MPL-2.0"

    local had_failure=0

    case "$MODE" in
        installed)
            audit_installed || had_failure=1
            ;;
        recipes)
            audit_recipes || had_failure=1
            ;;
    esac

    print_summary || had_failure=1

    exit "$had_failure"
}

main "$@"
