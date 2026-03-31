#!/bin/sh
# size-report.sh — Measure and report jonerix rootfs size breakdown
#
# Analyzes a jonerix rootfs (directory or tarball) and reports:
#   - Size per installed package (from jpkg database)
#   - Size per top-level directory
#   - Largest individual files
#   - Total rootfs size
#   - Comparison against size targets
#
# Usage:
#   size-report.sh <rootfs-dir>          # analyze a rootfs directory
#   size-report.sh <rootfs-tarball>      # analyze a rootfs tarball
#   size-report.sh --target minimal      # compare against minimal (8MB) target
#   size-report.sh --target server       # compare against server (15MB) target
#
# Exit codes:
#   0 — report generated (and within target if --target used)
#   1 — over target size
#   2 — usage error
#
# Part of jonerix — MIT License

set -eu

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

# Size targets in bytes
TARGET_MINIMAL=$((8 * 1024 * 1024))       # 8 MB
TARGET_SERVER=$((15 * 1024 * 1024))        # 15 MB
TARGET_FULL=$((500 * 1024 * 1024))         # 500 MB

ROOTFS=""
TARGET=""
TOP_FILES=20
WORK_DIR=""

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

die() {
    printf "size-report: error: %s\n" "$1" >&2
    exit 2
}

info() {
    printf "%s\n" "$1"
}

cleanup() {
    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi
}

trap cleanup EXIT INT TERM

# Format bytes as human-readable
human_size() {
    local bytes="$1"
    if [ "$bytes" -ge $((1024 * 1024 * 1024)) ]; then
        printf '%d.%d GB' "$((bytes / 1073741824))" "$(( (bytes % 1073741824) * 10 / 1073741824 ))"
    elif [ "$bytes" -ge $((1024 * 1024)) ]; then
        printf '%d.%d MB' "$((bytes / 1048576))" "$(( (bytes % 1048576) * 10 / 1048576 ))"
    elif [ "$bytes" -ge 1024 ]; then
        printf '%d.%d KB' "$((bytes / 1024))" "$(( (bytes % 1024) * 10 / 1024 ))"
    else
        printf '%d B' "$bytes"
    fi
}

# Print a bar chart line
bar_chart() {
    local label="$1"
    local value="$2"
    local total="$3"
    local max_width=40

    local pct=0
    if [ "$total" -gt 0 ]; then
        pct=$(( (value * 100) / total ))
    fi

    local bar_len=0
    if [ "$total" -gt 0 ]; then
        bar_len=$(( (value * max_width) / total ))
    fi
    [ "$bar_len" -lt 1 ] && [ "$value" -gt 0 ] && bar_len=1

    local bar=""
    local i=0
    while [ "$i" -lt "$bar_len" ]; do
        bar="${bar}#"
        i=$((i + 1))
    done

    printf "  %-20s %8s %3d%% |%-${max_width}s|\n" "$label" "$(human_size "$value")" "$pct" "$bar"
}

# ---------------------------------------------------------------------------
# Analysis functions
# ---------------------------------------------------------------------------

# Get total size of a directory
dir_size_bytes() {
    local dir="$1"
    # Use du -sb if available (Linux), fall back to du -sk * 1024
    if du -sb "$dir" >/dev/null 2>&1; then
        du -sb "$dir" 2>/dev/null | cut -f1
    else
        echo $(( $(du -sk "$dir" 2>/dev/null | cut -f1) * 1024 ))
    fi
}

# Analyze by jpkg package database
analyze_packages() {
    local rootfs="$1"
    local db_dir="${rootfs}/var/db/jpkg"

    if [ ! -d "$db_dir" ]; then
        info "  (jpkg database not found — skipping package breakdown)"
        return
    fi

    local pkg_count=0
    local pkg_sizes=""

    for pkg_dir in "$db_dir"/*/; do
        [ -d "$pkg_dir" ] || continue
        local pkg_name
        pkg_name="$(basename "$pkg_dir")"

        local pkg_size=0

        # Read file list from package manifest
        if [ -f "$pkg_dir/files" ]; then
            while IFS= read -r file; do
                [ -z "$file" ] && continue
                local full_path="${rootfs}/${file}"
                if [ -f "$full_path" ]; then
                    local fsize
                    fsize="$(wc -c < "$full_path" 2>/dev/null | tr -d ' ')"
                    pkg_size=$((pkg_size + fsize))
                fi
            done < "$pkg_dir/files"
        else
            # Estimate from metadata if no file list
            if [ -f "$pkg_dir/metadata" ]; then
                local size_field
                size_field="$(grep -i '^size' "$pkg_dir/metadata" | head -1 | sed 's/^[^=]*=[[:space:]]*//' | tr -d '"'"'" 2>/dev/null || echo '0')"
                pkg_size="${size_field:-0}"
            fi
        fi

        pkg_sizes="${pkg_sizes}${pkg_size}:${pkg_name}\n"
        pkg_count=$((pkg_count + 1))
    done

    if [ "$pkg_count" -eq 0 ]; then
        info "  (no packages found in jpkg database)"
        return
    fi

    local total
    total="$(dir_size_bytes "$rootfs")"

    info "Packages ($pkg_count installed):"
    info ""

    # Sort packages by size (descending) and display
    printf '%b' "$pkg_sizes" | sort -t: -k1 -rn | while IFS=: read -r size name; do
        [ -z "$name" ] && continue
        bar_chart "$name" "$size" "$total"
    done
}

