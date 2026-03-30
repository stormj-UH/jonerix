#!/bin/mksh
# Install Claude Code on jonerix
# Works with mksh (no bash required)

set -e

ARCH=$(uname -m)
case $ARCH in
    aarch64) PLATFORM="linux-arm64" ;;
    x86_64)  PLATFORM="linux-x64" ;;
    *) echo "Unsupported arch: $ARCH" >&2; exit 1 ;;
esac

GCS="https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases"
INSTALL_DIR="${CLAUDE_HOME:-$HOME/.claude}"

echo "Detecting latest Claude Code version..."
VER=$(curl -fsSL "$GCS/latest" 2>/dev/null)

if [ -z "$VER" ]; then
    echo "Error: Could not determine Claude Code version" >&2
    exit 1
fi

echo "Installing Claude Code $VER for $PLATFORM"
URL="$GCS/$VER/$PLATFORM/claude"

mkdir -p "$INSTALL_DIR"

echo "Downloading..."
curl -fsSL "$URL" -o "$INSTALL_DIR/claude"
chmod +x "$INSTALL_DIR/claude"

# Create symlink in /bin if writable
if [ -w /bin ]; then
    ln -sf "$INSTALL_DIR/claude" /bin/claude 2>/dev/null || true
    echo "Symlinked /bin/claude"
fi

echo "Claude Code $VER installed"
"$INSTALL_DIR/claude" --version 2>/dev/null || echo "Run: $INSTALL_DIR/claude"
