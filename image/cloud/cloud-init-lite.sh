#!/bin/sh
# cloud-init-lite.sh — Lightweight instance initialization for cloud VMs
#
# A ~500-line shell script that reads metadata from the cloud provider's
# metadata service, configures the instance (hostname, SSH keys, networking,
# user-data), and logs all actions.
#
# Supported platforms:
#   - AWS EC2 (IMDSv1 and IMDSv2)
#   - GCP Compute Engine
#   - Generic (fallback — reads from /etc/cloud-init-lite/ overrides)
#
# Usage:
#   cloud-init-lite.sh              # auto-detect platform and run all stages
#   cloud-init-lite.sh --stage <n>  # run specific stage (1=network, 2=config, 3=userdata)
#   cloud-init-lite.sh --detect     # print detected platform and exit
#
# Run once at boot via OpenRC or as a oneshot service.
# Idempotent — safe to re-run.
#
# Part of jonerix — MIT License

set -eu

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

VERSION="1.0.0"
METADATA_TIMEOUT=2
METADATA_RETRIES=3
METADATA_RETRY_DELAY=2

# AWS metadata endpoints
AWS_METADATA="http://169.254.169.254"
AWS_IMDSV2_TOKEN_URL="${AWS_METADATA}/latest/api/token"
AWS_IMDSV2_TOKEN_TTL=300
AWS_METADATA_BASE="${AWS_METADATA}/latest/meta-data"
AWS_USERDATA_URL="${AWS_METADATA}/latest/user-data"

# GCP metadata endpoints
GCP_METADATA="http://metadata.google.internal"
GCP_METADATA_BASE="${GCP_METADATA}/computeMetadata/v1"
GCP_HEADER="Metadata-Flavor: Google"

# Local state
STATE_DIR="/var/lib/cloud-init-lite"
LOG_FILE="/var/log/cloud-init-lite.log"
OVERRIDE_DIR="/etc/cloud-init-lite"
LOCK_FILE="/run/cloud-init-lite.lock"

# Detected platform (aws, gcp, generic)
PLATFORM=""
# IMDSv2 token for AWS
IMDS_TOKEN=""

# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------

_log() {
    local level="$1"; shift
    local ts
    ts="$(date -u '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || echo 'unknown')"
    printf '[%s] %s: %s\n' "$ts" "$level" "$*" | tee -a "$LOG_FILE"
}

log_info()  { _log INFO "$@"; }
log_warn()  { _log WARN "$@"; }
log_error() { _log ERROR "$@"; }

die() {
    log_error "$@"
    exit 1
}

# ---------------------------------------------------------------------------
# Locking (prevent concurrent runs)
# ---------------------------------------------------------------------------

acquire_lock() {
    if [ -f "$LOCK_FILE" ]; then
        local pid
        pid="$(cat "$LOCK_FILE" 2>/dev/null || echo '')"
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            die "Another instance is running (PID $pid)"
        fi
        log_warn "Stale lock file found, removing"
        rm -f "$LOCK_FILE"
    fi
    printf '%s\n' "$$" > "$LOCK_FILE"
}

release_lock() {
    rm -f "$LOCK_FILE"
}

# ---------------------------------------------------------------------------
# HTTP helpers (use curl, fall back to wget-like behavior)
# ---------------------------------------------------------------------------

# Generic HTTP GET with timeout and optional headers
http_get() {
    local url="$1"
    shift
    local headers=""
    local attempt=0

    # Build header arguments
    while [ $# -gt 0 ]; do
        headers="$headers -H '$1'"
        shift
    done

    while [ "$attempt" -lt "$METADATA_RETRIES" ]; do
        attempt=$((attempt + 1))

        if command -v curl >/dev/null 2>&1; then
            local result
            result="$(eval curl -sf --connect-timeout "$METADATA_TIMEOUT" \
                --max-time 10 $headers "\"$url\"" 2>/dev/null)" && {
                printf '%s' "$result"
                return 0
            }
        fi

        [ "$attempt" -lt "$METADATA_RETRIES" ] && sleep "$METADATA_RETRY_DELAY"
    done

    return 1
}

