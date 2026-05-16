# brash

`brash` is a clean-room Rust reimplementation of GNU Bash 5.3 — full surface (`[[ ]]`, regex, indexed and associative arrays, here-docs, command/arithmetic/process substitution, traps, history, mapfile/declare/printf/test/read/compgen). Byte-equivalent to bash 5.3 across the upstream test suite plus 1100+ realworld / dash-POSIX / mksh / shellcheck / shfmt corpora. License: MIT.

This directory is the in-tree jonerix package — the upstream Rust project lives on a private Forgejo at `castle.great-morpho.ts.net:3000/jonerik/brash`. [`install.sh`](install.sh) is mirrored from there for public reach.

## Install (any Linux)

One-liner:

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/core/brash/install.sh | sh
```

Or with options:

```sh
sh install.sh --version 1.0.18 --prefix /usr/local --arch aarch64
```

Flags:

| Flag                       | Default       | Meaning                                                                 |
|----------------------------|---------------|-------------------------------------------------------------------------|
| `--version VER`            | `1.0.18`      | Published `.jpkg` version to fetch                                       |
| `--prefix DIR`             | `/usr/local`  | Install root                                                             |
| `--arch ARCH`              | `uname -m`    | `aarch64` or `x86_64`                                                    |
| `--register-shell`         | off           | Append `$PREFIX/bin/brash` to `/etc/shells`                              |
| `--no-register-shell`      | (default)     | Skip the `/etc/shells` opt-in                                            |
| `--make-default-bash`      | off           | Symlink `$PREFIX/bin/bash → brash`                                       |
| `--no-make-default-bash`   | (default)     | Skip the bash symlink                                                    |
| `--no-prompt`, `--yes`     | off           | Non-interactive: honor flags only, no prompts                            |
| `--help`, `-h`             |               | Show usage                                                               |

Long-form `--key=value` is accepted alongside `--key value`.

The script downloads `brash-<VERSION>-<ARCH>.jpkg` from the `stormj-UH/jonerix` `packages` release pool, verifies the JPKG header magic, and extracts the zstd-compressed tar payload.

**Default install (minimal)** lays down only:

```
$PREFIX/bin/brash
$PREFIX/share/licenses/brash/LICENSE
$PREFIX/share/man/...                  (if shipped)
```

`/etc/shells` registration and the `bash` symlink are **opt-in** — pass the flags above, or accept the interactive prompts (when stdin is a tty). The installer never modifies `/etc/shells` without explicit consent. Writing `/etc/shells` needs root; the script suggests `sudo` when run unprivileged.

### Interactive prompts

When run on a tty (no `--no-prompt` / `--yes`), the installer asks per opt-in:

```
[brash] Add $PREFIX/bin/brash to /etc/shells (so chsh -s works)? [y/N]
[brash] Symlink $PREFIX/bin/bash -> brash? Note: dangerous if /bin/bash is ahead on $PATH. [y/N]
```

Default for every prompt is **no**. CLI flags override the prompts; pipe through `sh` (curl|sh, no tty) bypasses prompts and uses defaults.

Required tools: `curl` or `wget`, `zstd`, `tar`, `od`, `dd`, `install`.

### Install (jonerix users)

You don't need this script. Use `jpkg`:

```sh
jpkg install brash
```

## Where things live

- [`recipe.toml`](recipe.toml) — jonerix package recipe (custom Rust build, vendored).
- [`install.sh`](install.sh) — POSIX-shell installer, served raw from GitHub, mirrored from the canonical castle.great-morpho.ts.net Forgejo.
- Source code: not in this tree — vendored tarball is fetched via the recipe's `[source].url`.
