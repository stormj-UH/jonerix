TODO:

1. Complete fix on CI pipeline build of jonerix:all from source. Works on my machine, but not on CI.
2. Complete rebuild of amd64 packages.
3. Add stormwall package. [EXTERNAL HOLD]
4. Add rush package. [EXTERNAL HOLD]
5. Build Ruby recipe and jpkg.
6. Build hostapd recipe and jpkg.
7. Build wpa_supplicant recipe and jpkg.
8. Remove blockers on tar and shell issues.
9. Are the bmake extensions from the root directory of the git still implemented? What is missing?
10. Java bootstrap?
11. Build pico receipe and jpkg.
12. Build Python 3.14 recipe and jpkg.
13. Create a BUILDER container image, a CORE container image that is like the "all" now but where \
    we stop adding in the things coming now that are non-essential for all roles. Another container \
    for ROUTER or IOT that includes a suite of packages for that role, but doesn't include the massive \
    compilers and runtimes.
14. Multi-user mode.
15. Low-level file system utilties and Grub or some other bootloader for raw metal installs.
16. Complete build of nginx recipe and package.
17. Clean up repository for isolated files and code that dont do anything aanymore.
18. Finally fix the alignment of the fastfetch ASCII art.
19. Build and customize a linux kernel in jonerix.
20. Raspberry Pi pre-reqs.