# HTTP PUT (for IMDSv2 token)
http_put() {
    local url="$1"
    shift

    if command -v curl >/dev/null 2>&1; then
        curl -sf --connect-timeout "$METADATA_TIMEOUT" \
            --max-time 10 -X PUT "$@" "$url" 2>/dev/null
        return $?
    fi

    return 1
}

# ---------------------------------------------------------------------------
# Platform detection
# ---------------------------------------------------------------------------

detect_platform() {
    log_info "Detecting cloud platform..."

    # Check for override
    if [ -f "$OVERRIDE_DIR/platform" ]; then
        PLATFORM="$(cat "$OVERRIDE_DIR/platform" | tr -d '[:space:]')"
        log_info "Platform override: $PLATFORM"
        return 0
    fi

    # Try AWS IMDSv2 first (more secure)
    IMDS_TOKEN="$(http_put "$AWS_IMDSV2_TOKEN_URL" \
        -H "X-aws-ec2-metadata-token-ttl-seconds: $AWS_IMDSV2_TOKEN_TTL" 2>/dev/null || echo '')"

    if [ -n "$IMDS_TOKEN" ]; then
        local ami_id
        ami_id="$(http_get "${AWS_METADATA_BASE}/ami-id" \
            "X-aws-ec2-metadata-token: $IMDS_TOKEN" 2>/dev/null || echo '')"
        if [ -n "$ami_id" ]; then
            PLATFORM="aws"
            log_info "Detected platform: AWS EC2 (IMDSv2)"
            return 0
        fi
    fi

    # Try AWS IMDSv1 fallback
    local aws_check
    aws_check="$(http_get "${AWS_METADATA_BASE}/ami-id" 2>/dev/null || echo '')"
    if [ -n "$aws_check" ]; then
        PLATFORM="aws"
        log_info "Detected platform: AWS EC2 (IMDSv1)"
        return 0
    fi

    # Try GCP
    local gcp_check
    gcp_check="$(http_get "${GCP_METADATA_BASE}/project/project-id" \
        "$GCP_HEADER" 2>/dev/null || echo '')"
    if [ -n "$gcp_check" ]; then
        PLATFORM="gcp"
        log_info "Detected platform: GCP Compute Engine"
        return 0
    fi

    PLATFORM="generic"
    log_info "No cloud metadata detected, using generic platform"
    return 0
}

# ---------------------------------------------------------------------------
# AWS metadata helpers
# ---------------------------------------------------------------------------

aws_metadata() {
    local path="$1"
    if [ -n "$IMDS_TOKEN" ]; then
        http_get "${AWS_METADATA_BASE}/${path}" \
            "X-aws-ec2-metadata-token: $IMDS_TOKEN"
    else
        http_get "${AWS_METADATA_BASE}/${path}"
    fi
}

aws_userdata() {
    if [ -n "$IMDS_TOKEN" ]; then
        http_get "$AWS_USERDATA_URL" \
            "X-aws-ec2-metadata-token: $IMDS_TOKEN"
    else
        http_get "$AWS_USERDATA_URL"
    fi
}

# ---------------------------------------------------------------------------
# GCP metadata helpers
# ---------------------------------------------------------------------------

gcp_metadata() {
    local path="$1"
    http_get "${GCP_METADATA_BASE}/${path}" "$GCP_HEADER"
}

# ---------------------------------------------------------------------------
# Stage 1: Networking
# ---------------------------------------------------------------------------

