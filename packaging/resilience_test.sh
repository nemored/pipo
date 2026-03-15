#!/usr/bin/env bash
set -euo pipefail

EXIT_NON_EXECUTABLE=11
EXIT_SUPERVISOR_BOOT_FAILURE=12
EXIT_READY_TIMEOUT=13

archive_path="${1:-}"
target_triple="${2:-unknown-target}"
if [[ -z "$archive_path" ]]; then
  echo "usage: $0 <archive_path> [target_triple]" >&2
  exit 2
fi
if [[ ! -f "$archive_path" ]]; then
  echo "archive not found: $archive_path" >&2
  exit 2
fi

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
smoke_script="$root_dir/packaging/smoke_test.sh"
work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

extract_and_repack() {
  local source_archive="$1"
  local output_archive="$2"
  local mutator="$3"
  local stage_dir="$work_dir/stage"

  rm -rf "$stage_dir"
  mkdir -p "$stage_dir"
  tar -xzf "$source_archive" -C "$stage_dir"
  "$mutator" "$stage_dir"
  (
    cd "$stage_dir"
    tar -czf "$output_archive" .
  )
}

expect_exit() {
  local expected="$1"
  shift
  set +e
  "$@"
  local actual=$?
  set -e
  if [[ "$actual" -ne "$expected" ]]; then
    echo "resilience check failed for $target_triple: expected exit $expected but got $actual" >&2
    return 1
  fi
  return 0
}

mutate_non_executable_transport() {
  local stage_dir="$1"
  chmod -x "$stage_dir/bin/pipo-transport"
}

non_exec_archive="$work_dir/non-executable.tar.gz"
extract_and_repack "$archive_path" "$non_exec_archive" mutate_non_executable_transport
expect_exit "$EXIT_NON_EXECUTABLE" "$smoke_script" "$non_exec_archive" "$work_dir/non-executable"

expect_exit "$EXIT_READY_TIMEOUT" env SMOKE_READY_TIMEOUT_SECONDS=0 "$smoke_script" "$archive_path" "$work_dir/invalid-timeout"

expect_exit "$EXIT_SUPERVISOR_BOOT_FAILURE" env SMOKE_RUN_CMD='bash -lc "echo simulated startup failure; exit 42"' "$smoke_script" "$archive_path" "$work_dir/boot-failure"

echo "Resilience checks passed for $target_triple"
