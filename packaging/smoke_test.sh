#!/usr/bin/env bash
set -euo pipefail

EXIT_LAYOUT_MISMATCH=10
EXIT_NON_EXECUTABLE=11
EXIT_SUPERVISOR_BOOT_FAILURE=12
EXIT_READY_TIMEOUT=13

fail() {
  local exit_code="$1"
  local message="$2"
  echo "$message" >&2
  exit "$exit_code"
}

archive_path="${1:-dist/pipo-bootstrap.tar.gz}"
extract_root="${2:-}"

if [[ ! -f "$archive_path" ]]; then
  shopt -s nullglob
  candidates=(dist/pipo-*.tar.gz)
  shopt -u nullglob
  if [[ ${#candidates[@]} -eq 1 ]]; then
    archive_path="${candidates[0]}"
    echo "archive not found at requested path; using discovered artifact: $archive_path" >&2
  else
    echo "archive not found: $archive_path" >&2
    if [[ ${#candidates[@]} -gt 1 ]]; then
      echo "multiple candidate artifacts found in dist/:" >&2
      printf '  %s\n' "${candidates[@]}" >&2
      echo "pass the intended archive path explicitly" >&2
    fi
    fail "$EXIT_LAYOUT_MISMATCH" "archive not found: $archive_path"
  fi
fi

cleanup_dir=""
if [[ -z "$extract_root" ]]; then
  tmp_dir="$(mktemp -d)"
  cleanup_dir="$tmp_dir"
  extract_root="$tmp_dir/runtime"
fi

if [[ -n "$cleanup_dir" ]]; then
  trap 'rm -rf "$cleanup_dir"' EXIT
fi

rm -rf "$extract_root"
mkdir -p "$extract_root"

tar -xzf "$archive_path" -C "$extract_root"

required=(
  "bin/pipo_supervisor"
  "bin/pipo-transport"
  "etc/pipo/config.example.json"
  "etc/pipo/transports.example.json"
  "releases/manifest.json"
  "releases/SHA256SUMS"
  "README-runtime.md"
)

for path in "${required[@]}"; do
  if [[ ! -e "$extract_root/$path" ]]; then
    fail "$EXIT_LAYOUT_MISMATCH" "missing required artifact member: $path"
  fi
done

"$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/validate_release_metadata.sh" "$archive_path"

if [[ ! -x "$extract_root/bin/pipo_supervisor" ]]; then
  fail "$EXIT_NON_EXECUTABLE" "expected executable bit on bin/pipo_supervisor"
fi
if [[ ! -x "$extract_root/bin/pipo-transport" ]]; then
  fail "$EXIT_NON_EXECUTABLE" "expected executable bit on bin/pipo-transport"
fi

runtime_config="$extract_root/etc/pipo/config.smoke.json"
cat > "$runtime_config" <<JSON
{
  "require_all_ready": true
}
JSON

log_file="$extract_root/smoke.log"
ready_timeout_seconds="${SMOKE_READY_TIMEOUT_SECONDS:-15}"
if ! [[ "$ready_timeout_seconds" =~ ^[0-9]+$ ]] || [[ "$ready_timeout_seconds" -lt 1 ]]; then
  fail "$EXIT_READY_TIMEOUT" "invalid SMOKE_READY_TIMEOUT_SECONDS value: $ready_timeout_seconds"
fi

if [[ -n "${SMOKE_RUN_CMD:-}" ]]; then
  (
    cd "$extract_root"
    eval "$SMOKE_RUN_CMD"
  ) >"$log_file" 2>&1 &
else
  "$extract_root/bin/pipo_supervisor" --config "$runtime_config" >"$log_file" 2>&1 &
fi

supervisor_pid="$!"
ready_pattern='"type"[[:space:]]*:[[:space:]]*"ready"|(^|[^[:alnum:]_])ready([^[:alnum:]_]|$)'
ready_seen=0

for (( second=0; second<ready_timeout_seconds; second++ )); do
  if [[ -f "$log_file" ]] && grep -Eqi "$ready_pattern" "$log_file"; then
    ready_seen=1
    break
  fi

  if ! kill -0 "$supervisor_pid" 2>/dev/null; then
    wait "$supervisor_pid" || supervisor_exit_code=$?
    supervisor_exit_code="${supervisor_exit_code:-0}"
    fail "$EXIT_SUPERVISOR_BOOT_FAILURE" "smoke test failed: supervisor exited before ready signal (exit $supervisor_exit_code)"
  fi

  sleep 1
done

if [[ "$ready_seen" -ne 1 ]]; then
  if kill -0 "$supervisor_pid" 2>/dev/null; then
    kill "$supervisor_pid" 2>/dev/null || true
    wait "$supervisor_pid" 2>/dev/null || true
  fi
  echo "smoke test failed: no ready signal observed within ${ready_timeout_seconds}s" >&2
  [[ -f "$log_file" ]] && cat "$log_file" >&2
  exit "$EXIT_READY_TIMEOUT"
fi

if kill -0 "$supervisor_pid" 2>/dev/null; then
  kill "$supervisor_pid" 2>/dev/null || true
  wait "$supervisor_pid" 2>/dev/null || true
fi

echo "Smoke test passed for $archive_path"
