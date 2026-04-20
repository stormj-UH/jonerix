#!/bin/sh
#
# Regression test for jpkg: upgrading a package MUST NOT reclaim
# file paths that another installed package has taken ownership of
# via `replaces`.
#
# Scenario: install packageA v1 (ships /bin/foo, /bin/bar). Install
# packageB v1 with `replaces = ["packageA"]` and files (/bin/foo,
# /bin/bar); packageA's manifest drops those entries, packageB
# becomes the owner. Now upgrade packageA to v2 — its tarball still
# contains /bin/foo and /bin/bar because the recipe hasn't changed.
#
# Before the fix, the upgrade silently re-extracted the old files
# over packageB's, and packageB's symlinks / binaries were gone.
# After the fix, jpkg sees those paths are owned by packageB, peels
# them out of the staging tree, and leaves packageB untouched.

set -eu

JPKG="${JPKG:-/bin/jpkg}"
WORK="$(mktemp -d)"
ROOTFS="$WORK/rootfs"
REPO="$WORK/repo"
mkdir -p "$ROOTFS" "$REPO"

cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

# Dummy package builder — writes a minimal jpkg tarball
# manually using the on-disk format: a concat of a TOML manifest
# header and a zstd-compressed tar of the rootfs payload.
# For this test we instead drop pre-cooked fixture jpkgs into
# a private repo dir.
#
# TODO: once jpkg grows a test-helper `jpkg pack` subcommand, use
# that. For now we skeleton the two packages via a fixture builder
# (`pack_pkg` below) that shells to `jpkg build`.
pack_pkg() {
    name=$1; version=$2; shift; shift
    recipe="$WORK/$name/recipe.toml"
    mkdir -p "$(dirname "$recipe")"
    {
        printf '[package]\nname = "%s"\nversion = "%s"\nlicense = "MIT"\ndescription = "fixture"\n' "$name" "$version"
        if [ -n "${REPLACES:-}" ]; then
            printf 'replaces = ["%s"]\n' "$REPLACES"
        fi
        printf '[source]\nurl = ""\nsha256 = "0000000000000000000000000000000000000000000000000000000000000000"\n'
        printf '[build]\nsystem = "custom"\n'
        printf 'build = "true"\n'
        # `install` heredoc populates DESTDIR with the requested files.
        printf 'install = """\n'
        for f in "$@"; do
            printf 'install -Dm755 /dev/null "$DESTDIR%s"\n' "$f"
            printf 'printf "%s-%s\\n" > "$DESTDIR%s"\n' "$name" "$version" "$f"
        done
        printf '"""\n'
        printf '[depends]\nruntime = ["musl"]\nbuild = []\n'
    } > "$recipe"
    "$JPKG" build "$recipe" --build-jpkg -o "$REPO" >/dev/null 2>&1
    unset REPLACES
}

# --- Fixture ---------------------------------------------------------------

pack_pkg packageA 1.0 /bin/foo /bin/bar
REPLACES=packageA pack_pkg packageB 1.0 /bin/foo /bin/bar
pack_pkg packageA 2.0 /bin/foo /bin/bar   # ← regression trigger

# --- Scenario --------------------------------------------------------------

"$JPKG" -r "$ROOTFS" install "$REPO/packageA-1.0-"*.jpkg >/dev/null
"$JPKG" -r "$ROOTFS" install --force "$REPO/packageB-1.0-"*.jpkg >/dev/null

# Sanity: /bin/foo should come from packageB now
pre=$(cat "$ROOTFS/bin/foo")
case "$pre" in
    packageB-1.0*) ;;
    *) echo "FAIL: expected packageB to own /bin/foo pre-upgrade, got '$pre'"; exit 1 ;;
esac

"$JPKG" -r "$ROOTFS" install --force "$REPO/packageA-2.0-"*.jpkg >/dev/null

# Regression check: /bin/foo must still belong to packageB,
# not packageA v2.
post=$(cat "$ROOTFS/bin/foo")
case "$post" in
    packageB-1.0*)
        echo "PASS: /bin/foo still owned by packageB after upgrading packageA"
        ;;
    packageA-2.0*)
        echo "FAIL: upgrade reclaimed /bin/foo — regression of"
        echo "      'replaces survives upgrade'; got '$post'"
        exit 1
        ;;
    *)
        echo "FAIL: /bin/foo has unexpected content '$post'"
        exit 1
        ;;
esac