stage_network() {
    log_info "Stage 1: Configuring networking..."

    local iface_conf="/etc/network/interfaces"

    case "$PLATFORM" in
        aws)
            # AWS assigns networking via DHCP. Ensure the primary interface
            # is configured for DHCP.
            local mac
            mac="$(aws_metadata 'network/interfaces/macs/' | head -1 | tr -d '/')"

            if [ -n "$mac" ]; then
                log_info "Primary interface MAC: $mac"
            fi

            # Write minimal DHCP config if no interfaces file exists
            if [ ! -f "$iface_conf" ] || ! grep -q "auto eth0" "$iface_conf" 2>/dev/null; then
                log_info "Writing DHCP network configuration"
                cat > "$iface_conf" <<-'EOF'
				auto lo
				iface lo inet loopback

				auto eth0
				iface eth0 inet dhcp
				EOF
            fi
            ;;

        gcp)
            # GCP also uses DHCP but provides MTU via metadata
            local mtu
            mtu="$(gcp_metadata 'instance/network-interfaces/0/mtu' 2>/dev/null || echo '1460')"

            if [ ! -f "$iface_conf" ] || ! grep -q "auto eth0" "$iface_conf" 2>/dev/null; then
                log_info "Writing DHCP network configuration (MTU: $mtu)"
                cat > "$iface_conf" <<-EOF
				auto lo
				iface lo inet loopback

				auto eth0
				iface eth0 inet dhcp
				    pre-up ip link set dev eth0 mtu $mtu
				EOF
            fi
            ;;

        generic)
            # Don't override existing config for generic platform
            if [ ! -f "$iface_conf" ]; then
                log_info "Writing default DHCP network configuration"
                cat > "$iface_conf" <<-'EOF'
				auto lo
				iface lo inet loopback

				auto eth0
				iface eth0 inet dhcp
				EOF
            fi
            ;;
    esac

    # Configure DNS
    configure_dns

    log_info "Stage 1: Networking configuration complete"
}

configure_dns() {
    # By default, jonerix uses unbound on localhost.
    # In cloud environments, we may need to add upstream forwarders.
    local resolv="/etc/resolv.conf"

    case "$PLATFORM" in
        aws)
            # AWS VPC DNS is at the base of the VPC CIDR +2
            # The default 169.254.169.253 also works as a link-local DNS
            if [ -f /etc/unbound/unbound.conf ]; then
                # Add AWS VPC DNS as forwarder to unbound
                if ! grep -q "forward-zone" /etc/unbound/unbound.conf 2>/dev/null; then
                    cat >> /etc/unbound/unbound.conf <<-'EOF'

					forward-zone:
					    name: "."
					    forward-addr: 169.254.169.253
					EOF
                    log_info "Added AWS VPC DNS forwarder to unbound"
                fi
            fi
            ;;
        gcp)
            if [ -f /etc/unbound/unbound.conf ]; then
                if ! grep -q "forward-zone" /etc/unbound/unbound.conf 2>/dev/null; then
                    cat >> /etc/unbound/unbound.conf <<-'EOF'

					forward-zone:
					    name: "."
					    forward-addr: 169.254.169.254
					EOF
                    log_info "Added GCP DNS forwarder to unbound"
                fi
            fi
            ;;
    esac
}

# ---------------------------------------------------------------------------
# Stage 2: Instance configuration
# ---------------------------------------------------------------------------

stage_config() {
    log_info "Stage 2: Configuring instance..."

    configure_hostname
    configure_ssh_keys
    configure_timezone
    configure_sysctl

    log_info "Stage 2: Instance configuration complete"
}

configure_hostname() {
    local hostname=""

    case "$PLATFORM" in
        aws)
            hostname="$(aws_metadata 'hostname' | cut -d. -f1)"
            # Also try the tag-based hostname
            if [ -z "$hostname" ] || [ "$hostname" = "ip-"* ]; then
                local tagged
                tagged="$(aws_metadata 'tags/instance/Name' 2>/dev/null || echo '')"
                [ -n "$tagged" ] && hostname="$tagged"
            fi
            ;;
        gcp)
            hostname="$(gcp_metadata 'instance/hostname' | cut -d. -f1)"
            # GCP also has a name attribute
            if [ -z "$hostname" ]; then
                hostname="$(gcp_metadata 'instance/name' 2>/dev/null || echo '')"
            fi
            ;;
        generic)
            if [ -f "$OVERRIDE_DIR/hostname" ]; then
                hostname="$(cat "$OVERRIDE_DIR/hostname" | tr -d '[:space:]')"
            fi
            ;;
    esac

    if [ -n "$hostname" ]; then
        # Sanitize hostname: lowercase, only alphanum and hyphens
        hostname="$(printf '%s' "$hostname" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9-]/-/g' | sed 's/^-//' | sed 's/-$//')"

        if [ -n "$hostname" ]; then
            log_info "Setting hostname: $hostname"
            printf '%s\n' "$hostname" > /etc/hostname
            hostname "$hostname" 2>/dev/null || true

            # Update /etc/hosts
            if ! grep -q "$hostname" /etc/hosts 2>/dev/null; then
                printf '127.0.1.1\t%s\n' "$hostname" >> /etc/hosts
            fi
        fi
    fi
}

