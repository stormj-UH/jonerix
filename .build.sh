set -e
cd /workspace
for p in core/jpkg core/brash core/toybox core/openrc; do
  n=$(basename $p)
  printf '\n===== building %s =====\n' "$n"
  JPKG_SOURCE_CACHE=/workspace/sources jpkg build /workspace/packages/$p --output /workspace/out 2>&1 | tail -6
done
ls -la /workspace/out
