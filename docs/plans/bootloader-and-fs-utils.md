# Plan: Bootloader and Filesystem Utilities for Bare Metal Installs

**TODO item #14**: Low-level filesystem utilities and bootloader for raw metal installs.

---

## Executive Summary

jonerix can support bare metal installation with fully permissive-licensed tooling. The recommended
approach is:

- **Bootloader**: Limine (BSD-2-Clause) for both UEFI and legacy BIOS
- **Disk partitioning**: Write a minimal `jpart` tool in C (~600 lines) using raw ioctl/sysfs —
  all existing partitioning tools are GPL
- **EFI FAT32 creation**: Port `newfs_msdos` from FreeBSD (BSD-2-Clause)
- **Root filesystem**: ext4 via `mke2fs` from e2fsprogs — however, e2fsprogs uses a mixed license
  (MIT for libs, GPL for binaries); see analysis below for the best path
- **Mount/swap**: Already covered by toybox (BLKID, LOSETUP, MKSWAP, SWAPON, SWAPOFF, MOUNT,
  UMOUNT all enabled in current toybox.config)

---

## 1. Bootloader Options

### 1.1 Options Ruled Out (GPL)

| Bootloader | License | Status |
|------------|---------|--------|
| GRUB 2 | GPL-3.0 | REJECTED — GPL runtime |
| SYSLINUX / ISOLINUX | GPL-2.0 | REJECTED — GPL runtime |
| U-Boot | GPL-2.0 | REJECTED — GPL (also primarily embedded/ARM firmware) |
| rEFInd | GPL-3.0 (main code) | REJECTED — GPL runtime |

Note: DESIGN.md currently documents syslinux as acceptable for "boot media only" citing an analogy
to UEFI firmware. This reasoning is debatable — if jonerix ships a bare metal installer, syslinux
would be distributed as part of that installer. The cleaner path is to avoid it entirely and use
Limine for everything.

### 1.2 systemd-boot (LGPL-2.1)

systemd-boot (formerly gummiboot) is the EFI boot manager component of systemd. It is licensed
under LGPL-2.1, not GPL.

**License analysis**: LGPL-2.1 is a copyleft license. The LGPL allows linking with non-GPL code, but
the bootloader itself — as a shipped binary on the ESP — must remain under LGPL. Because jonerix's
policy is "every binary on the running system must be permissive," LGPL-2.1 is a borderline case.
LGPL is weaker than GPL but is still copyleft. The jonerix philosophy as stated in DESIGN.md lists
"MIT, BSD, ISC, Apache-2.0, 0BSD, CC0, public domain" as acceptable — LGPL is not on that list.

**Recommendation**: Reject systemd-boot for runtime inclusion. It also carries the full systemd
build dependency weight.

### 1.3 EFISTUB (Linux kernel built-in)

The Linux kernel (since 3.3) includes an EFI stub that allows the kernel image to be launched
directly by UEFI firmware as an EFI executable, with no separate bootloader installed.

**How it works**:
1. Kernel is compiled with `CONFIG_EFI_STUB=y` (standard in most distro configs)
2. The kernel image (`vmlinuz`) is placed on the EFI System Partition as
   `/EFI/jonerix/vmlinuz.efi`
3. UEFI firmware launches it directly
4. Kernel parameters are passed via UEFI boot variables (`efibootmgr`) or embedded in the
   initrd as a `.cmdline` section (using `objcopy` to append them)

**License**: This is part of the Linux kernel (GPLv2), which is already jonerix's sole accepted GPL
exception. No additional binaries are required on the filesystem. The `efibootmgr` tool (GPL) would
be needed to write UEFI boot entries, but it only runs at install time, not at runtime.

**Limitations**:
- Requires UEFI firmware (no legacy BIOS support)
- Multi-boot configurations require UEFI boot variables per OS
- No boot menu without a boot manager
- Secure Boot requires signing the kernel EFI stub

**Verdict**: EFISTUB is the cleanest UEFI path with zero additional runtime binaries. For a
single-OS install on UEFI hardware it is ideal. For multi-boot or legacy BIOS, a boot manager
is needed.

### 1.4 Limine (BSD-2-Clause) — Primary Recommendation

Limine is a "modern, advanced, portable, multiprotocol bootloader and boot manager" released under
BSD-2-Clause. It is the reference implementation of the Limine boot protocol.