configure_ssh_keys() {
    local ssh_keys=""

    case "$PLATFORM" in
        aws)
            # AWS provides keys via metadata
            ssh_keys="$(aws_metadata 'public-keys/0/openssh-key' 2>/dev/null || echo '')"
            ;;
        gcp)
            # GCP stores keys in project and instance metadata
            local raw_keys
            raw_keys="$(gcp_metadata 'instance/attributes/ssh-keys' 2>/dev/null || echo '')"

            if [ -z "$raw_keys" ]; then
                raw_keys="$(gcp_metadata 'project/attributes/ssh-keys' 2>/dev/null || echo '')"
            fi

            # GCP format: "user:ssh-rsa AAAA... user@host"
            if [ -n "$raw_keys" ]; then
                ssh_keys="$(printf '%s\n' "$raw_keys" | sed 's/^[^:]*://' | head -20)"
            fi
            ;;
        generic)
            if [ -f "$OVERRIDE_DIR/authorized_keys" ]; then
                ssh_keys="$(cat "$OVERRIDE_DIR/authorized_keys")"
            fi
            ;;
    esac

    if [ -n "$ssh_keys" ]; then
        log_info "Injecting SSH authorized keys"

        # Install for root (will be the initial login for most cloud images)
        local root_ssh="/root/.ssh"
        mkdir -p "$root_ssh"
        chmod 0700 "$root_ssh"

        local auth_keys="$root_ssh/authorized_keys"

        # Append keys (avoid duplicates)
        printf '%s\n' "$ssh_keys" | while IFS= read -r key; do
            [ -z "$key" ] && continue
            if [ ! -f "$auth_keys" ] || ! grep -qF "$key" "$auth_keys" 2>/dev/null; then
                printf '%s\n' "$key" >> "$auth_keys"
            fi
        done

        chmod 0600 "$auth_keys"
        log_info "SSH keys installed to $auth_keys"

        # Enable root login for initial setup (via key only, no password)
        # Note: dropbear -w disables root password login but allows key auth
        # when -g is not set.
        local dropbear_conf="/etc/conf.d/dropbear"
        if [ -f "$dropbear_conf" ]; then
            if grep -q "DROPBEAR_OPTS" "$dropbear_conf"; then
                # Ensure -w (disable root password) but not -g (disable root entirely)
                sed -i 's/-g //g' "$dropbear_conf" 2>/dev/null || true
            fi
        fi
    fi
}

configure_timezone() {
    local tz=""

    case "$PLATFORM" in
        aws)
            # AWS doesn't provide timezone in metadata; default to UTC
            tz="UTC"
            ;;
        gcp)
            # GCP provides the zone, from which we can infer a timezone
            local zone
            zone="$(gcp_metadata 'instance/zone' 2>/dev/null || echo '')"
            # zone looks like "projects/12345/zones/us-central1-a"
            # Default to UTC; timezone would need a mapping table
            tz="UTC"
            ;;
        generic)
            if [ -f "$OVERRIDE_DIR/timezone" ]; then
                tz="$(cat "$OVERRIDE_DIR/timezone" | tr -d '[:space:]')"
            fi
            ;;
    esac

    if [ -n "$tz" ] && [ -f "/usr/share/zoneinfo/$tz" ] 2>/dev/null; then
        log_info "Setting timezone: $tz"
        ln -sf "/usr/share/zoneinfo/$tz" /etc/localtime
        printf '%s\n' "$tz" > /etc/timezone
    elif [ -n "$tz" ] && [ "$tz" = "UTC" ]; then
        log_info "Setting timezone: UTC"
        printf '%s\n' "UTC" > /etc/timezone
    fi
}

