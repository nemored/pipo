#!/usr/bin/env bash
set -euo pipefail

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
    exit 1
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
    echo "missing required artifact member: $path" >&2
    exit 1
  fi
done

"$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/validate_release_metadata.sh" "$archive_path"

if [[ ! -x "$extract_root/bin/pipo_supervisor" ]]; then
  echo "expected executable bit on bin/pipo_supervisor" >&2
  exit 1
fi
if [[ ! -x "$extract_root/bin/pipo-transport" ]]; then
  echo "expected executable bit on bin/pipo-transport" >&2
  exit 1
fi

runtime_config="$extract_root/etc/pipo/config.smoke.json"
cat > "$runtime_config" <<JSON
{
  "require_all_ready": true
}
JSON

log_file="$extract_root/smoke.log"
if [[ -n "${SMOKE_RUN_CMD:-}" ]]; then
  (
    cd "$extract_root"
    eval "$SMOKE_RUN_CMD"
  ) >"$log_file" 2>&1
else
  "$extract_root/bin/pipo_supervisor" --config "$runtime_config" >"$log_file" 2>&1
fi

if ! grep -Eqi '"type"[[:space:]]*:[[:space:]]*"ready"|(^|[^[:alnum:]_])ready([^[:alnum:]_]|$)' "$log_file"; then
  echo "smoke test failed: no ready signal observed" >&2
  cat "$log_file" >&2
  exit 1
fi

echo "Smoke test passed for $archive_path"