**Repository**: https://github.com/limine-bootloader/limine
**License**: BSD-2-Clause (confirmed in COPYING file)
**Version at time of writing**: v11.x series

#### Architecture support

| Architecture | Status |
|-------------|--------|
| x86-64 (UEFI + BIOS) | Full support |
| IA-32 (BIOS) | Supported |
| aarch64 (UEFI) | Supported |
| riscv64 (UEFI) | Supported |
| loongarch64 (UEFI) | Supported |

This covers jonerix's current target architectures (x86_64) and future targets (aarch64 for
Raspberry Pi per TODO item #18).

#### Boot protocols

Limine can boot kernels via:
- **Linux protocol** — standard Linux boot for compressed/EFI kernel images
- **Limine protocol** — native high-feature protocol
- **Multiboot 1 / Multiboot 2** — for legacy bootable kernels
- **Chainloading** — hand off to another bootloader

For jonerix, the Linux protocol is the correct choice.

#### Partition table support

- GPT (recommended)
- MBR
- Unpartitioned media (e.g., USB sticks)

#### Filesystem support on boot partition

Limine reads its config and kernel from FAT12/16/32 or ISO9660. For the EFI System Partition
(which is always FAT32 by spec), this is natural.

#### Installation workflow

**UEFI (GPT):**
1. Create GPT partition table
2. Create EFI System Partition (FAT32, type `ef00`)
3. Create root partition
4. Copy `BOOTX64.EFI` (or arch-appropriate EFI) to `/EFI/BOOT/` on the ESP
5. Create `limine.conf` on the ESP
6. No MBR installation needed

**BIOS (GPT):**
1. Create GPT partition table with a BIOS Boot Partition (type `ef02`, 1 MB, no filesystem)
2. Create root partition
3. Run: `limine bios-install <device> <bios-boot-partition-number>`
4. Copy `limine-bios.sys` and `limine.conf` to the boot partition

**BIOS (MBR):**
1. Create MBR partition table
2. Run: `limine bios-install <device>`
3. Copy `limine-bios.sys` and `limine.conf` to the root or a separate `/boot` partition

#### limine.conf example for jonerix

```toml
timeout: 5
default_entry: 1

/ jonerix
    protocol: linux
    path: boot():/vmlinuz
    cmdline: root=/dev/sda2 rootfstype=ext4 init=/bin/openrc-init quiet
```

For systems without initramfs (i.e., the root device is known at compile time or passed via
cmdline), the `module_path` (initrd) line can be omitted entirely, which is consistent with
jonerix's current DESIGN.md boot sequence.

#### Build dependencies for Limine

Limine requires at build time:
- `nasm` (assembler — MIT-like license)
- `clang` or `gcc` (clang is already in jonerix)
- `make` (GNU make — only at build time, acceptable per jonerix policy)
- `mtools` (for `limine-uefi-cd.bin` only — GPL, but only needed for ISO creation, not for
  disk install)

The `limine` host utility itself (for `bios-install`) is a small C program. It can be
cross-compiled for the jonerix installer environment.

**Verdict**: Limine is the correct choice. BSD-2-Clause, supports both UEFI and legacy BIOS,
supports x86_64 and aarch64, requires no GPL runtime components, and has a clean simple config
format. It is actively maintained with a regular release cadence.

#### Dual-mode recommendation

- **UEFI systems**: Use Limine EFI + GPT (Limine EFI binary on ESP)
- **Legacy BIOS systems**: Use Limine BIOS + GPT (BIOS Boot Partition) or MBR
- **Single-boot UEFI (advanced)**: EFISTUB direct boot is an option to avoid even the Limine
  EFI binary; use `efibootmgr` (GPL, install-time only) to register the entry

---

## 2. Filesystem Utilities

### 2.1 What Toybox Already Provides

The current `toybox.config` enables the following disk-related utilities (0BSD license):

| Command | Purpose |
|---------|---------|
| `blkid` | Identify block device filesystems and UUIDs |
| `blockdev` | Call block device ioctls (get/set size, read-only, etc.) |
| `losetup` | Set up and control loop devices |
| `mount` | Mount filesystems |
| `umount` | Unmount filesystems |
| `mountpoint` | Check if a path is a mountpoint |
| `mkswap` | Create a Linux swap area |
| `swapon` | Enable a swap device |
| `swapoff` | Disable a swap device |
| `fsck` | Basic filesystem consistency check wrapper |
| `pivot_root` | Change the root filesystem |
| `sync` | Flush filesystem buffers |
| `df` | Report filesystem disk space usage |
| `du` | Estimate file space usage |

