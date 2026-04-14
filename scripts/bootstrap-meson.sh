#!/bin/sh
set -e

if command -v meson >/dev/null 2>&1; then
    exit 0
fi

if ! command -v python3 >/dev/null 2>&1; then
    echo "bootstrap-meson: python3 is required" >&2
    exit 1
fi

if ! command -v curl >/dev/null 2>&1; then
    echo "bootstrap-meson: curl is required" >&2
    exit 1
fi

MESON_VERSION="${MESON_VERSION:-1.7.0}"
MESON_PREFIX="${MESON_PREFIX:-/opt/meson-$MESON_VERSION}"
MESON_WRAPPER="${MESON_WRAPPER:-/bin/meson}"
MESON_URL="https://github.com/mesonbuild/meson/archive/refs/tags/${MESON_VERSION}.tar.gz"

tmpdir="$(mktemp -d 2>/dev/null || echo "/tmp/meson-bootstrap-$$")"
mkdir -p "$tmpdir"
trap 'rm -rf "$tmpdir"' EXIT INT TERM

curl -fsSL "$MESON_URL" -o "$tmpdir/meson.tar.gz"
mkdir -p "$tmpdir/src"

if command -v bsdtar >/dev/null 2>&1 && bsdtar --version >/dev/null 2>&1; then
    bsdtar -xf "$tmpdir/meson.tar.gz" -C "$tmpdir/src"
elif [ -x /bin/toybox ]; then
    /bin/toybox tar -xf "$tmpdir/meson.tar.gz" -C "$tmpdir/src"
else
    tar -xf "$tmpdir/meson.tar.gz" -C "$tmpdir/src"
fi

rm -rf "$MESON_PREFIX"
mkdir -p "$(dirname "$MESON_PREFIX")"
mv "$tmpdir/src/meson-$MESON_VERSION" "$MESON_PREFIX"

mkdir -p "$(dirname "$MESON_WRAPPER")"
cat > "$MESON_WRAPPER" <<EOF
#!/bin/sh
exec python3 "$MESON_PREFIX/meson.py" "\$@"
EOF
chmod 755 "$MESON_WRAPPER"

meson --version
