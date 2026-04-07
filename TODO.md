TODO:

1. ~~Complete fix on CI pipeline build of jonerix:all from source.~~ [DONE — disabled, OOM on 4GB runners]
2. ~~bmake extensions cleanup~~ [DONE — removed 27 legacy Makefiles, rules.mk, orphaned patch]
3. ~~Build Python 3.14 recipe and jpkg. Replace 3.12 packages.~~ [DONE — recipe updated to 3.14.3]
4. ~~Build sqlite and unzip recipes and jpkg.~~ [DONE — sqlite 3.51.3, unzip via libarchive bsdunzip]
5. ~~Complete build of nginx recipe and package.~~ [DONE — nginx 1.28.3, added pcre2 10.47 dependency]
6. Add stormwall package. [EXTERNAL HOLD]
7. Add rush package. [EXTERNAL HOLD]
8. ~~Finally fix the alignment of the fastfetch ASCII art and in the README.~~ [DONE — synced tagline to 9-char equals]
9. ~~Build hostapd recipe and jpkg.~~ [DONE — recipe exists, needs nl80211 headers + GNU make to build]
10. ~~Build wpa_supplicant recipe and jpkg.~~ [DONE — recipe exists, same blockers as hostapd]
11. ~~Build pico recipe and jpkg.~~ [REMOVED — mistake]
12. ~~Create BUILDER, CORE, ROUTER container images.~~ [DONE — Dockerfile.builder, Dockerfile.core, Dockerfile.router; CI pipeline builds minimal->core->{builder,router} with smoke tests]
13. ~~Multi-user mode.~~ [DONE — getty, inittab, system accounts, SUID bits, securetty]
14. Low-level file system utilities and Grub or some other bootloader for raw metal installs. This should require BYOK Bring/Build Your Own Kernel
      Since we do not distribute the kernel! Curl links to reliable prebuilt kernels can be selected, or the user can build their own from the          setup.
16. Java bootstrap?
17. ~~Clean up repository for isolated files and code that don't do anything anymore.~~ [DONE — partial, legacy Makefiles removed]
18. Build and customize a linux kernel in jonerix.
19. Raspberry Pi pre-reqs.
20. Build Ruby recipe and jpkg. [~~BLOCKED — needs GNU make at build time; jpkg license fixed for "BSD-2-Clause AND Ruby"~~Install jmake!]
21. ~~Purge gmake from repo.~~ [DONE — gmake recipe deleted; runc/containerd/toybox rewritten to avoid GNU make; hostapd/wpa_supplicant/ruby documented as needing GNU make at Alpine build time]
22. ~~Add local build script (build-local.sh).~~ [DONE — mirrors CI chain: minimal->core->builder->router + packages target]
23. ~~Add build-from-source.sh script.~~ [DONE — runs inside builder to rebuild all recipes]
24. ~~Fix jpkg license allowlist.~~ [DONE — added Ruby, MPL-2.0]
25. ~~Fix ci-build-x86_64.sh bsdtar fallback bug.~~ [DONE]

## Unpublished packages (have recipe, no .jpkg)

| Package | Blocker |
|---------|---------|
| ca-certificates | Was blocked by MPL-2.0 license — now fixed, needs build |
| hostapd | Needs nl80211 kernel headers + GNU make (Alpine build) |
| wpa_supplicant | Needs nl80211 kernel headers + GNU make (Alpine build) |
| ruby | Needs jmake ~~GNU make (Alpine build)~~; license now unblocked |
| linux | Kernel recipe — large project, separate effort |
~~| m4 | GPL build tool, pre_bootstrap=true |~~

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
- Build ruby jpkg (both arches) — license unblocked, needs GNU make in Alpine container
- Build x86_64 packages: btop, libatomic, npm, sqlite, unzip
- Build hostapd + wpa_supplicant jpkg (needs nl80211 headers packaged first)
- Linux kernel recipe + build
- Bootloader (grub alternative?) for bare metal
- Raspberry Pi support (device tree, kernel config)
- Java bootstrap chain
- Create build-from-source.yml CI workflow (worker instructions at .claude/worker-instructions-build-from-source-ci.md)