**What toybox does NOT provide**: `fdisk`, `gdisk`, `sfdisk`, `parted`, `mkfs.ext4`, `mkfs.fat`,
`mke2fs`, `tune2fs`, `resize2fs`, `e2fsck`. These are all missing and must be sourced elsewhere.

### 2.2 Disk Partitioning

#### GPL tools (cannot use)

| Tool | License | Source |
|------|---------|--------|
| `fdisk` | GPL-1.0-or-later | util-linux |
| `sfdisk` | GPL-2.0-or-later | util-linux |
| `cfdisk` | GPL-2.0-or-later | util-linux |
| `gdisk` / `sgdisk` | GPL-2.0 | gptfdisk |
| `parted` | GPL-3.0 | GNU parted |

All mainstream disk partitioning tools for Linux are GPL. This is a gap that requires a custom
solution.

#### Option A: Write `jpart` (recommended)

A minimal GPT/MBR partitioning tool in C using Linux kernel ioctls directly. The kernel's
partition management interface via `/dev/sdX` ioctls does not require any GPL library. The
necessary kernel interfaces are:

- `BLKGETSIZE64` ioctl — get disk size
- `BLKDISCARD` ioctl — discard/zero sectors
- Writing raw partition table structures to the disk via `write()`
- GPT data structures are documented in the UEFI spec (permissively licensed spec)

Estimated scope: ~600-800 lines of C. Supports:
- Create GPT or MBR partition table
- Add/remove/list partitions with type GUIDs
- Write EFI System Partition entry (type GUID `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`)
- Write BIOS Boot Partition entry for Limine BIOS mode
- Write Linux root partition entry

This tool would be MIT-licensed and become a jonerix core package.

#### Option B: Port `gpart` from FreeBSD (BSD-3-Clause)

FreeBSD's `gpart` utility handles GPT/MBR partitioning and is BSD-3-Clause licensed. However,
it depends on FreeBSD's GEOM framework and libc extensions. Porting it to Linux/musl would
require significant adaptation. The custom `jpart` approach (Option A) is more practical.

#### Option C: Use sgdisk at install time only (not in rootfs)

During the installer phase (running from Alpine/live environment), GPL tools like `sfdisk` or
`sgdisk` can be used to partition the disk, then the installed jonerix rootfs never contains
them. This is analogous to how Alpine is used as the build host. The installer does not need to
be part of the runtime image.

**Recommendation**: Option C for the initial bare metal installer (use GPL tools in the
installer environment, not in the installed rootfs), with Option A (`jpart`) as a medium-term
goal to make jonerix self-sufficient for re-partitioning at runtime.

### 2.3 FAT32 Filesystem Creation (EFI System Partition)

The EFI System Partition must be FAT32 per the UEFI specification.

#### GPL tools (cannot use for runtime)

| Tool | License | Source |
|------|---------|--------|
| `mkfs.fat` / `mkdosfs` | GPL-3.0 | dosfstools |
| `mkfs.vfat` | GPL (via util-linux) | util-linux |

#### Permissive option: Port `newfs_msdos` from FreeBSD

FreeBSD's `newfs_msdos` creates FAT12/16/32 filesystems and is licensed **BSD-2-Clause**
(confirmed in `sbin/newfs_msdos/newfs_msdos.c` SPDX header).

**Source**: `https://github.com/freebsd/freebsd-src/tree/main/sbin/newfs_msdos`

Porting effort: Medium. The tool uses standard POSIX file I/O. FreeBSD-specific calls like
`getdiskbyname()` would be stubbed out or replaced. Estimated ~200 lines of adaptation.

The result would be a `jmkfat` (or `newfs_msdos`) package in jonerix, MIT-wrapped around the
BSD-2-Clause FreeBSD code.

#### Alternative: Build mtools (MIT/LGPL mixed)

`mtools` provides `mformat` for FAT creation. The core is MIT-like but some components are LGPL.
This is not clean enough for jonerix's policy.

