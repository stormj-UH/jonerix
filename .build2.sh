set -e
cd /workspace
jpkg update >/dev/null 2>&1
jpkg install jonerix-headers 2>&1 | tail -1
jpkg install onetrueawk 2>&1 | tail -1
command -v awk || echo "awk STILL MISSING"
JPKG_SOURCE_CACHE=/workspace/sources jpkg build /workspace/packages/core/toybox --output /workspace/out 2>&1 | tail -6
