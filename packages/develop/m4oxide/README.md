# m4oxide

Clean-room Rust implementation of POSIX `m4`, written from public documentation and black-box behavior of compiled `m4` binaries — no GNU m4 source consulted. Zero runtime crate dependencies; the regex engine, eval parser, and macro engine are all hand-rolled against the Rust standard library. License: MIT.

This directory is the in-tree jonerix package — the upstream Rust project lives on a private Forgejo at `castle.great-morpho.ts.net:3000/jonerik/m4oxide`. [`install.sh`](install.sh) is mirrored from there for public reach.

## Install (any Linux)

One-liner:

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/develop/m4oxide/install.sh | sh
```

Or with options:

```sh
sh install.sh                               # default: $PREFIX/bin/m4oxide only
sh install.sh --prefix "$HOME/.local"
sh install.sh --make-default                # also link $PREFIX/bin/m4 → m4oxide
sh install.sh --version 0.1.2-r0 --arch x86_64
sh install.sh --help
```

Flags:

| Flag                   | Default       | Meaning                                              |
|------------------------|---------------|------------------------------------------------------|
| `--version VER`        | `0.1.2-r0`    | Published `.jpkg` version to fetch                    |
| `--prefix DIR`         | `/usr/local`  | Install root                                          |
| `--arch ARCH`          | `uname -m`    | `aarch64` or `x86_64`                                 |
| `--make-default`       | off           | Also symlink `$PREFIX/bin/m4 → m4oxide`               |
| `--no-make-default`    | (default)     | Skip the `m4` symlink                                 |
| `--no-prompt`, `--yes` | off           | Non-interactive: honor flags only, no prompts         |
| `--help`, `-h`         |               | Show usage                                            |

Long-form `--key=value` is accepted.

**Default install** lays down only:

```
$PREFIX/bin/m4oxide
$PREFIX/share/licenses/m4oxide/LICENSE       (if shipped)
$PREFIX/share/man/manN/m4oxide.N             (if shipped)
```

The `$PREFIX/bin/m4` symlink is **strictly opt-in** — either pass `--make-default`, or accept the interactive prompt (when stdin is a tty). The installer never touches `/usr/bin/m4` or any other system binary.

### Install (jonerix users)

You don't need this script. Use `jpkg`:

```sh
jpkg install m4oxide
```

## Where things live

- [`recipe.toml`](recipe.toml) — jonerix package recipe.
- [`install.sh`](install.sh) — POSIX-shell installer, served raw from GitHub, mirrored from the canonical castle.great-morpho.ts.net Forgejo.
- Source code: not in this tree — vendored tarball is fetched via the recipe's `[source].url`.
