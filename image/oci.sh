#!/bin/sh
# oci.sh — Create an OCI container image from a jonerix rootfs tarball
#
# Produces a standards-compliant OCI image layout that can be loaded by
# Docker, Podman, or any OCI-compatible runtime:
#   docker load < jonerix-oci.tar
#   podman load < jonerix-oci.tar
#
# Usage: oci.sh <rootfs-tarball> [output-oci-tar] [tag]
#
# No Docker or container runtime is required to build the image.
#
# Part of jonerix — MIT License

set -eu

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

ROOTFS_TAR="${1:?Usage: oci.sh <rootfs-tarball> [output-oci-tar] [tag]}"
OUTPUT="${2:-jonerix-oci.tar}"
TAG="${3:-jonerix:latest}"

# OCI image metadata
AUTHOR="Jon-Erik G. Storm, Inc. DBA Lava Goat Software"
ARCH="${JONERIX_ARCH:-amd64}"
OS="linux"
CREATED="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || echo '2026-01-01T00:00:00Z')"
ENTRYPOINT='["/bin/sh"]'
CMD='["-l"]'

WORK_DIR=""

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

die() {
    printf "oci: error: %s\n" "$1" >&2
    exit 1
}

info() {
    printf "oci: %s\n" "$1"
}

cleanup() {
    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi
}

trap cleanup EXIT INT TERM

require_cmd() {
    for cmd in "$@"; do
        command -v "$cmd" >/dev/null 2>&1 || die "required command not found: $cmd"
    done
}

# Compute sha256 hash of a file, output just the hex digest
sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 -hex "$1" | sed 's/.*= //'
    else
        die "no sha256 tool available (need sha256sum, shasum, or openssl)"
    fi
}

# File size in bytes
file_size() {
    wc -c < "$1" | tr -d ' '
}

# ---------------------------------------------------------------------------
# Validate inputs
# ---------------------------------------------------------------------------

require_cmd tar gzip

[ -f "$ROOTFS_TAR" ] || die "rootfs tarball not found: $ROOTFS_TAR"

info "Creating OCI container image"
info "  Rootfs: $ROOTFS_TAR"
info "  Output: $OUTPUT"
info "  Tag:    $TAG"

# ---------------------------------------------------------------------------
# Setup work directory
# ---------------------------------------------------------------------------

WORK_DIR="$(mktemp -d /tmp/jonerix-oci.XXXXXX)"

OCI_DIR="$WORK_DIR/oci"
BLOBS_DIR="$OCI_DIR/blobs/sha256"

mkdir -p "$BLOBS_DIR"

# ---------------------------------------------------------------------------
# Step 1: Prepare the layer
# ---------------------------------------------------------------------------

info "Preparing filesystem layer..."

# The OCI layer must be a gzip-compressed tar archive.
# If the rootfs is already a tar, re-compress as gzip.
# If it's tar.zst, decompress and re-compress as gzip.
LAYER_FILE="$WORK_DIR/layer.tar.gz"

case "$ROOTFS_TAR" in
    *.tar.zst|*.tar.zstd)
        if command -v zstd >/dev/null 2>&1; then
            zstd -dc "$ROOTFS_TAR" | gzip -n > "$LAYER_FILE"
        elif command -v zstdcat >/dev/null 2>&1; then
            zstdcat "$ROOTFS_TAR" | gzip -n > "$LAYER_FILE"
        else
            die "rootfs is zstd-compressed but no zstd decompressor found"
        fi
        ;;
    *.tar.gz|*.tgz)
        cp "$ROOTFS_TAR" "$LAYER_FILE"
        ;;
    *.tar)
        gzip -n -c "$ROOTFS_TAR" > "$LAYER_FILE"
        ;;
    *)
        die "unsupported tarball format: $ROOTFS_TAR"
        ;;
esac

# Compute the diff_id (sha256 of the uncompressed layer)
LAYER_DIFFID_FILE="$WORK_DIR/layer_uncompressed.tar"
gzip -dc "$LAYER_FILE" > "$LAYER_DIFFID_FILE"
LAYER_DIFFID="sha256:$(sha256_file "$LAYER_DIFFID_FILE")"
rm -f "$LAYER_DIFFID_FILE"

LAYER_HASH="$(sha256_file "$LAYER_FILE")"
LAYER_SIZE="$(file_size "$LAYER_FILE")"

# Store layer blob
cp "$LAYER_FILE" "$BLOBS_DIR/$LAYER_HASH"

info "  Layer: sha256:${LAYER_HASH} (${LAYER_SIZE} bytes)"

# ---------------------------------------------------------------------------
# Step 2: Create the image config
# ---------------------------------------------------------------------------

info "Creating image configuration..."

