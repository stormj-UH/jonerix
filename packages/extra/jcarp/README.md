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

| Flag                 | Default       | Meaning                                              |
|----------------------|---------------|------------------------------------------------------|
| `--version VER`      | `0.1.0-r1`    | Published `.jpkg` version to fetch                   |
| `--prefix DIR`       | `/usr/local`  | Install root                                         |
| `--arch ARCH`        | `uname -m`    | `aarch64` or `x86_64`                                |
| `--with-init-script` | off           | Install OpenRC service to `$PREFIX/etc/init.d/jcarp` |
| `--no-init-script`   | (default)     | Skip the OpenRC service                              |
| `--with-config`      | off           | Install default config to `$PREFIX/etc/jcarp/`       |
| `--no-config`        | (default)     | Skip the default config                              |
| `--setcap`           | off           | Run `setcap cap_net_admin,cap_net_raw=ep` on binary  |
| `--no-setcap`        | (default)     | Skip setcap                                          |
| `--no-prompt`, `--yes` | off         | Non-interactive: honor flags only, no prompts        |
| `--help`             |               | Show usage                                           |

Long-form `--key=value` is accepted alongside `--key value`.

The script downloads `jcarp-<VERSION>-<ARCH>.jpkg` from the
`stormj-UH/jonerix` `packages` release pool, verifies the JPKG header magic,
and extracts the zstd-compressed tar payload.

**Default install (minimal)** lays down only:

```
$PREFIX/bin/jcarp
$PREFIX/share/licenses/jcarp/LICENSE
$PREFIX/share/man/...                  (if shipped)
```

The OpenRC service, default config, and `setcap` step are **opt-in** — pass
the flags above, or accept the interactive prompts (when stdin is a tty).
The installer never overwrites an existing `jcarp.conf`; the default config,
when requested, always lands in `jcarp.conf.default`.

All etc/ paths are written under `$PREFIX/etc/...` — the script never
touches the system `/etc/`.

### Interactive prompts

When run on a tty (no `--no-prompt` / `--yes`), the installer asks per
opt-in:

```
[jcarp] Install OpenRC service file (/usr/local/etc/init.d/jcarp)? [y/N]
[jcarp] Install default config (/usr/local/etc/jcarp/jcarp.conf.default)? [y/N]
[jcarp] Set CAP_NET_ADMIN+CAP_NET_RAW on /usr/local/bin/jcarp via setcap? Requires root. [y/N]
```

Default for every prompt is **no**. CLI flags override the prompts; pipe
through `sh` (curl|sh) bypasses prompts and uses defaults.

Required tools: `curl` or `wget`, `zstd`, `tar`, `od`, `dd`, `install`.
Distro-specific install hints are printed on miss.

### Install (jonerix users)

You don't need this script. Use `jpkg`:

```sh
jpkg install jcarp
```

## Post-install

jcarp is a privileged daemon — it will fail without `CAP_NET_ADMIN` +
`CAP_NET_RAW` or running as root, and it will not run until configured.

1. Drop a default config (if you skipped it at install time):

   ```sh
   sh install.sh --prefix /usr/local --with-config --no-prompt
   ```

   Then copy and edit it:

   ```sh
   cp /usr/local/etc/jcarp/jcarp.conf.default /usr/local/etc/jcarp/jcarp.conf
   $EDITOR /usr/local/etc/jcarp/jcarp.conf
   ```

2. Privileges. CARP/VRRP needs raw sockets and link-layer manipulation. Run
   the installer with `--setcap`, or do it by hand as root:

   ```sh
   sudo setcap cap_net_admin,cap_net_raw=ep /usr/local/bin/jcarp
   ```

   Or run as root via a service.

3. Service management.

   **OpenRC (Alpine / jonerix)** — install the service file (if you skipped
   it at install time) and enable it:

   ```sh
   sh install.sh --prefix /usr/local --with-init-script --no-prompt
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
