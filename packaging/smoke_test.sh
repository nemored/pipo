#!/usr/bin/env bash
set -euo pipefail

archive_path="${1:-dist/pipo-bootstrap.tar.gz}"
extract_dir="${2:-dist/smoke}"

rm -rf "$extract_dir"
mkdir -p "$extract_dir"
tar -xzf "$archive_path" -C "$extract_dir"

for required in apps native schema packaging docs; do
  test -d "$extract_dir/$required"
done

echo "Smoke test passed for $archive_path"
