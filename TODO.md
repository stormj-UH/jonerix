TODO:

1. ~~Complete fix on CI pipeline build of jonerix:all from source.~~ [DONE — disabled, OOM on 4GB runners]
2. ~~bmake extensions cleanup~~ [DONE — removed 27 legacy Makefiles, rules.mk, orphaned patch]
3. ~~Build Python 3.14 recipe and jpkg. Replace 3.12 packages.~~ [DONE — recipe updated to 3.14.3]
4. ~~Build sqlite and unzip recipes and jpkg.~~ [DONE — sqlite 3.51.3, unzip via libarchive bsdunzip]
5. ~~Complete build of nginx recipe and package.~~ [DONE — nginx 1.28.3, added pcre2 10.47 dependency]
6. Add stormwall package. [EXTERNAL HOLD]
7. Add rush package. [EXTERNAL HOLD]
8. ~~Finally fix the alignment of the fastfetch ASCII art and in the README.~~ [DONE — synced tagline to 9-char equals]
9. ~~Build hostapd recipe and jpkg.~~ [DONE — hostapd 2.11, BSD-3-Clause]
10. ~~Build wpa_supplicant recipe and jpkg.~~ [DONE — wpa_supplicant 2.11, BSD-3-Clause]
11. Build pico recipe and jpkg. [BLOCKED — GNU pico/nano are GPL; micro (MIT) already exists; needs clarification]
12. Create a BUILDER container image with gix, all the compilers, Python, Perl and development tools; a CORE container image that is like the "all" now but where we stop adding in the things coming now that are non-essential for all roles, minus compilers. Another container for ROUTER or IOT that includes a suite of packages for that role, but doesn't include the massive compilers and runtimes.
13. Multi-user mode.
14. Low-level file system utilities and Grub or some other bootloader for raw metal installs.
15. Java bootstrap?
16. ~~Clean up repository for isolated files and code that don't do anything anymore.~~ [DONE — partial, legacy Makefiles removed]
17. Build and customize a linux kernel in jonerix.
18. Raspberry Pi pre-reqs.
19. Build Ruby recipe and jpkg.