# Analyze by top-level directory
analyze_directories() {
    local rootfs="$1"
    local total
    total="$(dir_size_bytes "$rootfs")"

    info "Directory breakdown:"
    info ""

    local dir_sizes=""

    for dir in "$rootfs"/*/; do
        [ -d "$dir" ] || continue
        local dname
        dname="$(basename "$dir")"

        # Skip pseudo-filesystems and empty dirs
        case "$dname" in
            proc|sys|dev|run) continue ;;
        esac

        local dsize
        dsize="$(dir_size_bytes "$dir")"
        dir_sizes="${dir_sizes}${dsize}:/${dname}\n"
    done

    # Add root-level files (not in subdirectories)
    local root_files_size=0
    for f in "$rootfs"/*; do
        [ -f "$f" ] || continue
        local fsize
        fsize="$(wc -c < "$f" 2>/dev/null | tr -d ' ')"
        root_files_size=$((root_files_size + fsize))
    done
    if [ "$root_files_size" -gt 0 ]; then
        dir_sizes="${dir_sizes}${root_files_size}:/(root files)\n"
    fi

    printf '%b' "$dir_sizes" | sort -t: -k1 -rn | while IFS=: read -r size name; do
        [ -z "$name" ] && continue
        bar_chart "$name" "$size" "$total"
    done
}

# Find largest individual files
analyze_largest_files() {
    local rootfs="$1"
    local count="$2"

    info "Largest files (top $count):"
    info ""

    # Find all regular files, get their sizes, sort descending
    find "$rootfs" -type f -not -path '*/proc/*' -not -path '*/sys/*' \
        -not -path '*/dev/*' 2>/dev/null | while IFS= read -r filepath; do
        local fsize
        fsize="$(wc -c < "$filepath" 2>/dev/null | tr -d ' ')"
        # Strip rootfs prefix for display
        local display_path="${filepath#"$rootfs"}"
        printf '%d\t%s\n' "$fsize" "$display_path"
    done | sort -rn | head -"$count" | while IFS='	' read -r size path; do
        printf "  %8s  %s\n" "$(human_size "$size")" "$path"
    done
}

# File type breakdown
analyze_file_types() {
    local rootfs="$1"

    info "File type breakdown:"
    info ""

    local elf_size=0 elf_count=0
    local script_size=0 script_count=0
    local config_size=0 config_count=0
    local other_size=0 other_count=0
    local total_size=0 total_count=0

    find "$rootfs" -type f -not -path '*/proc/*' -not -path '*/sys/*' \
        -not -path '*/dev/*' 2>/dev/null | while IFS= read -r filepath; do
        local fsize
        fsize="$(wc -c < "$filepath" 2>/dev/null | tr -d ' ')"
        printf '%d\t%s\n' "$fsize" "$filepath"
    done | while IFS='	' read -r size path; do
        total_size=$((total_size + size))
        total_count=$((total_count + 1))

        # Categorize by file type
        local ftype
        ftype="$(file -b "$path" 2>/dev/null | head -1 || echo 'unknown')"

        case "$ftype" in
            ELF*|*executable*|*shared\ object*)
                elf_size=$((elf_size + size))
                elf_count=$((elf_count + 1))
                ;;
            *script*|*text*executable*)
                script_size=$((script_size + size))
                script_count=$((script_count + 1))
                ;;
            *text*|*ASCII*|*UTF-8*)
                config_size=$((config_size + size))
                config_count=$((config_count + 1))
                ;;
            *)
                other_size=$((other_size + size))
                other_count=$((other_count + 1))
                ;;
        esac
    done

    # This subshell approach loses state, so use a temp file instead
    local tmpfile
    tmpfile="$(mktemp /tmp/jonerix-size.XXXXXX)"

    find "$rootfs" -type f -not -path '*/proc/*' -not -path '*/sys/*' \
        -not -path '*/dev/*' -exec sh -c '
        for f do
            s=$(wc -c < "$f" | tr -d " ")
            t=$(file -b "$f" 2>/dev/null | head -1)
            case "$t" in
                ELF*|*executable*|*"shared object"*) printf "elf %d\n" "$s" ;;
                *script*|*"text executable"*) printf "script %d\n" "$s" ;;
                *text*|*ASCII*|*UTF-8*) printf "config %d\n" "$s" ;;
                *) printf "other %d\n" "$s" ;;
            esac
        done
    ' _ {} + > "$tmpfile" 2>/dev/null || true

    local elf_total=0 script_total=0 config_total=0 other_total=0 grand_total=0

    while read -r ftype fsize; do
        grand_total=$((grand_total + fsize))
        case "$ftype" in
            elf) elf_total=$((elf_total + fsize)) ;;
            script) script_total=$((script_total + fsize)) ;;
            config) config_total=$((config_total + fsize)) ;;
            other) other_total=$((other_total + fsize)) ;;
        esac
    done < "$tmpfile"

    rm -f "$tmpfile"

    if [ "$grand_total" -gt 0 ]; then
        bar_chart "ELF binaries" "$elf_total" "$grand_total"
        bar_chart "Shell scripts" "$script_total" "$grand_total"
        bar_chart "Config/text" "$config_total" "$grand_total"
        bar_chart "Other" "$other_total" "$grand_total"
    fi
}

# Compare against target
compare_target() {
    local rootfs="$1"
    local target_name="$2"
    local target_bytes

    case "$target_name" in
        minimal) target_bytes="$TARGET_MINIMAL" ;;
        server)  target_bytes="$TARGET_SERVER" ;;
        full)    target_bytes="$TARGET_FULL" ;;
        *)       die "Unknown target: $target_name (use: minimal, server, full)" ;;
    esac

    local actual
    actual="$(dir_size_bytes "$rootfs")"

    info ""
    info "=== Target Comparison ==="
    info "  Target:  $(human_size "$target_bytes") ($target_name)"
    info "  Actual:  $(human_size "$actual")"

    if [ "$actual" -le "$target_bytes" ]; then
        local under=$((target_bytes - actual))
        info "  Status:  \033[32mWITHIN TARGET\033[0m ($(human_size "$under") under)"
        return 0
    else
        local over=$((actual - target_bytes))
        info "  Status:  \033[31mOVER TARGET\033[0m by $(human_size "$over")"
        return 1
    fi
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    # Parse arguments
    while [ $# -gt 0 ]; do
        case "$1" in
            --target)
                shift
                TARGET="${1:?--target requires a value (minimal, server, full)}"
                ;;
            --top)
                shift
                TOP_FILES="${1:?--top requires a number}"
                ;;
            --help|-h)
                printf 'Usage: size-report.sh [options] <rootfs-dir-or-tarball>\n'
                printf '\n'
                printf 'Options:\n'
                printf '  --target <name>  Compare against size target (minimal, server, full)\n'
                printf '  --top <n>        Number of largest files to show (default: 20)\n'
                printf '\n'
                printf 'Targets:\n'
                printf '  minimal  8 MB  (toybox + musl + jpkg)\n'
                printf '  server   15 MB (adds dropbear, curl, OpenRC, socklog)\n'
                printf '  full     500 MB (adds LLVM/Clang)\n'
                exit 0
                ;;
            -*)
                die "Unknown option: $1"
                ;;
            *)
                ROOTFS="$1"
                ;;
        esac
        shift
    done

    [ -n "$ROOTFS" ] || die "rootfs path required (directory or tarball)"

    # If the input is a tarball, extract it to a temp directory
    if [ -f "$ROOTFS" ] && [ ! -d "$ROOTFS" ]; then
        WORK_DIR="$(mktemp -d /tmp/jonerix-size-report.XXXXXX)"
        info "Extracting tarball to temporary directory..."

        case "$ROOTFS" in
            *.tar.zst|*.tar.zstd)
                zstd -dc "$ROOTFS" | tar -xf - -C "$WORK_DIR"
                ;;
            *.tar.gz|*.tgz)
                tar -xzf "$ROOTFS" -C "$WORK_DIR"
                ;;
            *.tar.xz)
                tar -xJf "$ROOTFS" -C "$WORK_DIR"
                ;;
            *.tar)
                tar -xf "$ROOTFS" -C "$WORK_DIR"
                ;;
            *)
                die "unsupported tarball format: $ROOTFS"
                ;;
        esac

        ROOTFS="$WORK_DIR"
    fi

    [ -d "$ROOTFS" ] || die "rootfs directory not found: $ROOTFS"

    # Header
    local total_size
    total_size="$(dir_size_bytes "$ROOTFS")"

    printf '╔══════════════════════════════════════════════════════════════════╗\n'
    printf '║              jonerix rootfs size report                        ║\n'
    printf '╠══════════════════════════════════════════════════════════════════╣\n'
    printf '║  Path:  %-55s ║\n' "$ROOTFS"
    printf '║  Total: %-55s ║\n' "$(human_size "$total_size")"
    printf '╚══════════════════════════════════════════════════════════════════╝\n'
    printf '\n'

    # Run analyses
    analyze_directories "$ROOTFS"
    printf '\n'

    analyze_packages "$ROOTFS"
    printf '\n'

    analyze_largest_files "$ROOTFS" "$TOP_FILES"
    printf '\n'

    # Only run file type analysis if 'file' command is available
    if command -v file >/dev/null 2>&1; then
        analyze_file_types "$ROOTFS"
        printf '\n'
    fi

    # Target comparison
    local exit_code=0
    if [ -n "$TARGET" ]; then
        compare_target "$ROOTFS" "$TARGET" || exit_code=1
    fi

    exit "$exit_code"
}

main "$@"
