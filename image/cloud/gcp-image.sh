#!/bin/sh
# gcp-image.sh — Convert a jonerix disk image to a GCP Compute Engine image
#
# GCP expects a raw disk image named "disk.raw" inside a gzip-compressed
# tar archive. This script converts the output of mkimage.sh to that format
# and optionally uploads it to GCS and creates a Compute Engine image.
#
# Usage: gcp-image.sh <disk-image> [image-name] [project]
#
# Requires: tar, gzip, gcloud (optional — for upload/registration)
#
# Part of jonerix — MIT License

set -eu

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

DISK_IMAGE="${1:?Usage: gcp-image.sh <disk-image> [image-name] [project]}"
IMAGE_NAME="${2:-jonerix-$(date +%Y%m%d)}"
PROJECT="${3:-${GCP_PROJECT:-}}"

GCS_BUCKET="${JONERIX_GCS_BUCKET:-jonerix-images}"
GCS_PREFIX="${JONERIX_GCS_PREFIX:-gcp-images}"
DESCRIPTION="jonerix — permissive Linux distribution"
IMAGE_FAMILY="${JONERIX_IMAGE_FAMILY:-jonerix}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

die() {
    printf "gcp-image: error: %s\n" "$1" >&2
    exit 1
}

info() {
    printf "gcp-image: %s\n" "$1"
}

require_cmd() {
    for cmd in "$@"; do
        command -v "$cmd" >/dev/null 2>&1 || die "required command not found: $cmd"
    done
}

WORK_DIR=""

cleanup() {
    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi
}

trap cleanup EXIT INT TERM

# ---------------------------------------------------------------------------
# Validate inputs
# ---------------------------------------------------------------------------

require_cmd tar gzip

[ -f "$DISK_IMAGE" ] || die "disk image not found: $DISK_IMAGE"

# GCP image names must match: [a-z]([-a-z0-9]*[a-z0-9])?
IMAGE_NAME="$(printf '%s' "$IMAGE_NAME" | tr '[:upper:]_.' '[:lower:]--')"

info "Creating GCP image from jonerix disk image"
info "  Image:  $DISK_IMAGE"
info "  Name:   $IMAGE_NAME"

# ---------------------------------------------------------------------------
# Step 1: Prepare disk.raw
# ---------------------------------------------------------------------------

WORK_DIR="$(mktemp -d /tmp/jonerix-gcp.XXXXXX)"

info "Preparing disk.raw..."

# GCP requires the raw disk to be a multiple of 1 GB in size (rounded up).
# Determine current size and pad if needed.
CURRENT_SIZE=$(wc -c < "$DISK_IMAGE" | tr -d ' ')
ONE_GB=$((1024 * 1024 * 1024))
REMAINDER=$((CURRENT_SIZE % ONE_GB))

if [ "$REMAINDER" -ne 0 ]; then
    TARGET_SIZE=$(( ((CURRENT_SIZE / ONE_GB) + 1) * ONE_GB ))
    info "Padding image to ${TARGET_SIZE} bytes (GCP requires 1GB alignment)..."
    cp "$DISK_IMAGE" "$WORK_DIR/disk.raw"
    truncate -s "$TARGET_SIZE" "$WORK_DIR/disk.raw"
else
    cp "$DISK_IMAGE" "$WORK_DIR/disk.raw"
fi

# ---------------------------------------------------------------------------
# Step 2: Create tar.gz archive
# ---------------------------------------------------------------------------

OUTPUT_TAR="$WORK_DIR/${IMAGE_NAME}.tar.gz"

info "Creating compressed tar archive..."
(cd "$WORK_DIR" && tar -czf "$OUTPUT_TAR" disk.raw)

# Remove the raw copy to save space
rm -f "$WORK_DIR/disk.raw"

OUTPUT_SIZE=$(wc -c < "$OUTPUT_TAR" | tr -d ' ')
OUTPUT_MB=$((OUTPUT_SIZE / 1048576))
info "Archive created: ${OUTPUT_TAR} (${OUTPUT_MB} MB)"

