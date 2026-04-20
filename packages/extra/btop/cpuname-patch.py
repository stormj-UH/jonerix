#!/usr/bin/env python3
"""
Patch btop's get_cpuName() to look up the ARM CPU name via
/sys/bus/event_source/devices/ before falling back to /sys/devices/.

Upstream scans /sys/devices/ one directory deep for an "arm*" entry.
That works on pre-6.x kernels (e.g. Pi 4 / Debian Bookworm 6.1) but
not on 6.x aarch64 where the PMU node moved from /sys/devices/armv8_*
to /sys/devices/platform/arm-pmu/armv8_*. The event-source bus still
carries the same "armv8_cortex_aNN" name as a symlink on both layouts,
so prefer it.

This is applied as a pre-configure source edit instead of a patch(1)
hunk because the CI builder ships toybox 0.8.11's patch, which rejects
the otherwise-clean diff this change would produce. A direct string
substitution sidesteps patch context matching entirely.
"""

from __future__ import annotations

import pathlib
import sys

SRC = pathlib.Path("src/linux/btop_collect.cpp")

OLD = (
    "\t\t\telse if (fs::exists(\"/sys/devices\")) {\n"
    "\t\t\t\tfor (const auto& d : fs::directory_iterator(\"/sys/devices\")) {\n"
    "\t\t\t\t\tif (string(d.path().filename()).starts_with(\"arm\")) {\n"
    "\t\t\t\t\t\tname = d.path().filename();\n"
    "\t\t\t\t\t\tbreak;\n"
    "\t\t\t\t\t}\n"
    "\t\t\t\t}\n"
    "\t\t\t\tif (not name.empty()) {\n"
    "\t\t\t\t\tauto name_vec = ssplit(name, '_');\n"
    "\t\t\t\t\tif (name_vec.size() < 2) return capitalize(name);\n"
    "\t\t\t\t\telse return capitalize(name_vec.at(1)) + "
    "(name_vec.size() > 2 ? ' ' + capitalize(name_vec.at(2)) : \"\");\n"
    "\t\t\t\t}\n"
    "\n"
    "\t\t\t}\n"
)

NEW = (
    "\t\t\telse {\n"
    "\t\t\t\t// Prefer /sys/bus/event_source/devices/ -- stable across\n"
    "\t\t\t\t// kernel versions. 6.x moved the PMU node from\n"
    "\t\t\t\t// /sys/devices/armv8_cortex_aNN to\n"
    "\t\t\t\t// /sys/devices/platform/arm-pmu/armv8_cortex_aNN, which\n"
    "\t\t\t\t// the old shallow scan missed on modern aarch64. The\n"
    "\t\t\t\t// event-source bus still carries the same\n"
    "\t\t\t\t// \"armv8_cortex_aNN\" name on both layouts.\n"
    "\t\t\t\tconst char *pmu_dirs[] = {\n"
    "\t\t\t\t\t\"/sys/bus/event_source/devices\",\n"
    "\t\t\t\t\t\"/sys/devices\",\n"
    "\t\t\t\t};\n"
    "\t\t\t\tfor (const char *dir : pmu_dirs) {\n"
    "\t\t\t\t\tif (not fs::exists(dir)) continue;\n"
    "\t\t\t\t\tfor (const auto& d : fs::directory_iterator(dir)) {\n"
    "\t\t\t\t\t\tif (string(d.path().filename()).starts_with(\"arm\")) {\n"
    "\t\t\t\t\t\t\tname = d.path().filename();\n"
    "\t\t\t\t\t\t\tbreak;\n"
    "\t\t\t\t\t\t}\n"
    "\t\t\t\t\t}\n"
    "\t\t\t\t\tif (not name.empty()) break;\n"
    "\t\t\t\t}\n"
    "\t\t\t\tif (not name.empty()) {\n"
    "\t\t\t\t\tauto name_vec = ssplit(name, '_');\n"
    "\t\t\t\t\tif (name_vec.size() < 2) return capitalize(name);\n"
    "\t\t\t\t\telse return capitalize(name_vec.at(1)) + "
    "(name_vec.size() > 2 ? ' ' + capitalize(name_vec.at(2)) : \"\");\n"
    "\t\t\t\t}\n"
    "\t\t\t}\n"
)


def main() -> int:
    if not SRC.exists():
        print(f"cpuname-patch: {SRC} not found (wrong cwd?)", file=sys.stderr)
        return 1
    text = SRC.read_text()
    if NEW in text:
        print("cpuname-patch: already applied, skipping")
        return 0
    if OLD not in text:
        print(
            "cpuname-patch: upstream get_cpuName() block not found -- "
            "btop may have been updated, review the patch",
            file=sys.stderr,
        )
        return 1
    SRC.write_text(text.replace(OLD, NEW, 1))
    print("cpuname-patch: applied to", SRC)
    return 0


if __name__ == "__main__":
    sys.exit(main())