**Recommendation**: Port `newfs_msdos` from FreeBSD as package `newfs_msdos` or `jmkfat`.
For the initial installer, use `mkfs.fat` in the Alpine/live installer environment (not in
the installed rootfs).

### 2.4 ext4 Filesystem Creation and Management

ext4 is the recommended root filesystem. It is well-supported by the Linux kernel (GPLv2,
already accepted), but the userspace tools have a mixed license situation.

#### e2fsprogs license analysis

e2fsprogs provides `mke2fs`/`mkfs.ext4`, `e2fsck`, `tune2fs`, `resize2fs`, `dumpe2fs`, etc.

The licensing within e2fsprogs is split:
- **`libext2fs`** (the core library): MIT licensed
- **`libcom_err`**: MIT licensed
- **`libss`**: MIT licensed
- **Binary programs** (`mke2fs`, `e2fsck`, `tune2fs`, `resize2fs`): GPL-2.0

This means the libraries are permissive, but the tools are GPL. The situation is similar to
util-linux.

#### Option A: Write minimal `jmkext4` using libext2fs directly

`libext2fs` (MIT) exposes a full API for creating and manipulating ext2/3/4 filesystems. A
small C program using this API to create a basic ext4 filesystem (equivalent to
`mke2fs -t ext4`) would be ~300-500 lines and could be MIT-licensed, linking only against
the MIT-licensed `libext2fs`.

This is the cleanest path for runtime ext4 creation on jonerix.

#### Option B: Use mke2fs at install time only (not in rootfs)

Same approach as Option C for partitioning: use GPL `mke2fs` in the Alpine/live installer
environment. The installed jonerix rootfs does not contain it. The root filesystem, once
created, just needs the kernel to mount it — no userspace ext4 tools are needed for normal
operation.

For runtime tasks like `fsck` on boot, toybox's `fsck` wrapper can be configured to call
a stripped-down permissive checker, or a jonerix-specific ext4 fsck can be written using
`libext2fs` (MIT).

#### Option C: Use F2FS as root filesystem

F2FS (Flash-Friendly File System) was developed by Samsung and its userspace tools
(`f2fs-tools`) are LGPL-2.1. Like the LGPL-vs-permissive analysis for systemd-boot, LGPL
is not in jonerix's accepted list.

#### Option D: Use UFS via FreeBSD's tools (BSD-3-Clause)

FreeBSD's `newfs` (UFS) is BSD-3-Clause. The Linux kernel has read/write UFS support but
it has historically been less reliable for UFS2. Porting `newfs` to Linux/musl is feasible
but UFS on Linux is not production-ready.

**Recommendation**: Option B (installer-time `mke2fs`) for immediate functionality.
Option A (jmkext4 via libext2fs MIT) as the medium-term solution for a fully permissive
runtime.

### 2.5 Summary: What Needs to Be Built/Ported

| Gap | Immediate Solution | Long-term (Permissive) |
|-----|-------------------|----------------------|
| Disk partitioning | Use sfdisk/sgdisk in installer env (GPL, not in rootfs) | `jpart` — custom C tool, MIT |
| FAT32 creation (ESP) | Use mkfs.fat in installer env (GPL, not in rootfs) | Port `newfs_msdos` from FreeBSD (BSD-2-Clause) |
| ext4 creation | Use mke2fs in installer env (GPL, not in rootfs) | `jmkext4` — C tool using libext2fs (MIT) |
| ext4 fsck | toybox fsck wrapper (0BSD) | `jfsck` — C tool using libext2fs (MIT) |
| Mount/unmount | toybox (already enabled, 0BSD) | Already solved |
| Swap management | toybox mkswap/swapon/swapoff (already enabled, 0BSD) | Already solved |
| Block device info | toybox blkid/blockdev (already enabled, 0BSD) | Already solved |
| Loop devices | toybox losetup (already enabled, 0BSD) | Already solved |

---

## 3. Partition Scheme

### 3.1 Recommended: GPT + UEFI

```
Device: /dev/sda (example)

Partition 1: EFI System Partition
  Type:    EFI System (GUID C12A7328-...)
  Size:    512 MiB
  FS:      FAT32
  Mount:   /boot/efi (or mounted only during install/upgrade)
  Content: /EFI/BOOT/BOOTX64.EFI   <- Limine EFI binary
           /boot/limine/limine.conf <- Limine config
           /boot/vmlinuz            <- jonerix kernel

Partition 2: Root filesystem
  Type:    Linux filesystem (GUID 0FC63DAF-...)
  Size:    Remaining disk space
  FS:      ext4
  Mount:   /
  Content: jonerix rootfs (merged-usr layout)
```

