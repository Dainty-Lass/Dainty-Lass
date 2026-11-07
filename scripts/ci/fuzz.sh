#!/usr/bin/env bash
# CI's short fuzz run (RD10, RX3), for both forges: every target of a cargo-fuzz workspace for 45 seconds,
# as many at once as the runner has cores. Run from the fuzz workspace's folder with NIGHTLY set.
# SHARDS and SHARD (0-based) split the targets between jobs; a target's log is printed only when it fails.
set -euo pipefail

: "${NIGHTLY:?the nightly toolchain to fuzz with}"
shards="${SHARDS:-1}"
shard="${SHARD:-0}"
jobs="${FUZZ_JOBS:-$(nproc)}"

# One build of every target, so the parallel runs below only check it is fresh
cargo "+$NIGHTLY" fuzz build

mapfile -t targets < <(cargo "+$NIGHTLY" fuzz list | awk -v n="$shards" -v i="$shard" '(NR - 1) % n == i')
echo "${#targets[@]} targets in shard $shard of $shards, $jobs at a time"
mkdir -p logs

run() {
  local target="$1"
  mkdir -p "corpus/$target"
  if cargo "+$NIGHTLY" fuzz run "$target" "corpus/$target" -- -max_total_time=45 -max_len=4096 \
    > "logs/$target.log" 2>&1; then
    echo "ok: $target"
  else
    echo "::error::$target failed"
    echo "::group::$target log"
    tail -n 200 "logs/$target.log"
    echo "::endgroup::"
    return 1
  fi
}
export -f run
export NIGHTLY

# xargs runs every target and exits non-zero if any of them failed
# shellcheck disable=SC2016 # $1 is the inner shell's
printf '%s\n' "${targets[@]}" | xargs -r -P "$jobs" -I{} bash -c 'run "$1"' _ {}
