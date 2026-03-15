#!/usr/bin/env bash
set -euo pipefail

archive_path="${1:-dist/pipo-bootstrap.tar.gz}"

if [[ ! -f "$archive_path" ]]; then
  echo "archive not found: $archive_path" >&2
  exit 1
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

tar -xzf "$archive_path" -C "$tmp_dir"

for required in \
  "$tmp_dir/releases/manifest.json" \
  "$tmp_dir/releases/SHA256SUMS" \
  "$tmp_dir/bin/pipo_supervisor" \
  "$tmp_dir/bin/pipo-transport"; do
  if [[ ! -f "$required" ]]; then
    echo "missing required file in archive: ${required#"$tmp_dir/"}" >&2
    exit 1
  fi
done

python3 - "$tmp_dir" <<'PY'
import hashlib
import json
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
manifest_path = root / "releases" / "manifest.json"
checksums_path = root / "releases" / "SHA256SUMS"

try:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
except json.JSONDecodeError as exc:
    raise SystemExit(f"manifest.json is not valid JSON: {exc}")

required_string_fields = [
    "app_version",
    "protocol_version",
    "target_triple",
    "build_timestamp",
    "git_commit_sha",
]

for field in required_string_fields:
    value = manifest.get(field)
    if not isinstance(value, str) or not value.strip():
        raise SystemExit(f"manifest field {field!r} must be a non-empty string")

if not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", manifest["build_timestamp"]):
    raise SystemExit("manifest field 'build_timestamp' must be UTC RFC3339 (YYYY-MM-DDTHH:MM:SSZ)")

if not re.fullmatch(r"[0-9a-f]{40}", manifest["git_commit_sha"]):
    raise SystemExit("manifest field 'git_commit_sha' must be a 40-character lowercase hex SHA")

sha_block = manifest.get("sha256")
if not isinstance(sha_block, dict):
    raise SystemExit("manifest field 'sha256' must be an object")

expected_paths = ["bin/pipo_supervisor", "bin/pipo-transport"]
for path in expected_paths:
    digest = sha_block.get(path)
    if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise SystemExit(f"manifest sha256 entry for {path!r} must be a 64-character lowercase hex SHA256")

actual = {}
for path in expected_paths:
    file_path = root / path
    digest = hashlib.sha256(file_path.read_bytes()).hexdigest()
    actual[path] = digest
    if digest != sha_block[path]:
        raise SystemExit(f"manifest sha256 mismatch for {path}: expected {sha_block[path]}, got {digest}")

line_re = re.compile(r"^([0-9a-f]{64})\s{2}(.+)$")
lines = [line for line in checksums_path.read_text(encoding="utf-8").splitlines() if line.strip()]
if len(lines) != len(expected_paths):
    raise SystemExit("SHA256SUMS must include exactly two entries for runtime binaries")

seen = {}
for line in lines:
    match = line_re.fullmatch(line)
    if not match:
        raise SystemExit("SHA256SUMS lines must match '<sha256><two spaces><relative path>'")
    seen[match.group(2)] = match.group(1)

for path in expected_paths:
    digest = seen.get(path)
    if digest is None:
        raise SystemExit(f"SHA256SUMS missing entry for {path}")
    if digest != actual[path]:
        raise SystemExit(f"SHA256SUMS mismatch for {path}: expected {actual[path]}, got {digest}")

print("release metadata validation passed")
PY
