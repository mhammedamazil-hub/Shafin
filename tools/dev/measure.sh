#!/usr/bin/env bash
# Run a command and report wall-clock time and peak resident memory.
# Peak memory is the kernel's high-water mark (VmHWM in /proc/PID/status), sampled until the
# process exits, so it does not depend on catching the peak at the right moment.
# usage: tools/dev/measure.sh LABEL command [args...]
set -u
label="$1"; shift
start=$(date +%s.%N)
"$@" &
pid=$!
peak_kb=0
while kill -0 "$pid" 2>/dev/null; do
  hwm=$(awk '/^VmHWM:/ {print $2}' "/proc/$pid/status" 2>/dev/null || true)
  if [ -n "${hwm:-}" ] && [ "$hwm" -gt "$peak_kb" ]; then peak_kb=$hwm; fi
  sleep 0.2
done
wait "$pid"; rc=$?
end=$(date +%s.%N)
awk -v l="$label" -v s="$start" -v e="$end" -v k="$peak_kb" -v rc="$rc" 'BEGIN { printf "MEASURE %s: exit %d, wall %.1f s (%.1f min), peak RSS %.0f MB\n", l, rc, e - s, (e - s) / 60, k / 1024 }'
exit $rc
