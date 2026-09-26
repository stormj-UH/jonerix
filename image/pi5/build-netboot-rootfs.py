#!/usr/bin/env python3
"""build-netboot-rootfs.py — build the live-installer rootfs tarball
that pairs with image/pi5/network-boot.py.

Output: jonerix-pi5-netboot-rootfs.tar.zst — a zstd-compressed tarball
of a complete minimal jonerix root, populated via `jpkg --root`,
pinned to the source tree's release tag, with the pi5-netboot-menu
script (image/pi5/netboot-menu.sh) wired in as the tty1 entry point.

When the bootstrap script (install/jonerix-pi5-netboot.sh) serves
this tarball over HTTP and a Pi 5 PXE-boots the matching kernel +
DTBs, the kernel's initramfs (built separately) fetches + extracts
this tarball into a tmpfs root. OpenRC starts. The menu appears on
tty1, offering the user:

  1) Run jonerix from this netboot session  (mode B — diskless)
  2) Install jonerix to a local disk         (mode A — equivalent
                                               to a build-image.py
                                               output, pinned to the
                                               same release tag)
  3) Drop to a shell

The package set is intentionally larger than the SD/USB image's
minimal-boot core: a live installer needs reforger (mkfs.ext4 +
mkfs.vfat for mode A), curl + bsdtar (firmware download in mode A),
python3 (pi5-install.sh runs python helpers), and the same shadow +
iproute-go + niceties so a user dropped into mode-B can poke around.

Standalone Python 3.9+ stdlib. Designed to run on a jonerix builder
container OR an Ubuntu CI runner that has jpkg available; the latter
is the path GitHub Actions uses.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone


# Same set as build-image.py's DEFAULT_PACKAGES + the live-installer
# extras. Keeping them in sync means a user who picks mode-A from the
# menu lands at exactly what build-image.py would have produced.
NETBOOT_ROOTFS_PACKAGES = [
    # Boot core
    "musl", "toybox", "mksh", "openrc",
    "dhcpcd", "openresolv", "ifupdown-ng", "dropbear",
    "bsdtar", "openntpd", "sudo", "python3",

    # mkfs + fsck (mode A target formatting)
    "reforger",
    "raspi-config",

    # Login chain
    "shadow",

    # Network tooling
    "jonerix-netutils",

    # Live-installer extras (curl is needed to fetch firmware tarball
    # in mode A; libressl + ca-certificates for HTTPS).
    "libressl", "ca-certificates", "curl",

    # Interactive niceties (parity with the SD/USB image)
    "zsh", "gitredoxide", "ripgrep", "pico", "fastfetch",

    # Pi-specific fixups
    "jonerix-raspi5-fixups",
]

# Installed last with --force (same as build-image.py LATE_PACKAGES): the
# package that owns a contested path (ca-certificates owns
# /etc/ssl/cert.pem) must land after everything else.
LATE_PACKAGES = ["ca-certificates"]

# Base account files seeded before any package hook runs (same as
# build-image.py): no package ships a root account, and addgroup-safe
# refuses to touch a missing /etc/group. Copied only if absent.
BASE_ACCOUNT_FILES = [("passwd", 0o644), ("group", 0o644),
                      ("shadow", 0o600), ("shells", 0o644)]

# Always installed, even with --no-default-packages (Pi 5 bring-up).
MANDATORY_PACKAGES = ["jonerix-raspi5-fixups"]

# dropbear's OpenRC service (same list as build-image.py SSH_SERVICES): the
# first one present goes into the default runlevel.
SSH_SERVICES = ("sshd", "dropbear")

RELEASE_BASE_URL = "https://github.com/stormj-UH/jonerix/releases/download"
ROLLING_TAG = "packages"

LOG = lambda m: print(f"[netboot-rootfs] {m}", flush=True)
DIE = lambda m: (print(f"error: {m}", file=sys.stderr), sys.exit(1))[1]


def _resolve_release_tag(tag: str) -> str:
    """Default to v$VERSION_ID from config/defaults/etc/os-release."""
    if tag:
        return tag
    try:
        repo_root = pathlib.Path(__file__).resolve().parents[2]
        osr = (repo_root / "config" / "defaults" / "etc" / "os-release").read_text()
        for line in osr.splitlines():
            if line.startswith("VERSION_ID="):
                return "v" + line.split("=", 1)[1].strip().strip('"')
    except Exception:
        pass
    return ROLLING_TAG


def run(cmd: list[str], **kw):
    LOG(" ".join(cmd))
    subprocess.run(cmd, check=True, **kw)


def resolve_packages(extra: str, no_defaults: bool) -> list[str]:
    """--packages is additive (same contract as build-image.py):
    NETBOOT_ROOTFS_PACKAGES (unless --no-default-packages) + the user's
    list + MANDATORY_PACKAGES, de-duplicated, order kept."""
    user = [p.strip() for p in (extra or "").replace(",", " ").split() if p.strip()]
    base = [] if no_defaults else list(NETBOOT_ROOTFS_PACKAGES)
    return list(dict.fromkeys(base + user + MANDATORY_PACKAGES))


def enable_ssh_service(root: pathlib.Path):
    """Put dropbear's service in the default runlevel when the package
    ships one, like build-image.py does for SD/USB images. root stays
    locked, so logins need a key in /root/.ssh/authorized_keys."""
    for svc in SSH_SERVICES:
        if (root / "etc" / "init.d" / svc).is_file():
            rl = root / "etc" / "runlevels" / "default"
            rl.mkdir(parents=True, exist_ok=True)
            link = rl / svc
            if link.exists() or link.is_symlink():
                link.unlink()
            link.symlink_to(f"/etc/init.d/{svc}")
            LOG(f"enabled {svc} in the default runlevel")
            return
    LOG("WARN: no sshd/dropbear OpenRC service in the rootfs; SSH stays off")


def seed_base_accounts(root: pathlib.Path):
    src_dir = pathlib.Path(__file__).resolve().parents[2] / "config" / "defaults" / "etc"
    etc = root / "etc"
    etc.mkdir(parents=True, exist_ok=True)
    for name, mode in BASE_ACCOUNT_FILES:
        dst = etc / name
        if dst.exists():
            continue
        src = src_dir / name
        if not src.is_file():
            DIE(f"missing {src}; cannot seed /etc/{name}")
        shutil.copyfile(src, dst)
        dst.chmod(mode)
        LOG(f"seeded /etc/{name}")


def jpkg_install(root: pathlib.Path, packages: list[str], release_tag: str):
    """Mirror of image/pi5/build-image.py's jpkg_install. Pin
    /etc/jpkg/repos.conf to the release tag during install, then
    rewrite to rolling so a booted Pi tracks main going forward."""
    staging_jpkg = root / "etc" / "jpkg"
    (staging_jpkg / "keys").mkdir(parents=True, exist_ok=True)

    # Pinned mirror for the install
    (staging_jpkg / "repos.conf").write_text(
        f"# Generated by build-netboot-rootfs.py — pinned to {release_tag}\n"
        f"# during install; rewritten to rolling before sealing.\n"
        f"[repo]\n"
        f'url = "{RELEASE_BASE_URL}/{release_tag}"\n'
    )

    # Trust keys
    host_jpkg = pathlib.Path("/etc/jpkg")
    if (host_jpkg / "keys").is_dir():
        for k in (host_jpkg / "keys").iterdir():
            dst = staging_jpkg / "keys" / k.name
            if not dst.exists():
                shutil.copy(k, dst)

    # merged-usr: /usr -> . symlink before any package install so
    # python3's --prefix=/usr layout resolves into the flat tree.
    usr = root / "usr"
    if not usr.is_symlink():
        if usr.is_dir():
            for child in usr.iterdir():
                tgt = root / child.name
                if tgt.exists():
                    continue
                shutil.move(str(child), str(tgt))
            usr.rmdir()
        usr.symlink_to(".")

    seed_base_accounts(root)

    run(["jpkg", "--root", str(root), "update"])
    if "toybox" in packages:
        # Mirror build-image.py: replacement hooks need toybox applets
        # available before mksh/shadow/raspi5-fixups run, and toybox must
        # not be installed later after those packages claim their links.
        run(["jpkg", "--root", str(root), "install", "toybox"])
    run(["jpkg", "--root", str(root), "install"]
        + [p for p in packages if p not in LATE_PACKAGES])
    for pkg in (p for p in packages if p in LATE_PACKAGES):
        run(["jpkg", "--root", str(root), "install", "--force", pkg])

    # Password hashes: owner-only, whatever a hook left behind.
    for name in ("shadow", "shadow-", "gshadow", "gshadow-"):
        f = root / "etc" / name
        if f.is_file() and not f.is_symlink():
            f.chmod(0o600)

    # Switch to rolling for post-boot updates
    (staging_jpkg / "repos.conf").write_text(
        f"# Default jonerix package mirror — rolling.\n"
        f"# To pin: jpkg conform <ver>\n"
        f"[repo]\n"
        f'url = "{RELEASE_BASE_URL}/{ROLLING_TAG}"\n'
    )


def write_netboot_fstab_and_state_service(root: pathlib.Path):
    """Mode B (diskless) needs writable storage so a user poking around
    isn't trapped on a read-only-ish tarball-extracted root.

    The /var/state mount is intentionally NOT in fstab — its size is
    set at boot via the `jonerix.state_size=` kernel cmdline param so
    a user can pick "small + save my RAM" or "big + happy" without
    rebuilding the rootfs. The pi5-state OpenRC service below parses
    /proc/cmdline and mounts the tmpfs accordingly.

    This file is overwritten by pi5-install.sh in mode A — the
    installed disk's fstab uses real ext4, not tmpfs.
    """
    fstab = root / "etc" / "fstab"
    fstab.parent.mkdir(parents=True, exist_ok=True)
    fstab.write_text("""# /etc/fstab — jonerix Pi 5 netboot live root (mode B)
