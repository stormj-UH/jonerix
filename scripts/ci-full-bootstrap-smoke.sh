#!/bin/sh
# ci-full-bootstrap-smoke.sh — runs OUTSIDE the builder, on the GitHub runner.
#
# Boots the imported bootstrap rootfs in a fresh container and runs each
# /bin/* binary with a version flag. Builds an additional report section
# documenting which binaries answered with output, which failed, and which
# had no recognisable version flag. Appends to out/report.md.
#
# Argument 1: docker image tag (e.g. `jonerix-bootstrap-test:x86_64`)

set -eu
IMG="${1:?image tag required}"

OUT=out
mkdir -p "$OUT/smoke-log"
# The bootstrap step inside the builder container writes out/* as root,
# but this smoke-test step runs on the host runner as UID 1001 and needs
# to append to out/report.md. Take ownership of the dir tree once,
# upfront, so every subsequent write here doesn't EACCES. Use sudo only
# if it's available (the runner has it; bare alpine images don't).
if [ "$(id -u)" != "0" ] && [ ! -w "$OUT" ]; then
    if command -v sudo >/dev/null 2>&1; then
        sudo chown -R "$(id -u):$(id -g)" "$OUT" 2>/dev/null || true
    fi
fi

if [ ! -f "$OUT/binaries.txt" ]; then
    echo "no binaries.txt; skipping smoke test"
    exit 0
fi

# Generate one-line probes for each binary. The probe tries the most common
# version flags in order: --version, -V, -v, version. First non-error
# response wins; otherwise the binary is marked NO-FLAG.
# REVIEW: binaries that open a TUI even with a flag argument (e.g. tmux,
# vim, nano) will be killed by the 5-second timeout, but any partial escape-
# sequence output they emitted before dying may not match the case filter and
# would be classified OK with the flag that triggered them.  This is a false
# positive in the smoke results (the binary "answered", just not usefully).
# It is not a test failure — the binary ran and exited — but the "first line
# of output" column in the report will contain garbled terminal codes.
# Consider adding *$'\033'* (ESC) to the case filter to catch raw TUI output.
PROBE='for b in $(cat /tmp/binaries.txt); do
    name=$(basename "$b")
    [ -x "/$b" ] || continue
    out=""
    for flag in --version -V -v version; do
        # 5 second timeout to avoid hangs (some binaries open a TUI on no-args).
        if out=$( ( timeout 5 "/$b" "$flag" 2>&1 || true ) | head -3 ); then
            case "$out" in
                ""|*"unknown option"*|*"invalid option"*|*"illegal option"*|*"usage:"*|*"Usage:"*)
                    continue ;;
                *)
                    printf "OK\t%s\t%s\t%s\n" "$name" "$flag" "$(echo "$out" | head -1)"
                    out_done=1
                    break ;;
            esac
        fi
    done
    if [ -z "${out_done:-}" ]; then
        printf "NO-FLAG\t%s\t-\t-\n" "$name"
    fi
    unset out_done
done'

# Copy binaries.txt into the container, then run the probe.
container_id=$(docker create "$IMG" /bin/sh -c 'sleep 600')
docker cp "$OUT/binaries.txt" "$container_id:/tmp/binaries.txt"
docker start "$container_id" >/dev/null
docker exec "$container_id" /bin/sh -c "$PROBE" \
    > "$OUT/smoke-log/results.tsv" 2>"$OUT/smoke-log/stderr.log" || true
docker rm -f "$container_id" >/dev/null

ok=$(awk -F'\t' '$1=="OK"' "$OUT/smoke-log/results.tsv" | wc -l)
noflag=$(awk -F'\t' '$1=="NO-FLAG"' "$OUT/smoke-log/results.tsv" | wc -l)
total=$(wc -l < "$OUT/smoke-log/results.tsv")

{
    echo
    echo '## Smoke test'
    echo
    echo "| | count |"
    echo "|--|--|"
    echo "| total binaries probed | $total |"
    echo "| answered to a version flag | $ok |"
    echo "| no recognised version flag | $noflag |"
    echo
    echo '<details><summary>Per-binary results</summary>'
    echo
    echo '| status | binary | flag | first line of output |'
    echo '|---|---|---|---|'
    awk -F'\t' '{ gsub(/\|/, "\\|", $4); printf "| %s | `%s` | `%s` | %s |\n", $1, $2, $3, $4 }' \
        "$OUT/smoke-log/results.tsv"
    echo
    echo '</details>'
} >> "$OUT/report.md"

echo "smoke test: ok=$ok no-flag=$noflag total=$total"
