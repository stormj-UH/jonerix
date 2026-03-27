#!/bin/sh
# aws-ami.sh — Convert a jonerix disk image to an AWS AMI
#
# This script takes a raw disk image (from mkimage.sh) and registers it
# as an Amazon Machine Image (AMI) via the AWS CLI.
#
# Process:
#   1. Upload raw image to S3
#   2. Import as EBS snapshot via ec2 import-snapshot
#   3. Register AMI from the snapshot
#
# Usage: aws-ami.sh <disk-image> [ami-name] [region]
#
# Requires: aws CLI (configured with credentials), jq
#
# Part of jonerix — MIT License

set -eu

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

DISK_IMAGE="${1:?Usage: aws-ami.sh <disk-image> [ami-name] [region]}"
AMI_NAME="${2:-jonerix-$(date +%Y%m%d)}"
REGION="${3:-${AWS_DEFAULT_REGION:-us-east-1}}"

S3_BUCKET="${JONERIX_S3_BUCKET:-jonerix-images}"
S3_PREFIX="${JONERIX_S3_PREFIX:-ami-import}"
DESCRIPTION="jonerix — permissive Linux distribution"
ARCHITECTURE="${JONERIX_ARCH:-x86_64}"
ROOT_DEVICE="/dev/xvda"
VOLUME_SIZE="${JONERIX_VOLUME_SIZE:-2}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

die() {
    printf "aws-ami: error: %s\n" "$1" >&2
    exit 1
}

info() {
    printf "aws-ami: %s\n" "$1"
}

require_cmd() {
    for cmd in "$@"; do
        command -v "$cmd" >/dev/null 2>&1 || die "required command not found: $cmd"
    done
}

cleanup() {
    # Clean up S3 upload on failure (optional — user may want to keep it)
    if [ "${CLEANUP_S3:-0}" = "1" ] && [ -n "${S3_KEY:-}" ]; then
        info "Cleaning up S3 object: s3://${S3_BUCKET}/${S3_KEY}"
        aws s3 rm "s3://${S3_BUCKET}/${S3_KEY}" --region "$REGION" 2>/dev/null || true
    fi
}

trap cleanup INT TERM

# ---------------------------------------------------------------------------
# Validate inputs
# ---------------------------------------------------------------------------

require_cmd aws jq

[ -f "$DISK_IMAGE" ] || die "disk image not found: $DISK_IMAGE"

# Verify AWS credentials
aws sts get-caller-identity --region "$REGION" >/dev/null 2>&1 || \
    die "AWS credentials not configured or invalid"

info "Creating AMI from jonerix disk image"
info "  Image:  $DISK_IMAGE"
info "  Name:   $AMI_NAME"
info "  Region: $REGION"
info "  Arch:   $ARCHITECTURE"

# ---------------------------------------------------------------------------
# Step 1: Upload raw image to S3
# ---------------------------------------------------------------------------

S3_KEY="${S3_PREFIX}/${AMI_NAME}.raw"

info "Uploading disk image to s3://${S3_BUCKET}/${S3_KEY}..."

# Ensure the bucket exists
if ! aws s3api head-bucket --bucket "$S3_BUCKET" --region "$REGION" 2>/dev/null; then
    info "Creating S3 bucket: $S3_BUCKET"
    if [ "$REGION" = "us-east-1" ]; then
        aws s3api create-bucket --bucket "$S3_BUCKET" --region "$REGION"
    else
        aws s3api create-bucket --bucket "$S3_BUCKET" --region "$REGION" \
            --create-bucket-configuration LocationConstraint="$REGION"
    fi
fi

aws s3 cp "$DISK_IMAGE" "s3://${S3_BUCKET}/${S3_KEY}" --region "$REGION"

CLEANUP_S3=1

# ---------------------------------------------------------------------------
# Step 2: Create vmimport service role if it doesn't exist
# ---------------------------------------------------------------------------

if ! aws iam get-role --role-name vmimport >/dev/null 2>&1; then
    info "Creating vmimport IAM role..."

    TRUST_POLICY='{
        "Version": "2012-10-17",
        "Statement": [{
            "Effect": "Allow",
            "Principal": {"Service": "vmie.amazonaws.com"},
            "Action": "sts:AssumeRole",
            "Condition": {
                "StringEquals": {"sts:Externalid": "vmimport"}
            }
        }]
    }'

    aws iam create-role --role-name vmimport \
        --assume-role-policy-document "$TRUST_POLICY" >/dev/null

    ROLE_POLICY="{
        \"Version\": \"2012-10-17\",
        \"Statement\": [{
            \"Effect\": \"Allow\",
            \"Action\": [
                \"s3:GetBucketLocation\",
                \"s3:GetObject\",
                \"s3:ListBucket\"
            ],
            \"Resource\": [
                \"arn:aws:s3:::${S3_BUCKET}\",
                \"arn:aws:s3:::${S3_BUCKET}/*\"
            ]
        }, {
            \"Effect\": \"Allow\",
            \"Action\": [
                \"ec2:ModifySnapshotAttribute\",
                \"ec2:CopySnapshot\",
                \"ec2:RegisterImage\",
                \"ec2:Describe*\"
            ],
            \"Resource\": \"*\"
        }]
    }"

    aws iam put-role-policy --role-name vmimport \
        --policy-name vmimport \
        --policy-document "$ROLE_POLICY" >/dev/null

    info "Waiting for IAM role propagation..."
    sleep 10