# ---------------------------------------------------------------------------
# Step 3: Upload to GCS (if gcloud is available)
# ---------------------------------------------------------------------------

if command -v gcloud >/dev/null 2>&1; then
    if [ -z "$PROJECT" ]; then
        PROJECT="$(gcloud config get-value project 2>/dev/null || true)"
    fi

    if [ -z "$PROJECT" ]; then
        info "No GCP project specified. Skipping upload."
        info "Set GCP_PROJECT or pass as third argument."
        info "Output archive: $OUTPUT_TAR"
        # Copy to current directory
        cp "$OUTPUT_TAR" "./${IMAGE_NAME}.tar.gz"
        info "Copied to: ./${IMAGE_NAME}.tar.gz"
        exit 0
    fi

    info "Using GCP project: $PROJECT"

    # Ensure bucket exists
    if ! gcloud storage buckets describe "gs://${GCS_BUCKET}" --project="$PROJECT" >/dev/null 2>&1; then
        info "Creating GCS bucket: $GCS_BUCKET"
        gcloud storage buckets create "gs://${GCS_BUCKET}" \
            --project="$PROJECT" \
            --location=us \
            --uniform-bucket-level-access
    fi

    # Upload the archive
    GCS_PATH="gs://${GCS_BUCKET}/${GCS_PREFIX}/${IMAGE_NAME}.tar.gz"
    info "Uploading to ${GCS_PATH}..."
    gcloud storage cp "$OUTPUT_TAR" "$GCS_PATH" --project="$PROJECT"

    # ---------------------------------------------------------------------------
    # Step 4: Create Compute Engine image
    # ---------------------------------------------------------------------------

    info "Creating Compute Engine image: $IMAGE_NAME..."

    GCLOUD_ARGS="--project=$PROJECT"
    GCLOUD_ARGS="$GCLOUD_ARGS --source-uri=$GCS_PATH"
    GCLOUD_ARGS="$GCLOUD_ARGS --description=$DESCRIPTION"
    GCLOUD_ARGS="$GCLOUD_ARGS --guest-os-features=UEFI_COMPATIBLE,VIRTIO_SCSI_MULTIQUEUE,GVNIC"

    if [ -n "$IMAGE_FAMILY" ]; then
        GCLOUD_ARGS="$GCLOUD_ARGS --family=$IMAGE_FAMILY"
    fi

    gcloud compute images create "$IMAGE_NAME" \
        --project="$PROJECT" \
        --source-uri="$GCS_PATH" \
        --description="$DESCRIPTION" \
        --guest-os-features=UEFI_COMPATIBLE,VIRTIO_SCSI_MULTIQUEUE,GVNIC \
        --family="$IMAGE_FAMILY" \
        --labels=project=jonerix

    # ---------------------------------------------------------------------------
    # Step 5: Clean up GCS object
    # ---------------------------------------------------------------------------

    info "Cleaning up GCS upload..."
    gcloud storage rm "$GCS_PATH" --project="$PROJECT" 2>/dev/null || true

    # ---------------------------------------------------------------------------
    # Done
    # ---------------------------------------------------------------------------

    info "Done. GCP image created successfully."
    info "  Image:   $IMAGE_NAME"
    info "  Project: $PROJECT"
    info "  Family:  $IMAGE_FAMILY"
    info ""
    info "Launch with:"
    info "  gcloud compute instances create jonerix-vm \\"
    info "    --image=$IMAGE_NAME --image-project=$PROJECT \\"
    info "    --machine-type=e2-micro --project=$PROJECT"
else
    info "gcloud CLI not found. Skipping upload and image registration."
    cp "$OUTPUT_TAR" "./${IMAGE_NAME}.tar.gz"
    info "Output archive: ./${IMAGE_NAME}.tar.gz"
    info ""
    info "To manually create the image:"
    info "  1. gsutil cp ${IMAGE_NAME}.tar.gz gs://<bucket>/images/"
    info "  2. gcloud compute images create $IMAGE_NAME \\"
    info "       --source-uri=gs://<bucket>/images/${IMAGE_NAME}.tar.gz \\"
    info "       --guest-os-features=UEFI_COMPATIBLE"
fi
