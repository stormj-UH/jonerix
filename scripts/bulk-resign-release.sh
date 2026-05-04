#!/bin/mksh
# scripts/bulk-resign-release.sh — Phase-1 bulk-resign helper.
#
# Retroactively signs every .jpkg currently published on the `packages` release
# tag with the jonerix Ed25519 signing key, then triggers an INDEX-only refresh
# so the new sha256s land in the public INDEX.zst.
#
# Run-once after Phase 0 (jpkg 2.1.0+) ships and BEFORE Phase 2 flips
# signature_policy=Require.  Idempotent — uses `jpkg resign --keep-existing`
# so re-runs skip already-signed packages.
#
# Prerequisites on the host running this script:
#   - jpkg 2.1.0+ (provides `jpkg resign`)
#   - gh CLI authenticated against the target repo
#   - /etc/jpkg/keys/jonerix-2026.sec readable, or pass --key
#
# Usage:
#   sudo mksh scripts/bulk-resign-release.sh \
#       [--key /path/to/jonerix.sec] \
#       [--key-id jonerix-2026] \
#       [--repo stormj-UH/jonerix] \
#       [--dry-run]

set -eu

# ── arg parsing ─────────────────────────────────────────────────────────────
SIGN_KEY=/etc/jpkg/keys/jonerix-2026.sec
KEY_ID=
REPO=stormj-UH/jonerix
DRY_RUN=0

usage() {
    cat <<'EOF'
usage: bulk-resign-release.sh [--key PATH] [--key-id ID] [--repo OWNER/NAME] [--dry-run]

Retroactively sign every .jpkg on the `packages` release tag.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --key)     shift; SIGN_KEY="$1"; shift ;;
        --key-id)  shift; KEY_ID="$1"; shift ;;
        --repo)    shift; REPO="$1"; shift ;;
        --dry-run) DRY_RUN=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *)         printf 'unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
done

# Default key-id from filename: /etc/jpkg/keys/jonerix-2026.sec → jonerix-2026.
if [ -z "$KEY_ID" ]; then
    base=$(basename "$SIGN_KEY")
    KEY_ID=${base%.sec}
fi

# ── preflight ───────────────────────────────────────────────────────────────
[ -r "$SIGN_KEY" ] || { printf 'FATAL: %s not readable\n' "$SIGN_KEY" >&2; exit 1; }
command -v jpkg >/dev/null || { printf 'FATAL: jpkg not on PATH\n' >&2; exit 1; }
command -v gh   >/dev/null || { printf 'FATAL: gh CLI not on PATH\n' >&2; exit 1; }

# Verify jpkg has the resign subcommand (2.1.0+).
if ! jpkg --help 2>&1 | grep -q '^[[:space:]]*resign'; then
    printf 'FATAL: jpkg %s does not support `resign`. Need 2.1.0+.\n' \
        "$(jpkg --version 2>&1 | head -1)" >&2
    exit 1
fi

# ── work area ───────────────────────────────────────────────────────────────
WORK=$(mktemp -d /tmp/jpkg-resign.XXXXXX)
trap 'rm -rf "$WORK"' EXIT INT TERM

printf '==> downloading every .jpkg from %s packages release\n' "$REPO"
gh release download packages \
    --repo "$REPO" \
    --pattern '*.jpkg' \
    --dir "$WORK" \
    --skip-existing

count=$(find "$WORK" -maxdepth 1 -name '*.jpkg' | wc -l | tr -d ' ')
printf '==> downloaded %s .jpkg files\n' "$count"
[ "$count" -gt 0 ] || { printf 'no .jpkg assets found; nothing to do\n'; exit 0; }

# ── resign ──────────────────────────────────────────────────────────────────
resign_args="--key $SIGN_KEY --key-id $KEY_ID --keep-existing"
[ "$DRY_RUN" = "1" ] && resign_args="$resign_args --dry-run"

printf '==> running jpkg resign %s ./*.jpkg (this is idempotent)\n' "$resign_args"
# shellcheck disable=SC2086
jpkg resign $resign_args "$WORK"/*.jpkg

if [ "$DRY_RUN" = "1" ]; then
    printf '==> dry-run complete; no uploads made\n'
    exit 0
fi

# ── upload ──────────────────────────────────────────────────────────────────
printf '==> uploading (--clobber) %s files\n' "$count"
# Upload in batches of 50 to stay under any single-call asset limit.
batch=50
i=0
batch_files=
for f in "$WORK"/*.jpkg; do
    batch_files="$batch_files $f"
    i=$((i + 1))
    if [ $((i % batch)) = 0 ]; then
        # shellcheck disable=SC2086
        gh release upload packages --repo "$REPO" --clobber $batch_files
        batch_files=
    fi
done
# Flush any remaining files.
if [ -n "$batch_files" ]; then
    # shellcheck disable=SC2086
    gh release upload packages --repo "$REPO" --clobber $batch_files
fi

# ── refresh INDEX ───────────────────────────────────────────────────────────
printf '==> dispatching publish-packages with index_only=true to refresh INDEX.zst\n'
gh workflow run publish-packages.yml \
    --repo "$REPO" \
    --ref main \
    -f index_only=true

printf '==> done.\n'
printf '   Watch INDEX refresh:  gh run list --repo %s --workflow publish-packages.yml --limit 1\n' "$REPO"
printf '   Once green, sha256s of all signed jpkgs will be in INDEX.zst, and any host\n'
printf '   running `jpkg update` + `jpkg install --force <pkg>` will get the signed copy.\n'