CONFIG_JSON="$WORK_DIR/config.json"
cat > "$CONFIG_JSON" <<EOF
{
  "created": "${CREATED}",
  "author": "${AUTHOR}",
  "architecture": "${ARCH}",
  "os": "${OS}",
  "config": {
    "Entrypoint": ${ENTRYPOINT},
    "Cmd": ${CMD},
    "Env": [
      "PATH=/bin",
      "LANG=C.UTF-8"
    ],
    "WorkingDir": "/"
  },
  "rootfs": {
    "type": "layers",
    "diff_ids": [
      "${LAYER_DIFFID}"
    ]
  },
  "history": [
    {
      "created": "${CREATED}",
      "author": "${AUTHOR}",
      "created_by": "jonerix bootstrap — permissive Linux from scratch",
      "comment": "jonerix base rootfs layer"
    }
  ]
}
EOF

CONFIG_HASH="$(sha256_file "$CONFIG_JSON")"
CONFIG_SIZE="$(file_size "$CONFIG_JSON")"

cp "$CONFIG_JSON" "$BLOBS_DIR/$CONFIG_HASH"

info "  Config: sha256:${CONFIG_HASH} (${CONFIG_SIZE} bytes)"

# ---------------------------------------------------------------------------
# Step 3: Create the manifest
# ---------------------------------------------------------------------------

info "Creating image manifest..."

MANIFEST_JSON="$WORK_DIR/manifest.json"
cat > "$MANIFEST_JSON" <<EOF
{
  "schemaVersion": 2,
  "mediaType": "application/vnd.oci.image.manifest.v1+json",
  "config": {
    "mediaType": "application/vnd.oci.image.config.v1+json",
    "digest": "sha256:${CONFIG_HASH}",
    "size": ${CONFIG_SIZE}
  },
  "layers": [
    {
      "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
      "digest": "sha256:${LAYER_HASH}",
      "size": ${LAYER_SIZE}
    }
  ],
  "annotations": {
    "org.opencontainers.image.title": "jonerix",
    "org.opencontainers.image.description": "jonerix — permissive Linux distribution",
    "org.opencontainers.image.url": "https://github.com/jonerix/jonerix",
    "org.opencontainers.image.source": "https://github.com/jonerix/jonerix",
    "org.opencontainers.image.licenses": "MIT"
  }
}
EOF

MANIFEST_HASH="$(sha256_file "$MANIFEST_JSON")"
MANIFEST_SIZE="$(file_size "$MANIFEST_JSON")"

cp "$MANIFEST_JSON" "$BLOBS_DIR/$MANIFEST_HASH"

info "  Manifest: sha256:${MANIFEST_HASH} (${MANIFEST_SIZE} bytes)"

# ---------------------------------------------------------------------------
# Step 4: Create the OCI layout files
# ---------------------------------------------------------------------------

info "Creating OCI layout..."

# oci-layout
cat > "$OCI_DIR/oci-layout" <<'EOF'
{
  "imageLayoutVersion": "1.0.0"
}
EOF

# index.json
# Parse tag into name:tag
TAG_NAME="${TAG%%:*}"
TAG_VERSION="${TAG#*:}"
[ "$TAG_VERSION" = "$TAG_NAME" ] && TAG_VERSION="latest"

cat > "$OCI_DIR/index.json" <<EOF
{
  "schemaVersion": 2,
  "mediaType": "application/vnd.oci.image.index.v1+json",
  "manifests": [
    {
      "mediaType": "application/vnd.oci.image.manifest.v1+json",
      "digest": "sha256:${MANIFEST_HASH}",
      "size": ${MANIFEST_SIZE},
      "annotations": {
        "org.opencontainers.image.ref.name": "${TAG_NAME}:${TAG_VERSION}"
      },
      "platform": {
        "architecture": "${ARCH}",
        "os": "${OS}"
      }
    }
  ]
}
EOF

# ---------------------------------------------------------------------------
# Step 5: Package as tar archive
# ---------------------------------------------------------------------------

info "Packaging OCI image as tar archive..."

# Also produce a Docker-compatible manifest for 'docker load' compatibility
cat > "$OCI_DIR/manifest.json" <<EOF
[
  {
    "Config": "blobs/sha256/${CONFIG_HASH}",
    "RepoTags": ["${TAG_NAME}:${TAG_VERSION}"],
    "Layers": ["blobs/sha256/${LAYER_HASH}"]
  }
]
EOF

# Create the tar archive
(cd "$OCI_DIR" && tar -cf - .) > "$OUTPUT"

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------

OUTPUT_SIZE="$(file_size "$OUTPUT")"
OUTPUT_MB="$((OUTPUT_SIZE / 1048576))"

info "Done. OCI image created: $OUTPUT (${OUTPUT_MB} MB)"
info "Load with: docker load < $OUTPUT"
info "       or: podman load < $OUTPUT"

# Cleanup layer temp file
rm -f "$LAYER_FILE"