#
# The whole rootfs is tmpfs (extracted from the netboot tarball into
# RAM at boot). These extra mounts give a netbooted Pi the same
# /run, /tmp, /dev/pts, /sys baseline that any rootfs needs.
proc       /proc        proc        defaults                          0 0
sysfs      /sys         sysfs       defaults                          0 0
devpts     /dev/pts     devpts      gid=5,mode=0620,ptmxmode=0666     0 0
tmpfs      /run         tmpfs       defaults,size=128M                0 0
tmpfs      /tmp         tmpfs       defaults,size=512M                0 0

# /var/state — scratch tmpfs for the netbooted user. Mounted by the
# pi5-state OpenRC service with size from `jonerix.state_size=` on
# the kernel cmdline (default 512M).
""")

    # OpenRC service that parses the cmdline and mounts /var/state.
    # The shebang must be exactly #!/bin/openrc-run: jonerix folds sbin
    # into /bin, so a /sbin shebang fails to exec (ENOENT) even though
    # gendepends.sh accepts it (scripts/check-init-shebangs.sh).
    svc = root / "etc" / "init.d" / "pi5-state"
    svc.write_text("""#!/bin/openrc-run
# pi5-state — mount /var/state with size driven by kernel cmdline.
#
# Parses /proc/cmdline for `jonerix.state_size=<value>` and mounts a
# tmpfs of that size at /var/state. Acceptable values are anything
# Linux's tmpfs `size=` knob takes: 256M, 1G, 2048k, 50% (percent of
# RAM), etc. Default 512M when the param is absent.
#
# Set on the install/jonerix-pi5-netboot.sh side via --state-size, or
# edit cmdline.txt yourself before booting.

