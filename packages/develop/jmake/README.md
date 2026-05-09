# jmake

`jmake` is a clean-room drop-in replacement for GNU Make 4.4.1, written in Rust. It implements the full GNU Make language and command surface (variables, automatic vars, pattern rules, double-colon rules, secondary expansion, `MAKEFLAGS`, `.PHONY`, `.PRECIOUS`, conditional directives, `$(eval)`, `$(call)`, parallel build with `-j`, `--debug`, etc.) without referencing the GNU make source code. License: MIT.

This directory is the in-tree jonerix package — the upstream Rust project lives at the public GitHub mirror [`stormj-UH/jmake`](https://github.com/stormj-UH/jmake) (also on the private Forgejo `castle.great-morpho.ts.net:3000/jonerik/jmake`). [`install.sh`](install.sh) is mirrored here for symmetry with the rest of the package tree; the same script also lives at the upstream's main branch and is publicly fetchable from there.

## Install (any Linux)

One-liner:

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/develop/jmake/install.sh | sh
```

(Equivalent: `https://raw.githubusercontent.com/stormj-UH/jmake/main/install.sh` — same script, identical content.)

Or with options:

```sh
sh install.sh --version 1.2.1 --prefix /usr/local --arch aarch64
sh install.sh --make-default                   # also link $PREFIX/bin/make → jmake
```

Flags:

| Flag                   | Default       | Meaning                                              |
|------------------------|---------------|------------------------------------------------------|
| `--version VER`        | `1.2.1`       | Published `.jpkg` version to fetch                    |
| `--prefix DIR`         | `/usr/local`  | Install root                                          |
| `--arch ARCH`          | `uname -m`    | `aarch64` or `x86_64`                                 |
| `--make-default`       | off           | Also symlink `$PREFIX/bin/make → jmake`               |
| `--no-make-default`    | (default)     | Skip the `make` symlink                               |
| `--no-prompt`, `--yes` | off           | Non-interactive: honor flags only, no prompts         |
| `--help`, `-h`         |               | Show usage                                            |

Long-form `--key=value` is accepted.

**Default install** lays down only:

```
$PREFIX/bin/jmake
$PREFIX/share/licenses/jmake/LICENSE
$PREFIX/share/man/...                       (if shipped)
```

The `$PREFIX/bin/make` symlink is **strictly opt-in** — either pass `--make-default`, or accept the interactive prompt (when stdin is a tty). The installer never replaces `/usr/bin/make`. Even with `--make-default`, only `$PREFIX/bin/make` is touched, and the installer warns if `/usr/bin/make` is ahead on `$PATH` (which would shadow the new symlink).

### Install (jonerix users)

You don't need this script. Use `jpkg`:

```sh
jpkg install jmake
```

## Where things live

- [`recipe.toml`](recipe.toml) — jonerix package recipe.
- [`install.sh`](install.sh) — POSIX-shell installer, served raw from GitHub, mirrored from `stormj-UH/jmake`.
- Source code: not in this tree — vendored tarball is fetched via the recipe's `[source].url` from a `source-jmake-vX.Y.Z` GitHub release.
