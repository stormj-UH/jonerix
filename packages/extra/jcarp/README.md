# jcarp

`jcarp` is a Rust OpenBSD-CARP-compatible failover daemon for Linux. It speaks
CARP v2 against OpenBSD peers and is wire-compatible with VRRPv2 in restricted
modes. License: BSD-2-Clause.

This directory is the in-tree jonerix package — the upstream Rust project lives
under [`src/`](src/), packaged by [`recipe.toml`](recipe.toml).

## Install (any Linux)

One-liner:

```sh
curl -sSfL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/extra/jcarp/install.sh | sh
```

Or with options:

```sh
sh install.sh --version 0.1.0-r1 --prefix /usr/local --arch aarch64
```

Flags:

| Flag        | Default       | Meaning                                  |
|-------------|---------------|------------------------------------------|
| `--version` | `0.1.0-r1`    | Published `.jpkg` version to fetch       |
| `--prefix`  | `/usr/local`  | Install root                             |
| `--arch`    | `uname -m`    | `aarch64` or `x86_64`                    |
| `--help`    |               | Show usage                               |

The script downloads `jcarp-<VERSION>-<ARCH>.jpkg` from the
`stormj-UH/jonerix` `packages` release pool, verifies the JPKG header magic,
extracts the zstd-compressed tar payload, and installs:

```
$PREFIX/bin/jcarp
$PREFIX/etc/jcarp/jcarp.conf.default
$PREFIX/etc/init.d/jcarp
$PREFIX/share/licenses/jcarp/LICENSE
```

The installer never overwrites an existing `jcarp.conf` — it always lands in
`jcarp.conf.default`. Required tools: `curl` or `wget`, `zstd`, `tar`, `od`,
`dd`, `install`. Distro-specific install hints are printed on miss.

### Install (jonerix users)

You don't need this script. Use `jpkg`:

```sh
jpkg install jcarp
```

## Post-install

jcarp is a daemon — it will not run until configured.

1. Copy and edit the config:

   ```sh
   cp /usr/local/etc/jcarp/jcarp.conf.default /usr/local/etc/jcarp/jcarp.conf
   $EDITOR /usr/local/etc/jcarp/jcarp.conf
   ```

2. Privileges. CARP/VRRP needs raw sockets and link-layer manipulation:

   ```sh
   setcap 'cap_net_admin,cap_net_raw=ep' /usr/local/bin/jcarp
   ```

   Or run as root via the shipped service.

3. Service management.

   **OpenRC (Alpine / jonerix)** — works as-is:

   ```sh
   rc-update add jcarp default
   rc-service jcarp start
   ```

   **systemd** — sample one-shot unit:

   ```ini
   # /etc/systemd/system/jcarp.service
   [Unit]
   Description=jcarp CARP failover daemon
   After=network-online.target
   Wants=network-online.target

   [Service]
   Type=simple
   ExecStartPre=/usr/local/bin/jcarp --config /usr/local/etc/jcarp/jcarp.conf check
   ExecStart=/usr/local/bin/jcarp --config /usr/local/etc/jcarp/jcarp.conf run
   AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW
   CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_RAW
   NoNewPrivileges=yes
   Restart=on-failure
   RestartSec=2s

   [Install]
   WantedBy=multi-user.target
   ```

   Then:

   ```sh
   systemctl daemon-reload
   systemctl enable --now jcarp
   ```

## Where things live

- [`recipe.toml`](recipe.toml) — jonerix package recipe (custom build).
- [`src/`](src/) — the jcarp Rust crate (BSD-2-Clause).
- [`files/jcarp.conf`](files/jcarp.conf) — default config shipped as `jcarp.conf.default`.
- [`files/jcarp.initd`](files/jcarp.initd) — OpenRC service.
- [`install.sh`](install.sh) — POSIX-shell installer, served raw from GitHub.
