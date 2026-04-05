TODO:

1. Complete fix on CI pipeline build of jonerix:all from source. Works for aarch64 but not x64.
2. Are the bmake extensions from the root directory of the git still implemented? What is missing? Is that being used in the builds?
3. Build Python 3.14 recipe and jpkg. Replace 3.12 packages.
4. Complete build of nginx recipe and package.
5. Add stormwall package. [EXTERNAL HOLD]
6. Add rush package. [EXTERNAL HOLD]
7. Finally fix the alignment of the fastfetch ASCII art and in the README.
8. Build hostapd recipe and jpkg.
9. Build wpa_supplicant recipe and jpkg
10. Build pico receipe and jpkg.
11. Create a BUILDER container image with gix, all the compilers, Python, Perl and development tools; a CORE container image that is like the "all"     now but where we stop adding in the things coming now that are non-essential for all roles, minus compilers. Another container \
    for ROUTER or IOT that includes a suite of packages for that role, but doesn't include the massive \
    compilers and runtimes.
12. Multi-user mode.
13. Low-level file system utilties and Grub or some other bootloader for raw metal installs.
14. Java bootstrap?
15. Clean up repository for isolated files and code that dont do anything aanymore.
16. Build and customize a linux kernel in jonerix.
17. Raspberry Pi pre-reqs.
18. Build Ruby recipe and jpkg.