fi

# ---------------------------------------------------------------------------
# Step 3: Import disk image as EBS snapshot
# ---------------------------------------------------------------------------

info "Importing disk image as EBS snapshot..."

IMPORT_TASK=$(aws ec2 import-snapshot \
    --region "$REGION" \
    --description "$DESCRIPTION" \
    --disk-container "{
        \"Description\": \"jonerix root disk\",
        \"Format\": \"raw\",
        \"UserBucket\": {
            \"S3Bucket\": \"${S3_BUCKET}\",
            \"S3Key\": \"${S3_KEY}\"
        }
    }" | jq -r '.ImportTaskId')

[ -n "$IMPORT_TASK" ] || die "failed to start import task"

info "Import task: $IMPORT_TASK"
info "Waiting for snapshot import to complete (this may take several minutes)..."

# Poll until complete
while true; do
    STATUS_JSON=$(aws ec2 describe-import-snapshot-tasks \
        --region "$REGION" \
        --import-task-ids "$IMPORT_TASK")

    STATUS=$(printf '%s' "$STATUS_JSON" | jq -r '.ImportSnapshotTasks[0].SnapshotTaskDetail.Status')
    PROGRESS=$(printf '%s' "$STATUS_JSON" | jq -r '.ImportSnapshotTasks[0].SnapshotTaskDetail.Progress // "0"')

    case "$STATUS" in
        completed)
            SNAPSHOT_ID=$(printf '%s' "$STATUS_JSON" | jq -r '.ImportSnapshotTasks[0].SnapshotTaskDetail.SnapshotId')
            break
            ;;
        active)
            printf "\r  Progress: %s%%  " "$PROGRESS"
            sleep 15
            ;;
        deleting|deleted|error)
            MSG=$(printf '%s' "$STATUS_JSON" | jq -r '.ImportSnapshotTasks[0].SnapshotTaskDetail.StatusMessage // "unknown error"')
            die "import failed: $MSG"
            ;;
        *)
            sleep 10
            ;;
    esac
done
printf "\n"

info "Snapshot created: $SNAPSHOT_ID"

# Tag the snapshot
aws ec2 create-tags --region "$REGION" \
    --resources "$SNAPSHOT_ID" \
    --tags "Key=Name,Value=${AMI_NAME}" "Key=Project,Value=jonerix"

# ---------------------------------------------------------------------------
# Step 4: Register AMI from snapshot
# ---------------------------------------------------------------------------

info "Registering AMI..."

# Determine boot mode based on architecture
BOOT_MODE="uefi"

AMI_ID=$(aws ec2 register-image \
    --region "$REGION" \
    --name "$AMI_NAME" \
    --description "$DESCRIPTION" \
    --architecture "$ARCHITECTURE" \
    --root-device-name "$ROOT_DEVICE" \
    --boot-mode "$BOOT_MODE" \
    --virtualization-type hvm \
    --ena-support \
    --block-device-mappings "[{
        \"DeviceName\": \"${ROOT_DEVICE}\",
        \"Ebs\": {
            \"SnapshotId\": \"${SNAPSHOT_ID}\",
            \"VolumeSize\": ${VOLUME_SIZE},
            \"VolumeType\": \"gp3\",
            \"DeleteOnTermination\": true
        }
    }]" | jq -r '.ImageId')

[ -n "$AMI_ID" ] || die "failed to register AMI"

# Tag the AMI
aws ec2 create-tags --region "$REGION" \
    --resources "$AMI_ID" \
    --tags "Key=Name,Value=${AMI_NAME}" "Key=Project,Value=jonerix"

# ---------------------------------------------------------------------------
# Step 5: Clean up S3 object
# ---------------------------------------------------------------------------

info "Cleaning up S3 upload..."
aws s3 rm "s3://${S3_BUCKET}/${S3_KEY}" --region "$REGION" 2>/dev/null || true
CLEANUP_S3=0

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------

info "Done. AMI registered successfully."
info "  AMI ID:      $AMI_ID"
info "  Snapshot ID: $SNAPSHOT_ID"
info "  Region:      $REGION"
info "  Name:        $AMI_NAME"
info ""
info "Launch with:"
info "  aws ec2 run-instances --image-id $AMI_ID --instance-type t3.micro --key-name <your-key>"
