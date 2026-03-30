#!/bin/mksh
# Install Claude Code on jonerix
# Works with mksh (no bash required)

set -e

ARCH=$(uname -m)
case $ARCH in
    aarch64) ARCH="arm64" ;;
    x86_64)  ARCH="x64" ;;
    *) echo "Unsupported arch: $ARCH" >&2; exit 1 ;;
esac

GCS="https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases"
INSTALL_DIR="${CLAUDE_HOME:-$HOME/.claude}"

echo "Detecting latest Claude Code version..."
VER=$(curl -fsSL "$GCS/latest/manifest.json" 2>/dev/null | \
    sed -n 's/.*"version":"\([^"]*\)".*/\1/p' | head -1)

if [ -z "$VER" ]; then
    echo "Failed to detect version. Trying 'stable'..."
    VER=$(curl -fsSL "$GCS/stable/manifest.json" 2>/dev/null | \
        sed -n 's/.*"version":"\([^"]*\)".*/\1/p' | head -1)
fi

if [ -z "$VER" ]; then
    echo "Error: Could not determine Claude Code version" >&2
    exit 1
fi

echo "Installing Claude Code $VER for linux-$ARCH"
URL="$GCS/$VER/claude-code-linux-${ARCH}.tar.gz"

mkdir -p "$INSTALL_DIR" /tmp/claude-install

echo "Downloading..."
curl -fsSL "$URL" -o /tmp/claude-install/claude.tar.gz

echo "Extracting to $INSTALL_DIR..."
cd "$INSTALL_DIR"
tar xf /tmp/claude-install/claude.tar.gz

# Create symlink in /bin if writable
if [ -w /bin ]; then
    ln -sf "$INSTALL_DIR/claude" /bin/claude 2>/dev/null || true
    echo "Symlinked /bin/claude"
fi

rm -rf /tmp/claude-install

echo "Claude Code $VER installed to $INSTALL_DIR/claude"
"$INSTALL_DIR/claude" --version 2>/dev/null || echo "(run: $INSTALL_DIR/claude)"