name="pi5-state"
description="Per-boot writable scratch tmpfs at /var/state"

depend() {
    need localmount
    keyword -jail -prefix
}

start() {
    SIZE=$(awk -v RS=' ' -F= '$1=="jonerix.state_size"{print $2}' /proc/cmdline)
    : "${SIZE:=512M}"
    ebegin "Mounting tmpfs at /var/state (size=$SIZE)"
    mkdir -p /var/state
    mount -t tmpfs -o "size=$SIZE,mode=0755" tmpfs /var/state
    eend $?
}

stop() {
    ebegin "Unmounting /var/state"
    umount /var/state 2>/dev/null
    eend 0
}
""")
    svc.chmod(0o755)

    # Wire it into the boot runlevel so it's up before anything else
    # tries to write to /var/state.
    rl_boot = root / "etc" / "runlevels" / "boot"
    rl_boot.mkdir(parents=True, exist_ok=True)
    link = rl_boot / "pi5-state"
    if link.exists() or link.is_symlink():
        link.unlink()
    link.symlink_to("/etc/init.d/pi5-state")


def install_menu_and_init(root: pathlib.Path, release_tag: str,
                          packages: list[str]):
    """Copy the menu script + an OpenRC service that runs it on tty1."""
    repo_root = pathlib.Path(__file__).resolve().parents[2]
    menu_src = repo_root / "image" / "pi5" / "netboot-menu.sh"
    if not menu_src.exists():
        DIE(f"missing {menu_src}")

    # The menu script itself lives in /bin, not /etc/init.d: every file
    # in init.d is listed by librc as a service, and this one is a plain
    # mksh script. netboot-menu.sh runs main_menu when invoked under any
    # name matching pi5-netboot-menu*.
    body_dst = root / "bin" / "pi5-netboot-menu"
    body_dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy(menu_src, body_dst)
    body_dst.chmod(0o755)
    menu_dst = root / "etc" / "init.d" / "pi5-netboot-menu"
    menu_dst.parent.mkdir(parents=True, exist_ok=True)

    # The pi5-install.sh script the menu's mode-A path execs. /bin, not
    # /usr/local/bin: jonerix is merged-usr-flat and dropped /usr/local/bin
    # in raspi5-fixups 1.6.28.
    pi5_install_src = repo_root / "install" / "pi5-install.sh"
    pi5_install_dst = root / "bin" / "pi5-install.sh"
    pi5_install_dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy(pi5_install_src, pi5_install_dst)
    pi5_install_dst.chmod(0o755)

    # Build-info breadcrumb so the menu can show the release tag
    info_dir = root / "etc" / "jonerix-netboot"
    info_dir.mkdir(parents=True, exist_ok=True)
    (info_dir / "build-info.json").write_text(json.dumps({
        "generator": "image/pi5/build-netboot-rootfs.py",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "release_tag": release_tag,
        "packages": packages,
    }, indent=2) + "\n")

    # Wire the menu to run on tty1 BEFORE shadow-login does. We replace
    # shadow-login's runlevel symlink with our service so the menu
    # claims tty1 first; on mode-B the menu's "exit gracefully" path
    # starts shadow-login itself, on mode-A the install reboots.
    runlevels_default = root / "etc" / "runlevels" / "default"
    runlevels_default.mkdir(parents=True, exist_ok=True)
    menu_link = runlevels_default / "pi5-netboot-menu"
    if menu_link.exists() or menu_link.is_symlink():
        menu_link.unlink()
    menu_link.symlink_to("/etc/init.d/pi5-netboot-menu")

    # Drop the OpenRC service-script header so init.d/pi5-netboot-menu
    # passes `rc-service ... start` correctly. The actual menu logic
    # is the body of netboot-menu.sh; we wrap it.
    wrapper = """#!/bin/openrc-run