configure_sysctl() {
    log_info "Applying security sysctl settings"

    local sysctl_conf="/etc/sysctl.d/99-cloud-init-lite.conf"
    mkdir -p /etc/sysctl.d

    cat > "$sysctl_conf" <<-'EOF'
	# cloud-init-lite security defaults
	kernel.kptr_restrict = 2
	kernel.dmesg_restrict = 1
	kernel.unprivileged_bpf_disabled = 1
	net.ipv4.conf.all.rp_filter = 1
	net.ipv4.conf.default.rp_filter = 1
	net.ipv4.conf.all.accept_redirects = 0
	net.ipv4.conf.default.accept_redirects = 0
	net.ipv6.conf.all.accept_redirects = 0
	net.ipv6.conf.default.accept_redirects = 0
	net.ipv4.conf.all.send_redirects = 0
	net.ipv4.conf.default.send_redirects = 0
	net.ipv4.tcp_syncookies = 1
	net.ipv4.icmp_echo_ignore_broadcasts = 1
	EOF

    # Apply sysctl settings
    if [ -x /bin/sysctl ]; then
        sysctl -p "$sysctl_conf" >/dev/null 2>&1 || log_warn "Some sysctl settings could not be applied"
    fi
}

# ---------------------------------------------------------------------------
# Stage 3: User-data execution
# ---------------------------------------------------------------------------

stage_userdata() {
    log_info "Stage 3: Processing user-data..."

    # Check if we've already run user-data for this instance
    local instance_id
    instance_id="$(get_instance_id)"
    local marker="${STATE_DIR}/userdata-${instance_id:-unknown}"

    if [ -f "$marker" ]; then
        log_info "User-data already executed for instance $instance_id, skipping"
        return 0
    fi

    local userdata=""

    case "$PLATFORM" in
        aws)
            userdata="$(aws_userdata 2>/dev/null || echo '')"
            ;;
        gcp)
            # GCP stores user-data as startup-script
            userdata="$(gcp_metadata 'instance/attributes/startup-script' 2>/dev/null || echo '')"
            ;;
        generic)
            if [ -f "$OVERRIDE_DIR/user-data" ]; then
                userdata="$(cat "$OVERRIDE_DIR/user-data")"
            fi
            ;;
    esac

    if [ -z "$userdata" ]; then
        log_info "No user-data found"
        touch "$marker"
        return 0
    fi

    # Determine user-data type and execute
    local userdata_file="${STATE_DIR}/user-data"
    printf '%s\n' "$userdata" > "$userdata_file"

    # Check for shebang
    local first_line
    first_line="$(head -1 "$userdata_file")"

    case "$first_line" in
        '#!'*)
            # Shell script or similar
            log_info "Executing user-data script..."
            chmod +x "$userdata_file"

            local userdata_log="${STATE_DIR}/user-data.log"
            if "$userdata_file" > "$userdata_log" 2>&1; then
                log_info "User-data script completed successfully"
            else
                local rc=$?
                log_error "User-data script exited with code $rc"
                log_error "Output: $(tail -5 "$userdata_log")"
            fi
            ;;
        '#cloud-config'*)
            # Simple cloud-config subset (YAML-like)
            log_info "Processing cloud-config user-data..."
            process_cloud_config "$userdata_file"
            ;;
        *)
            # Treat as shell script by default
            log_warn "User-data has no shebang, treating as shell script"
            chmod +x "$userdata_file"
            sh "$userdata_file" > "${STATE_DIR}/user-data.log" 2>&1 || \
                log_error "User-data script failed with code $?"
            ;;
    esac

    # Mark as completed
    touch "$marker"
    log_info "Stage 3: User-data processing complete"
}

get_instance_id() {
    case "$PLATFORM" in
        aws)
            aws_metadata 'instance-id' 2>/dev/null || echo 'unknown'
            ;;
        gcp)
            gcp_metadata 'instance/id' 2>/dev/null || echo 'unknown'
            ;;
        generic)
            # Use machine-id or generate one
            if [ -f /etc/machine-id ]; then
                cat /etc/machine-id
            else
                echo 'generic'
            fi
            ;;
    esac
}

# ---------------------------------------------------------------------------
# Simple cloud-config processor
# ---------------------------------------------------------------------------

