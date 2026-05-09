# gitredoxide

`gitredoxide` is a drop-in `/bin/git` replacement — 77 subcommands (~95% of git's documented main porcelain plus ancillary). Hard-fork of [gitoxide](https://github.com/GitoxideLabs/gitoxide)'s `gix-*` crates with our own write paths upstream gitoxide didn't have: `gix-commitgraph::write` (verified single-file commit-graph writer) and `gix-protocol::fetch::oids` (explicit-OID fetch for partial-clone backfill). Helper-mode dispatch on `argv[0]` serves `/bin/git`, `/bin/git-upload-pack`, and `/bin/git-receive-pack` from the same binary. License: MIT or Apache-2.0.

This directory is the in-tree jonerix package — the upstream Rust project lives at the private GitHub mirror [`stormj-UH/gitredoxide`](https://github.com/stormj-UH/gitredoxide). [`install.sh`](install.sh) is mirrored here for public reach (the upstream is private and `raw.githubusercontent.com/stormj-UH/gitredoxide/...` returns 404 to anonymous fetches).

## Install (any Linux)

One-liner (default: primary binary only, named `git-redoxide` so it doesn't shadow an existing `/usr/bin/git`):

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/core/gitredoxide/install.sh | sh
```

Full server install (helpers + take over plain `git`):

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/core/gitredoxide/install.sh \
  | sh -s -- --with-helpers --rename-bin git --no-prompt
```

Pin a version and prefix:

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/core/gitredoxide/install.sh \
  | sh -s -- --version 1.0.7 --prefix "$HOME/.local"
```

Flags:

| Flag                   | Default         | Meaning                                                                      |
|------------------------|-----------------|------------------------------------------------------------------------------|
| `--version VER`        | `1.0.7`         | Published `.jpkg` version to fetch                                            |
| `--prefix DIR`         | `/usr/local`    | Install root                                                                  |
| `--arch ARCH`          | `uname -m`      | `aarch64` or `x86_64`                                                         |
| `--with-helpers`       | off             | Also install `git-upload-pack` + `git-receive-pack` (server-side helpers)     |
| `--no-helpers`         | (default)       | Skip the server helpers                                                       |
| `--rename-bin NAME`    | `git-redoxide`  | Install primary binary as `$PREFIX/bin/<NAME>`. Pass `git` to take over plain `git`. |
| `--no-prompt`, `--yes` | off             | Non-interactive: honor flags only, no prompts                                 |
| `--help`, `-h`         |                 | Show usage                                                                    |

Long-form `--key=value` is accepted.

**Default install (minimal)** lays down only:

```
$PREFIX/bin/git-redoxide
$PREFIX/share/licenses/gitredoxide/LICENSE
$PREFIX/share/man/...                          (if shipped)
```

`git-upload-pack`/`git-receive-pack` server helpers and the `git` rename are **opt-in** — pass `--with-helpers` and/or `--rename-bin git`, or accept the interactive prompts (when stdin is a tty). The installer never overwrites `/usr/bin/git`.

### Install (jonerix users)

You don't need this script. Use `jpkg`:

```sh
jpkg install gitredoxide
```

## Where things live

- [`recipe.toml`](recipe.toml) — jonerix package recipe (vendored Rust workspace, multi-binary).
- [`install.sh`](install.sh) — POSIX-shell installer, served raw from GitHub, mirrored from the private `stormj-UH/gitredoxide`.
- Source code: not in this tree — vendored tarball is fetched via the recipe's `[source].url` from a `source-gitredoxide-vX.Y.Z` GitHub release.
