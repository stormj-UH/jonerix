# `config/kernel/` — kernel configuration fragments

These are **reference fragments** for end users who want to build a
Linux kernel tuned for the jonerix layout (merged-usr-flat, musl
libc, OpenRC, no GUI, headless server defaults).  They are NOT
copied into the rootfs by the image-builder — they are documentation
plus a Lego-brick set for hand-rolled kernels.

## Layout

```
config/kernel/
├── README.md           (this file)
├── base.config         mandatory floor — features every jonerix kernel needs
├── arch/
│   ├── x86_64.config           Intel / AMD desktop, server, cloud VM
│   ├── aarch64-pi5.config      Raspberry Pi 5 (BCM2712 + Broadcom drivers)
│   └── aarch64-server.config   Generic aarch64 (Ampere, AWS Graviton, KVM guest)
└── profile/
    ├── minimal.config          bare boot — serial console + virtio
    ├── builder.config          containers + KVM + perf + namespaces + cgroups v2
    └── router.config           full netfilter + tunneling + tc/qdisc + wireguard
```

## How to use

The fragments are designed to be merged with upstream Linux's
`scripts/kconfig/merge_config.sh` tool.  Typical workflow:

```sh
# 1. Get the kernel source you want to build.
cd /usr/src
git clone --depth 1 --branch v6.18 https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git
cd linux

# 2. Start from a clean defconfig for your architecture.
make defconfig          # x86_64
# or for aarch64:
# make ARCH=arm64 defconfig

# 3. Layer jonerix's fragments on top.  Last argument wins on
#    conflicts — order is base, arch, profile.
./scripts/kconfig/merge_config.sh -m -O . .config \
    /path/to/jonerix/config/kernel/base.config \
    /path/to/jonerix/config/kernel/arch/x86_64.config \
    /path/to/jonerix/config/kernel/profile/builder.config

# 4. Reconcile against the kernel's current options (fills in
#    defaults for new options introduced since these fragments
#    were last refreshed).
make olddefconfig

# 5. Build.
make -j$(nproc)
make modules_install INSTALL_MOD_PATH=/path/to/staging
cp arch/x86/boot/bzImage /path/to/staging/boot/vmlinuz-jonerix
```

For Raspberry Pi 5, you almost certainly want the official Pi
foundation kernel tree instead of mainline:

```sh
git clone --depth 1 --branch rpi-6.18.y \
    https://github.com/raspberrypi/linux.git
cd linux
make ARCH=arm64 bcm2712_defconfig
./scripts/kconfig/merge_config.sh -m -O . .config \
    /path/to/jonerix/config/kernel/base.config \
    /path/to/jonerix/config/kernel/arch/aarch64-pi5.config \
    /path/to/jonerix/config/kernel/profile/minimal.config    # or builder, router
make ARCH=arm64 olddefconfig
make ARCH=arm64 Image dtbs modules -j$(nproc)
```

## Fragment combination rules

You pick exactly one fragment from each of `arch/` and `profile/`.
`base.config` is always included.  Other recipes:

| Target system                        | base + arch                     + profile          |
|--------------------------------------|--------------------------------------------------- |
| Tormenta (Pi 5, dev server)          | base + aarch64-pi5             + builder           |
| Pi 5 home router                     | base + aarch64-pi5             + router            |
| AWS Graviton EC2 / Oracle Ampere     | base + aarch64-server          + minimal           |
| AWS Graviton CI runner               | base + aarch64-server          + builder           |
| x86_64 cloud VM (DigitalOcean, EC2)  | base + x86_64                  + minimal           |
| x86_64 build farm                    | base + x86_64                  + builder           |
| x86_64 home router (mini-ITX, fanless)| base + x86_64                 + router            |

## Conventions

- **`=y` vs `=m`** — bulky network and filesystem features are
  built as modules (`=m`) so they're loadable on demand.  Core kernel
  features that the system can't boot without (namespaces, cgroups
  v2, ext4, the BPF subsystem) are `=y`.
- **Security baseline** — `STACKPROTECTOR_STRONG`, `RANDOMIZE_BASE`,
  `RETPOLINE` (x86), `INIT_STACK_ALL_ZERO`, `HARDENED_USERCOPY` are
  in `base.config`.  You can't disable them via profile fragments
  (the wins-on-conflict order is base → arch → profile, but anything
  in base.config is `# always set, leave alone`).
- **`# CONFIG_FOO is not set`** — explicit disables are documented
  in profile fragments, NOT in base.config.  base.config only sets
  things to `=y` / `=m`.

## What's NOT in here

- A `defconfig` substitute — these are fragments, not full configs.
  You still start from the kernel's own per-architecture defconfig
  and merge our fragments on top.
- Distribution-specific quirks (e.g. specific HID driver lists for
  some random USB peripheral).  Add those as a separate fragment
  via `merge_config.sh -m`.

## Maintenance

When kernel versions shift, run:

```sh
make ARCH=$ARCH listnewconfig    # show newly-added options the merge doesn't cover
make ARCH=$ARCH menuconfig       # eyeball-check the result
diff -u .config.before .config   # see what changed
```

and update the fragments if any new option deserves to be pinned.