process_cloud_config() {
    local config_file="$1"

    # Very basic YAML-like parser for common cloud-config directives.
    # Supports: hostname, ssh_authorized_keys, write_files, runcmd, packages
    local in_section=""
    local line_buf=""

    while IFS= read -r line || [ -n "$line" ]; do
        # Skip comments and blank lines
        case "$line" in
            '#'*|'') continue ;;
        esac

        # Detect top-level keys
        case "$line" in
            'hostname:'*)
                local val="${line#hostname:}"
                val="$(printf '%s' "$val" | tr -d '[:space:]"'"'")"
                if [ -n "$val" ]; then
                    log_info "cloud-config: setting hostname to $val"
                    printf '%s\n' "$val" > /etc/hostname
                    hostname "$val" 2>/dev/null || true
                fi
                in_section=""
                ;;
            'ssh_authorized_keys:'*)
                in_section="ssh_keys"
                ;;
            'runcmd:'*)
                in_section="runcmd"
                ;;
            'write_files:'*)
                in_section="write_files"
                ;;
            '  - '*)
                # List item
                local item="${line#  - }"
                item="$(printf '%s' "$item" | sed 's/^"//' | sed 's/"$//' | sed "s/^'//" | sed "s/'$//")"

                case "$in_section" in
                    ssh_keys)
                        if [ -n "$item" ]; then
                            local root_ssh="/root/.ssh"
                            mkdir -p "$root_ssh"
                            chmod 0700 "$root_ssh"
                            printf '%s\n' "$item" >> "$root_ssh/authorized_keys"
                            chmod 0600 "$root_ssh/authorized_keys"
                            log_info "cloud-config: added SSH key"
                        fi
                        ;;
                    runcmd)
                        if [ -n "$item" ]; then
                            log_info "cloud-config: running command: $item"
                            eval "$item" >> "${STATE_DIR}/runcmd.log" 2>&1 || \
                                log_warn "cloud-config: command failed: $item"
                        fi
                        ;;
                esac
                ;;
            *)
                # Reset section on any unrecognized top-level key
                case "$line" in
                    [a-z]*:*) in_section="" ;;
                esac
                ;;
        esac
    done < "$config_file"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    local run_stage=""
    local detect_only=0

    # Parse arguments
    while [ $# -gt 0 ]; do
        case "$1" in
            --stage)
                shift
                run_stage="${1:?--stage requires a value (1, 2, or 3)}"
                ;;
            --detect)
                detect_only=1
                ;;
            --version)
                printf 'cloud-init-lite %s\n' "$VERSION"
                exit 0
                ;;
            --help|-h)
                printf 'Usage: cloud-init-lite.sh [--stage N] [--detect] [--version]\n'
                printf '\n'
                printf 'Stages:\n'
                printf '  1  Networking (interfaces, DNS)\n'
                printf '  2  Configuration (hostname, SSH keys, sysctl)\n'
                printf '  3  User-data (scripts, cloud-config)\n'
                printf '\n'
                printf 'Without --stage, all stages run in order.\n'
                exit 0
                ;;
            *)
                die "Unknown argument: $1"
                ;;
        esac
        shift
    done

    # Initialize state directory and log
    mkdir -p "$STATE_DIR"
    mkdir -p "$(dirname "$LOG_FILE")"
    touch "$LOG_FILE"

    log_info "cloud-init-lite v${VERSION} starting"

    # Detect platform
    detect_platform

    if [ "$detect_only" -eq 1 ]; then
        printf '%s\n' "$PLATFORM"
        exit 0
    fi

    # Acquire lock
    acquire_lock
    trap 'release_lock' EXIT INT TERM

    # Run stages
    if [ -n "$run_stage" ]; then
        case "$run_stage" in
            1) stage_network ;;
            2) stage_config ;;
            3) stage_userdata ;;
            *) die "Invalid stage: $run_stage (must be 1, 2, or 3)" ;;
        esac
    else
        stage_network
        stage_config
        stage_userdata
    fi

    # Record completion
    printf '%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || echo 'done')" \
        > "${STATE_DIR}/boot-complete"

    log_info "cloud-init-lite completed successfully"
}

main "$@"