# pi5-netboot-menu — wraps image/pi5/netboot-menu.sh as an OpenRC
# service that owns tty1 at first netboot. supervise-daemon respawns
# it if the user picks "drop to shell" and exits.

name="pi5-netboot-menu"
description="jonerix Pi 5 netboot live menu (tty1)"

depend() {
    need localmount
    after devfs net
    before shadow-login
    keyword -jail -prefix
}

start() {
    ebegin "Launching netboot menu on tty1"
    setsid /bin/mksh /bin/pi5-netboot-menu \
        </dev/tty1 >/dev/tty1 2>&1 &
    echo $! > /run/pi5-netboot-menu.pid
    eend 0
}

stop() {
    ebegin "Stopping netboot menu"
    if [ -f /run/pi5-netboot-menu.pid ]; then
        kill -TERM "$(cat /run/pi5-netboot-menu.pid)" 2>/dev/null
        rm -f /run/pi5-netboot-menu.pid
    fi
    eend 0
}
"""
    menu_dst.write_text(wrapper)
    menu_dst.chmod(0o755)


def build(args):
    if os.geteuid() != 0:
        DIE("must run as root (jpkg --root needs to chown extracted files)")
    if shutil.which("jpkg") is None:
        DIE("jpkg not on PATH; install it from packages/jpkg/ first")

    args.release_tag = _resolve_release_tag(args.release_tag)
    LOG(f"target release: {args.release_tag}")
    packages = resolve_packages(args.packages, args.no_default_packages)
    LOG(f"packages: {' '.join(packages)}")

    out = pathlib.Path(args.output).resolve()
    out.parent.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="netboot-rootfs-") as tmp:
        root = pathlib.Path(tmp) / "rootfs"
        root.mkdir()

        jpkg_install(root, packages, args.release_tag)
        write_netboot_fstab_and_state_service(root)
        install_menu_and_init(root, args.release_tag, packages)
        enable_ssh_service(root)

        # Tar + zstd. Use `tar --xattrs` so file caps + ACLs survive.
        # zstd -19 --long for ~2-3x better compression than default.
        LOG(f"creating {out}")
        tar_cmd = [
            "tar", "--xattrs", "--numeric-owner",
            "-C", str(root), "-cf", "-", ".",
        ]
        zst_cmd = ["zstd", "-19", "--long", "-T0", "-o", str(out)]
        with subprocess.Popen(tar_cmd, stdout=subprocess.PIPE) as tar_p:
            run(zst_cmd, stdin=tar_p.stdout)
            tar_p.wait()

        sz = out.stat().st_size
        LOG(f"built {out.name}: {sz/1024/1024:.1f} MiB")


def parse_args():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--output", "-o", default="jonerix-pi5-netboot-rootfs.tar.zst",
                   help="Output tarball path")
    p.add_argument("--release-tag", default="",
                   help="Pin packages to this jonerix release tag (default: v$VERSION_ID)")
    p.add_argument("--packages", default="",
                   help="Comma-separated extra packages, added to the live-rootfs "
                        "defaults.")
    p.add_argument("--no-default-packages", action="store_true",
                   help="Install only --packages plus "
                        f"{','.join(MANDATORY_PACKAGES)}.")
    return p.parse_args()


if __name__ == "__main__":
    build(parse_args())