No separate `/boot` partition is needed because Limine can read the kernel directly from the
ESP. The ESP acts as both the EFI partition and the boot partition.

If a dedicated boot partition is preferred (e.g., for clarity or if the root FS is encrypted):

```
Partition 1: EFI System Partition   512 MiB  FAT32
Partition 2: Boot partition         256 MiB  FAT32 or ext4
Partition 3: Root filesystem        remaining ext4
```

### 3.2 BIOS Legacy: GPT + BIOS Boot Partition

```
Partition 1: BIOS Boot Partition
  Type:    BIOS boot (GUID 21686148-...)
  Size:    1 MiB
  FS:      None (raw, no filesystem)
  Purpose: Limine BIOS code installed here by: limine bios-install /dev/sda 1

Partition 2: Boot/Root filesystem
  Type:    Linux filesystem
  Size:    Remaining disk space
  FS:      ext4
  Mount:   /
  Content: /boot/limine/limine-bios.sys
           /boot/limine/limine.conf
           /boot/vmlinuz
           Full jonerix rootfs
```

### 3.3 BIOS Legacy: MBR (simplest)

```
Partition 1: Root filesystem
  Type:    0x83 Linux
  Size:    Full disk
  FS:      ext4
  Flags:   bootable
  Mount:   /

Limine installed to MBR: limine bios-install /dev/sda
limine-bios.sys placed in /boot/limine/ on root partition
```

### 3.4 Merged-usr Considerations

jonerix uses merged-usr: `/usr -> /` (symlink). This has no impact on the bootloader or
partition layout since the kernel, init, and all binaries are at standard paths regardless
of the usr merge.

The boot sequence remains as documented in DESIGN.md:
```
/boot/vmlinuz  ->  kernel command line: init=/bin/openrc-init root=/dev/sda2
                ->  /bin/openrc-init (PID 1)
                ->  OpenRC service graph
```

---

## 4. Installation Workflow

### 4.1 Installer Architecture

The bare metal installer runs from a live medium (USB drive or ISO). It does not need to be
a jonerix image — it can be an Alpine-based installer that:

1. Partitions the target disk (using GPL sfdisk/sgdisk — acceptable in installer environment)
2. Formats the ESP with mkfs.fat (GPL — acceptable in installer environment)
3. Formats the root partition with mke2fs/mkfs.ext4 (GPL — acceptable in installer environment)
4. Mounts both partitions
5. Unpacks the jonerix rootfs tarball (jpkg archive or compressed tar)
6. Installs Limine (Limine EFI binary is BSD-2-Clause — goes into the installed rootfs/ESP)
7. Writes limine.conf with the correct root device UUID
8. Optionally: runs efibootmgr to register the UEFI boot entry (GPL, installer-only)
9. Unmounts and reboots

The installed jonerix rootfs never contains the GPL installer tools. The Limine EFI binary
on the ESP is the only bootloader-related permissive binary that persists.

### 4.2 From-Container to Bare Metal

jonerix is primarily container-native. Converting a container image to a bootable disk image:

```sh
# Step 1: Create raw disk image
dd if=/dev/zero of=jonerix.img bs=1M count=4096

# Step 2: Partition (installer environment, GPL tools OK here)
sfdisk jonerix.img << EOF
label: gpt
1 : size=512MiB, type=uefi
2 : type=linux
EOF

# Step 3: Format partitions
LOOP=$(losetup --partscan --find --show jonerix.img)
mkfs.fat -F32 ${LOOP}p1          # GPL tool, installer-only
mkfs.ext4 ${LOOP}p2              # GPL tool, installer-only

# Step 4: Install rootfs
mount ${LOOP}p2 /mnt
mount ${LOOP}p1 /mnt/boot/efi
# Unpack jonerix container/tarball
tar -xzf jonerix-rootfs.tar.gz -C /mnt

# Step 5: Install Limine (BSD-2-Clause)
mkdir -p /mnt/boot/efi/EFI/BOOT
cp limine/BOOTX64.EFI /mnt/boot/efi/EFI/BOOT/
cat > /mnt/boot/efi/boot/limine/limine.conf << 'EOF'
timeout: 3
default_entry: 1

/ jonerix
    protocol: linux
    path: boot():/vmlinuz
    cmdline: root=UUID=<ROOT_UUID> rootfstype=ext4 init=/bin/openrc-init quiet
EOF

# Step 6: Clean up
umount /mnt/boot/efi
umount /mnt
losetup -d $LOOP
```

