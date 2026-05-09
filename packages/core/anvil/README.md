# anvil

`anvil` is a clean-room MIT Rust ext2/3/4 userland — `mkfs.ext4`, `e2fsck`, `tune2fs`, `debugfs`, and friends — written from public ext2/3/4 specifications and black-box behavior of the GNU e2fsprogs binaries (no GNU source consulted). License: MIT.

This directory is the in-tree jonerix package — the upstream Rust project lives on a private Forgejo at `castle.great-morpho.ts.net:3000/jonerik/anvil`. [`install.sh`](install.sh) is mirrored from there for public reach.

## Install (any Linux)

One-liner:

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/core/anvil/install.sh | sh
```

Or with options:

```sh
sh install.sh --version 0.2.1-r1 --prefix /usr/local --arch aarch64
```

Flags:

| Flag                     | Default       | Meaning                                                       |
|--------------------------|---------------|---------------------------------------------------------------|
| `--version VER`          | `0.2.1-r1`    | Published `.jpkg` version to fetch                             |
| `--prefix DIR`           | `/usr/local`  | Install root                                                   |
| `--arch ARCH`            | `uname -m`    | `aarch64` or `x86_64`                                          |
| `--install-etc`          | off           | Install any `etc/` payload to the live `/etc/` instead of `$PREFIX/etc-default/` (review staged config first) |
| `--no-install-etc`       | (default)     | Stage `etc/` payload at `$PREFIX/etc-default/` for review only |
| `--no-prompt`, `--yes`   | off           | Non-interactive: honor flags only, no prompts                  |
| `--help`, `-h`           |               | Show usage                                                     |

Long-form `--key=value` is accepted.

The script downloads `anvil-<VERSION>-<ARCH>.jpkg` from the `stormj-UH/jonerix` `packages` release pool, verifies the JPKG header magic, and extracts the zstd-compressed tar payload.

**Default install** lays down all binaries (`mkfs.ext{2,3,4}`, `e2fsck`, `tune2fs`, `debugfs`, `dumpe2fs`, `resize2fs`, etc.) plus internal mkfs/fsck dispatch symlinks, license, and man pages — all under `$PREFIX/`. **No system `/etc/`** is written; any `etc/` payload is staged at `$PREFIX/etc-default/` for the user to review and copy.

### Install (jonerix users)

You don't need this script. Use `jpkg`:

```sh
jpkg install anvil
```

## Where things live

- [`recipe.toml`](recipe.toml) — jonerix package recipe.
- [`install.sh`](install.sh) — POSIX-shell installer, served raw from GitHub, mirrored from the canonical castle.great-morpho.ts.net Forgejo.
- Source code: not in this tree — vendored tarball is fetched via the recipe's `[source].url`.
