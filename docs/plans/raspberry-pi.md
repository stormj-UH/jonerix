# Raspberry Pi Support Plan

## Overview

This document covers the prerequisites, design decisions, and work required to run jonerix on
Raspberry Pi hardware. The primary target is bare-metal SD card boot (not container-in-QEMU).
The zero-GPL-runtime constraint remains fully in force — the Broadcom GPU firmware and Linux
kernel are the only non-permissive pieces that land on the device, both of which are already
accepted exceptions in the project's license policy.

---

## 1. Supported Models

Only models with a 64-bit ARM Cortex-A core (ARMv8-A / AArch64) are in scope. jonerix already
builds aarch64 packages, so no new cross-compile chain is needed.

| Model | SoC | CPU | 64-bit | Notes |
|---|---|---|---|---|
| Raspberry Pi 3B | BCM2837 | Cortex-A53 | Yes | First 64-bit Pi; 1 GB RAM |
| Raspberry Pi 3B+ | BCM2837B0 | Cortex-A53 | Yes | Better thermals; 1 GB RAM |
| Raspberry Pi 3A+ | BCM2837B0 | Cortex-A53 | Yes | Slim form factor; 512 MB RAM |
| Raspberry Pi 4B | BCM2711 | Cortex-A72 | Yes | 1/2/4/8 GB variants; EEPROM bootloader |
| Raspberry Pi 400 | BCM2711 | Cortex-A72 | Yes | Keyboard form factor; same as 4B |
| Raspberry Pi CM4 | BCM2711 | Cortex-A72 | Yes | Compute Module; no SD card by default |
| Raspberry Pi 5 | BCM2712 | Cortex-A76 | Yes | EEPROM bootloader; PCIe; newest |
| Raspberry Pi Zero 2 W | RP3A0 | Cortex-A53 | Yes | Low-power; 512 MB RAM |

**Recommended starting point**: Raspberry Pi 4B (4 GB). It has the widest mainline kernel
support, enough RAM for a comfortable jonerix development loop, and USB 3.0 for fast SD writing.
Raspberry Pi 5 is the most capable but mainline kernel coverage for some peripherals (RP1 chip,
PCIe, Ethernet behind RP1) is still maturing as of 2025-2026.

**Models NOT in scope**: All Pi 1, Pi 2 (v1.1 and earlier), and Pi Zero (original) — these are
32-bit ARMv6/ARMv7 only and incompatible with the aarch64 userland.

---

## 2. Firmware Requirements

### 2.1 The Boot Partition

The FAT32 boot partition on the SD card must contain:

```
/boot/
  bootcode.bin          # Pi 3 and earlier only (GPU second-stage bootloader)
  start.elf             # GPU firmware (VideoCore IV / VI)
  start4.elf            # Pi 4-specific GPU firmware
  fixup.dat             # Linker file matching start.elf
  fixup4.dat            # Pi 4-specific linker file
  config.txt            # Machine configuration (replaces BIOS setup)
  cmdline.txt           # Kernel command-line arguments
  bcm2710-rpi-3-b.dtb   # Device tree blob (Pi 3B example)
  bcm2711-rpi-4-b.dtb   # Device tree blob (Pi 4B)
  bcm2712-rpi-5-b.dtb   # Device tree blob (Pi 5)
  overlays/             # Optional DT overlay .dtbo files
  kernel8.img           # 64-bit kernel image (arm64)
```

### 2.2 Pi 4 and Pi 5: EEPROM Bootloader

On Pi 4 and Pi 5, the second-stage bootloader is stored in on-board SPI flash EEPROM rather
than as `bootcode.bin` on the SD card. This is managed by the `rpi-eeprom` package
(maintained by the Raspberry Pi Foundation). The SD card boot partition still needs
`start4.elf` / `fixup4.dat` / `config.txt` and the kernel image — the EEPROM loader reads
those. `bootcode.bin` is not used on Pi 4 and Pi 5.

Pi 3 and earlier still require `bootcode.bin` on the FAT32 partition.

### 2.3 Firmware License: Broadcom Proprietary — Redistributable