### 4.3 initramfs Considerations

**Does jonerix need an initramfs?**

An initramfs is required when:
1. The root filesystem driver is not built into the kernel
2. The root device requires setup before mounting (LVM, LUKS, software RAID, network root)
3. The root device name changes between boots (e.g., USB drives with changing enumeration)

**For a simple bare metal install** (ext4 root on a directly-attached SATA/NVMe disk), if the
ext4 driver and the disk controller driver are compiled into the kernel (not as modules), then
**no initramfs is required**. The kernel can mount root directly.

jonerix's current DESIGN.md boot sequence does not include an initramfs step, which is consistent
with this: the kernel is expected to have all needed drivers built in.

For a general-purpose installer that must work across hardware, a minimal initramfs (containing
just enough to load the necessary kernel modules) would be needed. Tools for this:

- `mkinitfs` from Alpine (MIT licensed) — the cleanest permissive option
- A custom `jmkinitfs` shell script using toybox's cpio support

**Recommendation**: For the first bare metal target (known hardware, e.g., VM or specific server),
build the kernel with all needed drivers compiled in. Defer initramfs support to when it is
needed (cloud images, diverse hardware support).

### 4.4 Secure Boot

Secure Boot requires the kernel EFI binary (either the kernel as EFISTUB or the Limine EFI
binary) to be signed with a key trusted by the platform firmware.

Options:
1. **MOK (Machine Owner Key)**: Enroll a custom key in the UEFI firmware's MOK database using
   `mokutil` (GPL, install-time only). Sign Limine and the kernel with the custom key using
   `sbsign` (MIT) or `pesign` (MPL-2.0, acceptable).
2. **Disable Secure Boot**: For controlled environments (VMs, dedicated hardware), simply
   disable Secure Boot in firmware settings.
3. **Use pre-signed Limine**: If Microsoft-signed or distro-signed Limine binaries become
   available, use them (Limine is working toward this).

Secure Boot is deferred to a future TODO item (DESIGN.md already notes this).

---

## 5. License Analysis Summary

| Component | License | Acceptable? | Notes |
|-----------|---------|------------|-------|
| Limine bootloader (EFI/BIOS binary) | BSD-2-Clause | YES | Primary bootloader recommendation |
| Linux kernel EFISTUB | GPLv2 | YES (kernel exception) | No additional binary needed for UEFI |
| GRUB 2 | GPL-3.0 | NO | Rejected |
| SYSLINUX | GPL-2.0 | NO | Rejected |
| rEFInd | GPL-3.0 | NO | Rejected |
| systemd-boot | LGPL-2.1 | NO | Copyleft, not on jonerix accepted list |
| U-Boot | GPL-2.0 | NO | GPL runtime |
| toybox (mount, blkid, etc.) | 0BSD | YES | Already in jonerix |
| fdisk (util-linux) | GPL-1.0+ | NO | Installer-only acceptable |
| sfdisk (util-linux) | GPL-2.0+ | NO | Installer-only acceptable |
| gdisk / sgdisk | GPL-2.0 | NO | Installer-only acceptable |
| parted (GNU) | GPL-3.0 | NO | Installer-only acceptable |
| jpart (proposed) | MIT | YES | Custom tool, to be written |
| mkfs.fat (dosfstools) | GPL-3.0 | NO | Installer-only acceptable |
| newfs_msdos (FreeBSD port) | BSD-2-Clause | YES | For FAT32/ESP creation in runtime |
| mke2fs (e2fsprogs binary) | GPL-2.0 | NO | Installer-only acceptable |
| libext2fs (e2fsprogs library) | MIT | YES | Foundation for jmkext4 |
| jmkext4 (proposed) | MIT | YES | Custom tool using libext2fs (MIT) |
| mtools (mformat) | LGPL/MIT mixed | BORDERLINE | Avoid |
| efibootmgr | GPL-2.0 | NO | Installer-only acceptable |

