TODO:

6. Add stormwall package. [EXTERNAL HOLD]
7. Add rush package. [EXTERNAL HOLD]
9. ~~Build hostapd recipe and jpkg.~~ [DONE — recipe exists, needs nl80211 headers + GNU make to build]
10. ~~Build wpa_supplicant recipe and jpkg.~~ [DONE — recipe exists, same blockers as hostapd]
14. Low-level file system utilities and Grub or some other bootloader for raw metal installs. This should require BYOK Bring/Build Your Own Kernel
      Since we do not distribute the kernel! Curl links to reliable prebuilt kernels can be selected, or the user can build their own from the          setup.
18. Build and customize a linux kernel in jonerix. (see also No.
19. Raspberry Pi pre-reqs.
20. Build Ruby recipe and jpkg and jmake

## Unpublished packages (have recipe, no .jpkg)

| Package | Blocker |
|---------|---------|
| ca-certificates | Was blocked by MPL-2.0 license — now fixed, needs build |
| hostapd | Needs nl80211 kernel headers + jmake |
| wpa_supplicant | Needs nl80211 kernel headers + jamke |
| ruby | Needs jmake  |
| linux | Kernel recipe — large project, separate effort |

## Missing architecture (have one arch, need the other)

| Package | Status |
|---------|--------|
| btop | arm64 only — need x86_64 |
| libatomic | arm64 only — need x86_64 |
| npm | arm64 only — need x86_64 |
| sqlite | arm64 only — need x86_64 |
| unzip | arm64 only — need x86_64 |

## Remaining work

- Build ca-certificates jpkg (both arches) — license unblocked
- Build ruby jpkg (both arches) — license unblocked, needs jmake
- Build x86_64 packages: btop, libatomic, npm, sqlite, unzip
- Build hostapd + wpa_supplicant jpkg (needs nl80211 headers packaged first)
- Linux kernel recipe + build
- Bootloader (grub alternative?) for bare metal
- Raspberry Pi support (device tree, kernel config)
- Create build-from-source.yml CI workflow (worker instructions at .claude/worker-instructions-build-from-source-ci.md)