The GPU firmware files (`start*.elf`, `fixup*.dat`, `bootcode.bin`) are Broadcom closed-source
binaries. Their license is described in `boot/LICENCE.broadcom` in the
[raspberrypi/firmware](https://github.com/raspberrypi/firmware) repository.

Key terms of LICENCE.broadcom:
- Redistribution and use **in binary form, without modification** is permitted.
- Free of charge for use with Raspberry Pi hardware.
- No source code will ever be provided.
- The firmware may only be used with Raspberry Pi hardware.

**Assessment for jonerix**: These binaries are in the same category as the Linux kernel — a
necessary non-permissive hardware interface layer. The Broadcom license is not a copyleft license
(it imposes no reciprocal source-disclosure obligations on the rest of the OS). Including these
files in the boot partition of a jonerix SD image is acceptable under the project's existing
hardware-interface exception, just as the GPLv2 kernel is accepted. They should be explicitly
documented in the image license notes alongside the kernel.

The firmware files are obtained from: https://github.com/raspberrypi/firmware/tree/master/boot

### 2.4 config.txt — Key Options for jonerix

```ini
# Enable 64-bit kernel (required for aarch64 userland)
arm_64bit=1

# Kernel image name (must match filename on FAT partition)
kernel=kernel8.img

# Enable UART for serial console (useful during bring-up)
enable_uart=1

# GPU memory split — minimal for headless server use
gpu_mem=16

# Disable rainbow splash
disable_splash=1

# Device tree overlays for hardware you need
# dtoverlay=i2c-arm          # I2C
# dtoverlay=spi0-1cs         # SPI
# dtoverlay=uart1            # Second UART
```

### 2.5 cmdline.txt — Kernel Arguments

```
console=serial0,115200 console=tty1 root=/dev/mmcblk0p2 rootfstype=ext4 rootwait quiet
```

For initramfs-less boot (jonerix default), `rootwait` is essential — the SD card controller
enumerates asynchronously and the root device may not be ready at the instant the kernel first
looks for it.

---

## 3. Kernel Considerations

### 3.1 Raspberry Pi Kernel vs. Mainline Linux

| Factor | RPi kernel (`raspberrypi/linux`) | Mainline (`torvalds/linux`) |
|---|---|---|
| Hardware support | Complete for all models | Pi 3 excellent; Pi 4 very good; Pi 5 partial (RP1 still landing) |
| Video codecs (HW) | Yes (proprietary MMAL/V4L2 M2M) | Partial (V4L2 stateless codec) |
| Camera (CSI) | Yes (libcamera IPA) | Partial |
| DT overlay loader | Full raspi-config support | Limited in mainline |
| GPIO/I2C/SPI/UART | Full | Full |
| WiFi (brcmfmac) | Full | Full (driver is upstream) |
| Stable for server use | Yes | Yes for Pi 3/4 |

**Recommendation**: Use the RPi Foundation kernel fork for the initial bring-up work. It is
simply less friction. The goal is a working jonerix image, not a mainline advocacy project.
Mainline can be revisited once the rest of the stack works.

The RPi kernel is available at https://github.com/raspberrypi/linux. It tracks mainline closely
(typically within one or two major versions) and adds RPi-specific device tree overlays,
platform drivers, and the DT overlay loader (`dtoverlay`).

The kernel itself is GPLv2 — already an accepted exception in jonerix.

### 3.2 Kernel Configuration

Starting from `bcm2711_defconfig` (Pi 4) or `bcm2709_defconfig` (Pi 3) is strongly preferred
over `allmodconfig`. Key config groups to confirm are enabled:

**Platform / boot:**
```
CONFIG_ARCH_BCM2835=y
CONFIG_ARM64=y
CONFIG_EFI_STUB=y            # Optional: enables UEFI boot path
CONFIG_OF=y                  # Device tree
CONFIG_OF_OVERLAY=y          # DT overlay loader
```

**Storage:**
```
CONFIG_MMC=y
CONFIG_MMC_BCM2835=y         # SD card controller
CONFIG_EXT4_FS=y
CONFIG_VFAT_FS=y             # FAT32 boot partition
```

**Networking:**
```
CONFIG_NET=y
CONFIG_ETHERNET=y
CONFIG_USB_NET_DRIVERS=y     # USB Ethernet dongles
CONFIG_CFG80211=y
CONFIG_MAC80211=y
CONFIG_BRCMFMAC=m            # Broadcom WiFi (brcmfmac)
CONFIG_BRCM_TRACING=n
CONFIG_BT=m
CONFIG_BT_HCIUART=m          # Bluetooth over UART
CONFIG_BT_BCM=m              # Broadcom BT firmware loader
```

**I/O buses:**
```
CONFIG_I2C_BCM2835=y
CONFIG_SPI_BCM2835=y
CONFIG_SERIAL_AMBA_PL011=y   # UART
CONFIG_GPIOLIB=y
CONFIG_GPIO_SYSFS=y          # Legacy sysfs GPIO (for tools)
CONFIG_GPIO_CDEV=y           # Modern chardev GPIO (libgpiod)
```

**GPU / display (headless server: modules only):**
```
CONFIG_DRM=m
CONFIG_DRM_VC4=m             # Pi 3/Zero2W VideoCore IV
CONFIG_DRM_V3D=m             # Pi 4/5 VideoCore VI
CONFIG_DRM_PANEL_SIMPLE=m
```

**Modules needed at boot (build in, not as modules):**
- `bcm2835-sdhost` or `bcm2835-mmc` (SD card)
- `ext4`, `vfat` (filesystems)
- `brcmfmac` can be a module loaded by OpenRC

### 3.3 Kernel Modules

The kernel module loading at runtime is handled by OpenRC. A `modules` service loading a
`/etc/modules` file (or OpenRC `conf.d/modules`) covers the common case:

```
# /etc/modules  (loaded by OpenRC at boot)
brcmfmac         # WiFi
btbcm            # Broadcom Bluetooth base
hci_uart         # Bluetooth UART transport
i2c-dev          # I2C userspace access
spi-dev          # SPI userspace access
```

---

## 4. Boot Process

### 4.1 Raspberry Pi 3 Boot Sequence

```
Power on
  → GPU (VideoCore) executes ROM bootloader from on-chip mask ROM
  → Reads FAT32 boot partition: loads bootcode.bin into L2 cache
  → bootcode.bin starts SDRAM, loads start.elf + fixup.dat
  → start.elf (GPU OS) reads config.txt
  → Loads kernel8.img + device tree blob into RAM
  → Releases ARM CPU reset → Linux kernel starts
  → OpenRC PID 1
```

### 4.2 Raspberry Pi 4 / 5 Boot Sequence

```
Power on
  → VPU executes SPI EEPROM bootloader (replaces bootcode.bin)
  → EEPROM bootloader reads config.txt from FAT32 partition
  → Loads start4.elf + fixup4.dat (Pi 4) or equivalent (Pi 5)
  → GPU firmware reads config.txt, loads kernel8.img + DTB
  → ARM CPU starts → Linux kernel
  → OpenRC PID 1
```

### 4.3 No GRUB Required

This is a key advantage for jonerix. The RPi GPU firmware acts as the first-stage bootloader
and directly loads the Linux kernel — no GRUB or U-Boot is in the chain at all. The kernel
command line comes from `cmdline.txt` on the FAT32 partition, not from a GRUB config.

U-Boot (GPL) can be inserted as an optional intermediary for features like network boot or
A/B update schemes, but it is not needed for a straightforward SD card install and introduces
a GPL layer. Avoid it unless A/B updates become a requirement.

An alternative to U-Boot for future UEFI-style booting: the [pftf/RPi4](https://github.com/pftf/RPi4)
and [worproject/rpi5-uefi](https://github.com/worproject/rpi5-uefi) projects provide EDK2-based
UEFI firmware images. EDK2 is licensed under BSD-2-Clause Plus Patent — fully permissive. This
would enable EFISTUB-style booting (consistent with how jonerix handles x86_64 UEFI), though
it adds EEPROM flashing complexity and is not needed for basic SD card boot.

### 4.4 initramfs

jonerix does not currently use an initramfs on x86_64. The same approach works for RPi:
compile the SD card driver and ext4 directly into the kernel (`=y`, not `=m`) and use
`rootwait` in `cmdline.txt`. This keeps the boot partition simple and avoids a mkinitramfs
dependency.

If encrypted root (`dm-crypt`) becomes a future requirement, initramfs will be necessary at
that point.

---

## 5. Hardware Support Packages

### 5.1 WiFi Firmware — brcmfmac (Broadcom/Cypress/Infineon)

The WiFi chips on all RPi models with built-in WiFi use Broadcom's brcmfmac driver, with
the following chips per model:

| Model | Chip | Firmware files |
|---|---|---|
| Pi 3B / 3B+ / Zero 2 W | CYW43430 / CYW43455 | `brcmfmac43430-sdio.*`, `brcmfmac43455-sdio.*` |
| Pi 4B / 400 / CM4 | CYW43455 | `brcmfmac43455-sdio.*` |
| Pi 5 | CYW43455 (or successor) | `brcmfmac43455-sdio.*` |

The firmware binary files are proprietary Cypress/Infineon blobs. The `brcmfmac` kernel driver
itself is open-source (dual GPL/BSD), but the firmware blobs it loads are not GPL.

**License**: The firmware files are distributed under a Cypress/Infineon "Redistributable" proprietary
license — similar in character to the Broadcom GPU firmware. They are free to redistribute but
not to modify, and must only run on compatible hardware. They are shipped by the Linux kernel's
companion firmware repository (`linux-firmware`) and by `RPi-Distro/firmware-nonfree`.

**Assessment for jonerix**: Same exception category as the GPU firmware and the kernel.
The WiFi firmware blobs should be documented alongside the other non-permissive hardware
interface files. They must be present in `/lib/firmware/brcm/` for the `brcmfmac` module to
load WiFi.

Sources:
- `linux-firmware` repository: https://git.kernel.org/pub/scm/linux/kernel/git/firmware/linux-firmware.git
- `RPi-Distro/firmware-nonfree`: https://github.com/RPi-Distro/firmware-nonfree

### 5.2 Bluetooth Firmware

Bluetooth on the RPi uses the same Broadcom/Cypress chip as WiFi (combo chip). It requires
HCI firmware (`.hcd` file) loaded by the `btbcm` kernel module. These files live in
`/lib/firmware/brcm/`:

```
BCM43430A1.hcd   # Pi 3B
BCM4345C0.hcd    # Pi 3B+ / Pi 4
```

Same license status as the WiFi firmware — Cypress Redistributable.

The `hciattach` tool (from BlueZ) is GPL. A permissive alternative:
- The kernel `hci_uart` module handles the UART attachment directly when loaded with
  the right parameters; `btattach` (also in BlueZ, GPL) orchestrates it.
- For a jonerix image that does not need interactive Bluetooth pairing, the kernel modules alone
  may suffice for headless use cases (BLE peripheral mode, serial-over-BT). For a full BT stack
  needing pairing, BlueZ is GPL — this is a **gap that needs a decision**.
- Alternatives to BlueZ: `btstack` (MIT-licensed, by BlueKitchen) is a complete Bluetooth stack
  with a permissive license. It is widely used in embedded systems. This is the preferred path.

### 5.3 GPU Acceleration — Mesa vc4 / v3d

| Driver | GPU | Pi Models |
|---|---|---|
| `vc4` | VideoCore IV | Pi 3, Zero 2 W |
| `v3d` | VideoCore VI | Pi 4, Pi 5 |

Mesa's vc4 and v3d drivers are fully open-source, maintained upstream in the Mesa project, and
licensed under the **MIT license** — fully permissive, no GPL anywhere in the Mesa userspace
stack. They talk to the `drm/vc4` and `drm/v3d` kernel DRM drivers (GPLv2, kernel exception).

GPU acceleration is not needed for jonerix's initial server/IoT focus, but it enables:
- Hardware-accelerated OpenGL/Vulkan for future desktop/kiosk use
- Video decode acceleration via V4L2 stateless API

Mesa is already a candidate for a future recipe. The dependency chain (libdrm, wayland-scanner
if building the Wayland EGL platform) is non-trivial but entirely permissively licensed.

### 5.4 Camera Interface

The Raspberry Pi camera uses the CSI-2 interface. The modern open-source stack is:

```
Hardware (CSI sensor) → kernel VC4/ISP drivers → libcamera → rpicam-apps
```

**libcamera**: The core library is LGPL-2.1-or-later. The Raspberry Pi IPA (Image Processing
Algorithm) module is BSD-2-Clause. The `cam` and `qcam` sample apps are GPL-2.0 but are
not required at runtime.

**Assessment**: libcamera itself (LGPL) is a linkage concern but not a distribution concern
for a server image. If camera support is wanted, libcamera can be linked dynamically (LGPL
permits dynamic linking without copyleft obligation). For jonerix's initial RPi scope, camera
support is **deferred** — it adds significant complexity (build dependencies, ISP tuning files,
kernel V4L2 configuration) and is not needed for the server/IoT baseline.

### 5.5 GPIO Tools

| Library | License | Status |
|---|---|---|
| `libgpiod` | LGPL-2.1-or-later (core C library) | Usable via dynamic linking |
| `libgpiod` C++ bindings | LGPL-3.0-or-later (v2.0+) | Avoid if strict permissive required |
| `gpiod` command-line tools | GPL-2.0-or-later | Cannot ship |
| `rppal` (Rust) | MIT | Excellent option for Rust users |
| Direct `/dev/gpiochip*` ioctl | kernel ABI (GPLv2 kernel accepted) | Always available |

**Recommendation for jonerix**:

The `libgpiod` C library (LGPL) can be dynamically linked — no copyleft obligation on the
jonerix userland. However, LGPL is not strictly permissive under the project's license policy.

The cleanest path for basic GPIO use is direct ioctl against `/dev/gpiochip*` using the
kernel's character device ABI (documented in `linux/gpio.h`). A small MIT-licensed `jgpio`
utility (~300 lines C) wrapping the ioctl interface would eliminate the libgpiod dependency
entirely for simple scripted GPIO control. This fits the jonerix pattern of writing small
permissive tools to replace GPL/LGPL equivalents.

For Rust users: `rppal` (MIT) is a pure-Rust GPIO library that talks directly to the kernel
character device ABI without libgpiod.

---

## 6. SD Card Image Creation

### 6.1 Partition Layout

Standard two-partition layout, consistent with official Raspberry Pi OS images:

```
Offset 0: MBR partition table (not GPT — RPi firmware expects MBR)
  Partition 1: FAT32, ~256 MB, type 0x0B or 0x0C
    → bootcode.bin (Pi 3), start*.elf, fixup*.dat
    → config.txt, cmdline.txt
    → kernel8.img
    → *.dtb device tree blobs
    → overlays/
    → lib/firmware/brcm/ (WiFi + BT firmware blobs)
  Partition 2: ext4, remainder of card
    → jonerix rootfs (all jpkg-installed packages)
```

Note: Pi 5 supports GPT partition tables, but MBR is universally supported and keeps the
image generation logic simple for now.

### 6.2 Image Generation Process

The image can be created entirely with tools already available or buildable in jonerix
(or Alpine as the build host). No GPL runtime tools are needed in the final image.

**Step-by-step build process:**

```sh
# 1. Allocate image file (e.g., 2 GB for a minimal image)
dd if=/dev/zero of=jonerix-rpi.img bs=1M count=2048

# 2. Create MBR partition table and partitions
# (using toybox fdisk, or sfdisk if available — both permissive)
# Partition 1: FAT32, 256 MB, starts at sector 8192 (4 MB aligned)
# Partition 2: ext4, remainder

# 3. Format partitions using loop device
losetup -P /dev/loop0 jonerix-rpi.img
mkfs.fat -F 32 -n BOOT /dev/loop0p1
mkfs.ext4 -L root /dev/loop0p2

# 4. Mount and populate boot partition
mount /dev/loop0p1 /mnt/boot
# Copy firmware files, DTBs, kernel8.img, config.txt, cmdline.txt, WiFi firmware

# 5. Mount and populate root partition
mount /dev/loop0p2 /mnt/root
# Install jpkg packages into /mnt/root via:
#   jpkg --root /mnt/root install musl toybox mksh openrc dhcpcd dropbear ...

# 6. Write /etc/fstab, /etc/hostname, configure OpenRC services

# 7. Unmount, detach loop device
umount /mnt/boot /mnt/root
losetup -d /dev/loop0

# 8. Compress for distribution
zstd --ultra -19 jonerix-rpi.img -o jonerix-rpi-aarch64.img.zst
```

Tools required for image creation:
- `dd` (toybox, 0BSD)
- `losetup` (toybox or util-linux — util-linux is GPL; toybox's `losetup` is sufficient)
- `mkfs.fat` — this is from `dosfstools` (GPL). **Gap: need a permissive FAT formatter.**
  Alternative: `newfs_msdos` from FreeBSD (BSD-2-Clause) — already common in BSD systems.
  This would be a new recipe. Size is small (~10 KB binary).
- `mkfs.ext4` — from `e2fsprogs` (GPL for `mkfs.ext4` specifically). **Gap: need a permissive ext4 formatter.**
  Alternatives: `genext2fs` (GPL as well). This is a hard gap. Options:
  1. Accept GPL tools **at image-build time only** (same exception as GNU make during bootstrap).
     The GPL tools never ship in the final image.
  2. Use a pre-formatted ext4 tarball extraction approach (ship a minimal ext4 image template,
     resize, populate with `cp`).
  3. Use F2FS (Flash-Friendly File System) — the kernel module is GPLv2, but that is already
     accepted, and `f2fs-tools` has a more permissive license. Investigate this option.

**Recommended approach for v1**: Use `mkfs.fat` and `mkfs.ext4` at image-build time only
(Alpine build container, same pattern as all other build-time GPL tool use). They never appear
in the final image.

### 6.3 Flashing

Users flash the image with:
```sh
# On macOS/Linux
dd if=jonerix-rpi-aarch64.img of=/dev/sdX bs=4M status=progress
```

Or the Raspberry Pi Imager (FOSS, Apache-2.0) which can consume raw `.img` files and handles
card writing.

---

## 7. Networking

### 7.1 Wired Ethernet

jonerix already has everything needed:
- `dhcpcd` (BSD-2-Clause) — DHCP client
- `ifupdown-ng` (ISC) — interface configuration
- `dropbear` (MIT) — SSH server

For Pi 4: The built-in Ethernet (BCM54213PE, connected via USB 3.0 controller on BCM2711)
is supported by the `genet` driver in the RPi and mainline kernels. No extra packages needed.

For Pi 5: The built-in Ethernet is behind the RP1 companion chip. RP1 Ethernet support
landed in mainline around Linux 6.6-6.8. The RPi kernel has had it longer.

### 7.2 WiFi

Already have `wpa_supplicant` (BSD-3-Clause) recipe built and packaged. The connection
between the existing packages and RPi hardware WiFi requires:

1. WiFi firmware blobs in `/lib/firmware/brcm/` (see Section 5.1)
2. `brcmfmac` kernel module loaded (OpenRC modules service)
3. `wpa_supplicant` with nl80211 driver (already configured in the recipe)
4. `dhcpcd` for address assignment

This should work without any new package recipes.

A minimal `/etc/wpa_supplicant/wpa_supplicant.conf`:
```
ctrl_interface=/var/run/wpa_supplicant
update_config=1

network={
    ssid="YourNetwork"
    psk="YourPassword"
}
```

### 7.3 Bluetooth

As noted in Section 5.2, a GPL-free full Bluetooth stack requires `btstack` (MIT) as an
alternative to BlueZ (GPL). For the initial RPi release, Bluetooth can be marked as
**not supported** with a note that btstack is the planned permissive alternative.

---

## 8. License Concern Summary

| Component | License | Verdict |
|---|---|---|
| Linux kernel | GPLv2 | Accepted exception (existing policy) |
| GPU firmware (start.elf, bootcode.bin, fixup.dat) | Broadcom Proprietary Redistributable | Accepted: binary-only hardware interface, no copyleft |
| EEPROM bootloader (Pi 4/5) | Broadcom Proprietary Redistributable | Same as GPU firmware |
| WiFi firmware (brcmfmac, CYW43xxx) | Cypress/Infineon Proprietary Redistributable | Accepted: same category |
| Bluetooth firmware (.hcd files) | Cypress/Infineon Proprietary Redistributable | Accepted: same category |
| `brcmfmac` kernel driver | GPL-2.0 (in kernel) | Accepted: kernel exception |
| Mesa vc4 / v3d (GPU userspace) | MIT | Fully permissive |
| libcamera core | LGPL-2.1-or-later | Dynamic link acceptable; deferred for v1 |
| libgpiod C library | LGPL-2.1-or-later | Dynamic link acceptable; prefer direct ioctl or rppal |
| BlueZ | GPL-2.0 | **Blocked.** Use btstack (MIT) instead |
| U-Boot | GPL-2.0+ | **Blocked.** Not needed for SD card boot |
| gpiod CLI tools | GPL-2.0 | **Blocked.** Write jgpio (MIT) instead |
| dosfstools (mkfs.fat) | GPL-2.0 | Build-time only; never ships in image |
| e2fsprogs (mkfs.ext4) | GPL-2.0 | Build-time only; never ships in image |
| wpa_supplicant | BSD-3-Clause | Already packaged |
| dhcpcd | BSD-2-Clause | Already packaged |
| dropbear | MIT | Already packaged |
| OpenRC | BSD-2-Clause | Already packaged |

---

## 9. New Work Required (Recipe Gaps)

The following items do not yet have recipes and are required or desirable for RPi support:

### Tier 1 — Required for Functional Image

| Item | Description | License | Notes |
|---|---|---|---|
| RPi kernel build | Build `raspberrypi/linux` for aarch64 | GPLv2 | `bcm2711_defconfig` starting point; follows TODO #17 |
| RPi firmware package | Package start.elf, fixup.dat, DTBs, bootcode.bin from `raspberrypi/firmware` | Broadcom Proprietary | Download-only; no source build |
| WiFi/BT firmware package | Package brcmfmac firmware blobs from `linux-firmware` or `RPi-Distro/firmware-nonfree` | Cypress Proprietary | Download-only |
| SD image build script | Script to assemble FAT32 + ext4 image from jpkg rootfs | MIT (new script) | Uses Alpine build-time mkfs tools |

### Tier 2 — Needed for Complete Networking

| Item | Description | License | Notes |
|---|---|---|---|
| OpenRC WiFi service | Init script to start wpa_supplicant + dhcpcd on wlan0 | BSD/MIT | Config in `config/openrc/init.d/` |
| wpa_supplicant config template | Default template for `/etc/wpa_supplicant/` | n/a | Simple config file |

### Tier 3 — Nice to Have

| Item | Description | License | Notes |
|---|---|---|---|
| `jgpio` | Minimal GPIO CLI using kernel chardev ioctl | MIT (new) | ~300 lines C; replaces GPL gpiod CLI |
| `newfs_msdos` | BSD FAT32 formatter from FreeBSD | BSD-2-Clause | Eliminates dosfstools at image-build time |
| `btstack` | Full permissive Bluetooth stack | MIT | Replaces GPL BlueZ |

---

## 10. Dependency on TODO #17 (Custom Kernel)

Raspberry Pi support is tightly coupled to TODO #17 (build and customize a Linux kernel in
jonerix). The RPi kernel is a specific configuration of the Linux build process. The kernel
recipe work for x86_64 should establish the build infrastructure (make/samurai wrapper,
kernel config management, modules packaging) that the RPi aarch64 kernel recipe then reuses
with a different defconfig and source fork.

**Suggested sequencing:**
1. Complete TODO #17 for x86_64 (generic kernel recipe infrastructure)
2. Add RPi aarch64 kernel recipe reusing that infrastructure
3. Build RPi firmware and WiFi firmware download packages
4. Write SD image assembly script
5. Test on hardware (Pi 4B recommended)

---

## 11. References

- Raspberry Pi firmware repo: https://github.com/raspberrypi/firmware
- RPi kernel: https://github.com/raspberrypi/linux
- RPi EEPROM tools: https://github.com/raspberrypi/rpi-eeprom
- RPi 4 UEFI (EDK2): https://github.com/pftf/RPi4
- RPi 5 UEFI (EDK2): https://github.com/worproject/rpi5-uefi
- Broadcom firmware license: https://github.com/raspberrypi/firmware/blob/master/boot/LICENCE.broadcom
- linux-firmware: https://git.kernel.org/pub/scm/linux/kernel/git/firmware/linux-firmware.git
- RPi-Distro firmware-nonfree: https://github.com/RPi-Distro/firmware-nonfree
- brcmfmac Infineon firmware: https://github.com/ifx-linux/ifx-linux-firmware
- Mesa vc4 driver docs: https://docs.mesa3d.org/drivers/vc4.html
- Mesa v3d driver docs: https://docs.mesa3d.org/drivers/v3d.html
- libcamera: https://libcamera.org/
- btstack (MIT Bluetooth): https://github.com/bluekitchen/btstack
- libgpiod: https://git.kernel.org/pub/scm/libs/libgpiod/libgpiod.git