---

## 6. Recommended Package Roadmap

### Phase 1: Bare Metal Installer (uses GPL tools in installer env, not in rootfs)

No new jonerix packages needed. Create an installer script and documentation that:
- Runs from Alpine live environment
- Partitions with `sfdisk` or `sgdisk`
- Formats with `mkfs.fat` and `mkfs.ext4`
- Unpacks jonerix rootfs tarball
- Places Limine EFI binary on ESP
- Writes `limine.conf`

New file: `install/bare-metal-install.sh` — the installer script

### Phase 2: Limine Package

**Package**: `limine`
**License**: BSD-2-Clause
**Version**: Latest stable (v11.x series)
**Provides**: `BOOTX64.EFI`, `BOOTAA64.EFI`, `limine-bios.sys`, `limine` (host utility for
BIOS install), `limine-bios-pxe.bin`
**Build deps**: `nasm`, `clang`, `make` (all build-time only)
**Runtime deps**: none (EFI binary is self-contained; `limine` host utility links musl)

The Limine package would install:
- `/bin/limine` — the host utility (for `limine bios-install`)
- `/share/limine/BOOTX64.EFI` — UEFI bootloader binary (x86_64)
- `/share/limine/BOOTAA64.EFI` — UEFI bootloader binary (aarch64)
- `/share/limine/limine-bios.sys` — BIOS bootloader code
- `/share/limine/limine-bios-pxe.bin` — PXE binary
- `/share/limine/limine.conf.example` — example configuration

### Phase 3: newfs_msdos Package (for runtime FAT32 creation)

**Package**: `newfs_msdos`
**License**: BSD-2-Clause (FreeBSD port)
**Provides**: `newfs_msdos` — creates FAT12/16/32 filesystems
**Build deps**: `clang`, `make`
**Runtime deps**: `musl`
**Adaptation needed**: Replace `getdiskbyname()` and other FreeBSD-isms with Linux equivalents

### Phase 4: libext2fs + jmkext4 (for runtime ext4 creation)

**Package**: `e2fsprogs-libs`
**License**: MIT (libext2fs, libcom_err, libss only — not the GPL binaries)
**Provides**: `libext2fs.so`, `libcom_err.so`
**Build deps**: `clang`, `make`
**Note**: Build ONLY the MIT-licensed library components, not the GPL binary tools

**Package**: `jmkext4`
**License**: MIT
**Provides**: `jmkext4` — creates ext4 filesystems using libext2fs
**Build deps**: `clang`, `e2fsprogs-libs`
**Runtime deps**: `musl`, `e2fsprogs-libs`

### Phase 5: jpart (for runtime partitioning, self-hosting)

**Package**: `jpart`
**License**: MIT
**Provides**: `jpart` — GPT/MBR partitioning utility
**Build deps**: `clang`
**Runtime deps**: `musl`
**Implementation**: Raw C using Linux ioctls and direct disk writes, no external library deps

---

## 7. Open Questions

1. **Installer medium**: Should the bare metal installer be a jonerix ISO (using Limine for
   the ISO boot itself) or an Alpine-based installer that installs jonerix? The ISO approach
   is more elegant but requires the initramfs problem to be solved first.

2. **UUID vs device path**: Limine config can use `root=UUID=...` to avoid device naming
   fragility. The installer needs to detect and embed the root partition UUID using `blkid`
   (available in toybox).

3. **Swap partition vs swap file**: A swap file on ext4 works on Linux 5.0+. This avoids a
   dedicated swap partition and simplifies the installer.

4. **aarch64 / Raspberry Pi (TODO #18)**: Raspberry Pi 4/5 support UEFI via the
   pftf/RPi4-UEFI-Firmware project (MIT). With UEFI firmware installed on the Pi, Limine's
   UEFI path works unmodified. Raspberry Pi 3 requires the proprietary GPU bootloader; Limine
   aarch64 supports it via the DTB path.

5. **Kernel config**: TODO #17 (custom kernel build) will need to ensure `CONFIG_EFI_STUB=y`,
   `CONFIG_EXT4_FS=y` (built-in, not module), and the appropriate disk controller drivers are
   compiled in to support initramfs-free booting.

6. **NASM license**: NASM uses a BSD-2-Clause license. It is a build-time dependency for
   Limine and could be added as a jonerix development tool.
