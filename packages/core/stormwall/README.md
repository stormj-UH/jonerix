# stormwall

`stormwall` is a pure-Rust firewall front-end that speaks both Linux **nft** and OpenBSD **pfctl** dialects, plus the legacy iptables/ip6tables/iptables-save/iptables-restore CLI surface — all dispatching to the same in-kernel netfilter backend via netlink. License: MIT.

This directory is the in-tree jonerix package — the upstream Rust project lives on a private Forgejo at `castle.great-morpho.ts.net:3000/jonerik/stormwall`. [`install.sh`](install.sh) is mirrored from there for public reach (the canonical home is on a Tailscale tailnet).

## Install (any Linux)

One-liner — auto-detects arch, downloads the latest published `.jpkg`, and drops `bin/stormwall` and `bin/pfctl` (plus license and man pages, when shipped) under `/usr/local`. **No system paths touched** by default — `/usr/sbin/iptables`, `/sbin/iptables`, `/etc/`, etc. are all untouched:

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/core/stormwall/install.sh | sh
```

Or with options:

```sh
sh install.sh --version 1.1.3 --prefix /usr/local --arch aarch64
```

Flags:

| Flag                   | Default       | Meaning                                                                 |
|------------------------|---------------|-------------------------------------------------------------------------|
| `--version VER`        | `1.1.3`       | Published `.jpkg` version to fetch                                       |
| `--prefix DIR`         | `/usr/local`  | Install root                                                             |
| `--arch ARCH`          | `uname -m`    | `aarch64` or `x86_64`                                                    |
| `--with-symlinks`      | off           | Create dispatch symlinks for `nft`, `iptables`, `iptables-save`, `iptables-restore`, `ip6tables`, `ip6tables-save`, `ip6tables-restore` under `$PREFIX/bin` |
| `--no-symlinks`        | (default)     | Skip the dispatch symlinks                                               |
| `--no-prompt`, `--yes` | off           | Non-interactive: honor flags only, no prompts                            |
| `--help`, `-h`         |               | Show usage                                                               |

Long-form `--key=value` is accepted alongside `--key value`.

The script downloads `stormwall-<VERSION>-<ARCH>.jpkg` from the `stormj-UH/jonerix` `packages` release pool, verifies the JPKG header magic, and extracts the zstd-compressed tar payload.

**Default install (minimal)** lays down only:

```
$PREFIX/bin/stormwall
$PREFIX/bin/pfctl
$PREFIX/share/licenses/stormwall/LICENSE
$PREFIX/share/man/...                       (if shipped)
```

The `nft`/`iptables`/`ip6tables`/etc. dispatch symlinks are **opt-in** — pass `--with-symlinks`, or accept the interactive prompt (when stdin is a tty). Either way, only `$PREFIX/bin` is touched; the system's own `/usr/sbin/iptables` is never replaced.

### Interactive prompt

When run on a tty (no `--no-prompt` / `--yes`), the installer asks once:

```
[stormwall] Create nft / iptables / ip6tables / *-save / *-restore symlinks under $PREFIX/bin? [y/N]
```

Default is **no**. CLI flags override the prompt; pipe through `sh` (curl|sh, no tty) bypasses prompts and uses defaults.

Required tools: `curl` or `wget`, `zstd`, `tar`, `od`, `dd`, `install`. Distro-specific install hints are printed on miss.

### Install (jonerix users)

You don't need this script. Use `jpkg`:

```sh
jpkg install stormwall
```

## Where things live

- [`recipe.toml`](recipe.toml) — jonerix package recipe (custom Rust build, vendored).
- [`install.sh`](install.sh) — POSIX-shell installer, served raw from GitHub, mirrored from the canonical castle.great-morpho.ts.net Forgejo.
- Source code: not in this tree — vendored tarball is fetched via the recipe's `[source].url` from a `source-stormwall-vX.Y.Z` GitHub release.
