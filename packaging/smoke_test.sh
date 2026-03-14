#!/usr/bin/env bash
set -euo pipefail

archive_path="${1:-}"
if [[ -z "$archive_path" ]]; then
  echo "usage: $0 <path-to-tarball>" >&2
  exit 2
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

extract_root="$tmp_dir/runtime"
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
  test -e "$extract_root/$path"
done

test -x "$extract_root/bin/pipo_supervisor"
test -x "$extract_root/bin/pipo-transport"

runtime_config="$tmp_dir/config.smoke.json"
cat > "$runtime_config" <<JSON
{
  "require_all_ready": true
}
JSON

log_file="$tmp_dir/supervisor.log"
if [[ -n "${SMOKE_RUN_CMD:-}" ]]; then
  (
    cd "$extract_root"
    eval "$SMOKE_RUN_CMD"
  ) >"$log_file" 2>&1
else
  "$extract_root/bin/pipo_supervisor" --config "$runtime_config" >"$log_file" 2>&1
fi

if ! rg -q '"type"\s*:\s*"ready"|\bready\b' "$log_file"; then
  echo "smoke test failed: no ready signal observed" >&2
  cat "$log_file" >&2
  exit 1
fi

echo "Smoke test passed for $archive_path"
