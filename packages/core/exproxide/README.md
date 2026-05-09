# exproxide

`exproxide` is a clean-room Rust implementation of POSIX `expr`, written for jonerix to provide a permissively-licensed `/bin/expr` without inheriting GNU coreutils' GPL surface. License: MIT.

This directory is the in-tree jonerix package — the upstream Rust project lives on a private Forgejo at `castle.great-morpho.ts.net:3000/jonerik/exproxide`. [`install.sh`](install.sh) is mirrored from there for public reach.

## Install (any Linux)

One-liner (defaults are conservative — see below):

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/core/exproxide/install.sh | sh
```

Or with options:

```sh
sh install.sh --prefix /opt/exproxide --version 0.1.1-r0 --arch x86_64 --no-prompt
```

Flags:

| Flag                       | Default       | Meaning                                                                 |
|----------------------------|---------------|-------------------------------------------------------------------------|
| `--version VER`            | `0.1.1-r0`    | Published `.jpkg` version to fetch                                       |
| `--prefix DIR`             | `/usr/local`  | Install root                                                             |
| `--arch ARCH`              | `uname -m`    | `aarch64` or `x86_64`                                                    |
| `--make-default-expr`      | off           | Symlink `$PREFIX/bin/expr → exproxide` (or rename, depending on payload) |
| `--no-make-default-expr`   | (default)     | Skip the `expr` symlink                                                  |
| `--install-etc`            | off           | Install any `etc/` payload to the live `/etc/`                           |
| `--no-install-etc`         | (default)     | Stage `etc/` payload at `$PREFIX/etc-default/` for review only           |
| `--no-prompt`, `--yes`     | off           | Non-interactive: honor flags only, no prompts                            |
| `--help`, `-h`             |               | Show usage                                                               |

Long-form `--key=value` is accepted.

**Default install (minimal)** lays down only:

```
$PREFIX/bin/exproxide
$PREFIX/share/licenses/exproxide/LICENSE
$PREFIX/share/man/...                       (if shipped)
```

The `expr` symlink and `/etc/` payload are **opt-in** — pass the flags above, or accept the interactive prompts (when stdin is a tty). The installer never replaces `/usr/bin/expr`.

### Install (jonerix users)

You don't need this script. Use `jpkg`:

```sh
jpkg install exproxide
```

## Where things live

- [`recipe.toml`](recipe.toml) — jonerix package recipe.
- [`install.sh`](install.sh) — POSIX-shell installer, served raw from GitHub, mirrored from the canonical castle.great-morpho.ts.net Forgejo.
- Source code: not in this tree — vendored tarball is fetched via the recipe's `[source].url`.
