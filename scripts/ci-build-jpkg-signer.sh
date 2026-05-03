#!/bin/sh
# Build the current Rust jpkg in a jonerix builder container and install it
# on the CI host for release INDEX signing.

set -eu

workspace=${GITHUB_WORKSPACE:-$(pwd)}
signer_image=${SIGNER_IMAGE:-ghcr.io/stormj-uh/jonerix:builder-amd64}
out=${JPKG_SIGNER_OUT:-/tmp/jpkg-signer}

mkdir -p "$out"

docker run --rm --privileged --entrypoint /bin/sh \
    -v "$workspace:/workspace" \
    -v "$out:/out" \
    -w /workspace/packages/core/jpkg \
    "$signer_image" \
    -c '
        set -eu
        triple=$(rustc -vV | sed -n "s/^host: //p")
        RUSTFLAGS="-C strip=symbols -C target-feature=+crt-static" \
            cargo build --release --frozen --target "$triple" --bin jpkg
        install -m 755 "target/$triple/release/jpkg" /out/jpkg
    '

if command -v sudo >/dev/null 2>&1; then
    sudo install -m 755 "$out/jpkg" /usr/local/bin/jpkg
else
    install -m 755 "$out/jpkg" /usr/local/bin/jpkg
fi

jpkg --version || true
