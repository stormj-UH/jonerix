# `config/` — jonerix system configuration

This directory holds every shipped system-configuration file
*outside* what's installed by packages.  An image-builder script
overlays these files into the rootfs in a deterministic order:

    1. defaults/       — every jonerix image gets this
    2. openrc/         — init system tuning (also goes everywhere)
    3. profile/<NAME>/ — exactly ONE profile per image (minimal,
                         builder, or router) overlays last and
                         wins on conflicts
    4. kernel/         — fragments for end-users to build their
                         own kernel (NOT installed into the rootfs)

## Layout

```
config/
├── defaults/etc/                 baseline — every image gets these
│   ├── os-release, passwd, group, shadow, securetty, services
│   ├── hostname, hosts, host.conf, resolv.conf, gai.conf
│   ├── profile, zshrc, inputrc, locale.conf, timezone
│   ├── issue, issue.net, motd
│   ├── shells, nanorc, protocols
│   ├── dhcpcd.conf, conf.d/{hostname,net}
│   ├── security/limits.conf
│   ├── sysctl.d/jonerix.conf     global sysctl tuning
│   ├── local.d/README            doc for OpenRC drop-ins
│   ├── skel/                     ~/.profile, .brashrc, .brash_profile, .zshrc
│   ├── jpkg/keys/jonerix.pub     package signing key
│   ├── fastfetch/                shell-greeting config
│   └── ssl/                      placeholder for CA bundle
├── openrc/                       init system tuning
│   ├── inittab
│   ├── rc.conf
│   └── init.d/snooze
├── profile/
│   ├── minimal/etc/              cloud VM / container guest
│   ├── builder/etc/              build host / dev workstation / CI runner
│   └── router/etc/               network appliance (NAT + DHCP + DNS + Wi-Fi AP)
└── kernel/
    ├── README.md                 how to build a jonerix-flavoured kernel
    ├── base.config               mandatory floor
    ├── arch/{x86_64,aarch64-pi5,aarch64-server}.config
    └── profile/{minimal,builder,router}.config
```

## Profile semantics

Pick ONE profile per image.  Each profile is a SUPERSET of the
defaults — files in `profile/<NAME>/etc/...` overlay on top of the
files in `defaults/etc/...` with the same relative path.

| Profile  | Targets                                                           |
|----------|-------------------------------------------------------------------|
| minimal  | Cloud VMs, container guests, USB-key recovery images              |
| builder  | Build hosts, CI runners, dev workstations (containers + KVM + perf) |
| router   | Network appliances (NAT, DHCP server, DNS resolver, Wi-Fi AP)     |

Within a profile, conventions:

- `etc/sysctl.d/<profile>.conf` overrides values in
  `defaults/etc/sysctl.d/jonerix.conf` (last `sysctl -p` wins)
- `etc/security/limits.d/<profile>.conf` adds rules on top of
  `defaults/etc/security/limits.conf`
- Service config files (dnsmasq.conf, hostapd.conf, …) live in their
  natural `/etc/<service>/` location; the image-builder copies them
  verbatim

## Image builder contract

The merging is line-by-line file overlay (think `cp -a`), NOT a
semantic merge of file contents.  An image-builder script (out of
scope for this directory but standard pattern) does roughly:

    rsync -a config/defaults/  $ROOTFS/
    rsync -a config/openrc/    $ROOTFS/etc/
    rsync -a config/profile/$P/  $ROOTFS/

For drop-in directories (`sysctl.d/`, `security/limits.d/`,
`local.d/`, `cron.d/`, `init.d/`, `conf.d/`) the profile's file
**adds** to whatever the defaults already provided (multiple files
in the directory, kernel reads them all).

For monolithic files (`profile`, `inputrc`, `hosts`, …) the
profile's file would **replace** the defaults version entirely.  In
practice the profiles don't ship monolithic overrides for those —
they ship their own additive drop-in fragments instead.

## Kernel

`config/kernel/` is *reference material* — these fragments don't
install anything into the rootfs.  They're for operators who want
to build a custom Linux kernel matching the jonerix conventions.
See `config/kernel/README.md` for the build recipe.
