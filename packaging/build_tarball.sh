#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="${DIST_DIR:-$ROOT_DIR/dist}"
TARGET_TRIPLE="${TARGET_TRIPLE:-$(rustc -vV | awk '/^host:/ {print $2}')}"
APP_VERSION="${APP_VERSION:-$(awk -F '"' '/^version = / {print $2; exit}' "$ROOT_DIR/Cargo.toml")}"
PROTOCOL_VERSION="${PROTOCOL_VERSION:-$(awk -F '"' '/protocol_version\(\)/, /}/ {if ($0 ~ /"[^"]+"/) {print $2; exit}}' "$ROOT_DIR/native/transport_runtime/src/lib.rs")}"
BUILD_TIMESTAMP="${BUILD_TIMESTAMP:-$(date -u +"%Y-%m-%dT%H:%M:%SZ")}"
GIT_SHA="${GIT_SHA:-$(git -C "$ROOT_DIR" rev-parse HEAD)}"

PIPO_SUPERVISOR_BIN="${PIPO_SUPERVISOR_BIN:-$ROOT_DIR/bin/pipo_supervisor}"
PIPO_TRANSPORT_BIN="${PIPO_TRANSPORT_BIN:-$ROOT_DIR/bin/pipo-transport}"
RUNTIME_README="${RUNTIME_README:-$ROOT_DIR/README-runtime.md}"
CONFIG_EXAMPLE="${CONFIG_EXAMPLE:-$ROOT_DIR/etc/pipo/config.example.json}"
TRANSPORTS_EXAMPLE="${TRANSPORTS_EXAMPLE:-$ROOT_DIR/etc/pipo/transports.example.json}"

if [[ ! -f "$PIPO_SUPERVISOR_BIN" ]]; then
  echo "missing supervisor binary: $PIPO_SUPERVISOR_BIN" >&2
  exit 1
fi
if [[ ! -f "$PIPO_TRANSPORT_BIN" ]]; then
  echo "missing transport binary: $PIPO_TRANSPORT_BIN" >&2
  exit 1
fi

mkdir -p "$DIST_DIR"

artifact_name="pipo-${APP_VERSION}-${TARGET_TRIPLE}.tar.gz"
artifact_path="$DIST_DIR/$artifact_name"
stage_dir="$(mktemp -d)"
trap 'rm -rf "$stage_dir"' EXIT

mkdir -p "$stage_dir/bin" "$stage_dir/etc/pipo" "$stage_dir/releases"
cp "$PIPO_SUPERVISOR_BIN" "$stage_dir/bin/pipo_supervisor"
cp "$PIPO_TRANSPORT_BIN" "$stage_dir/bin/pipo-transport"
cp "$RUNTIME_README" "$stage_dir/README-runtime.md"
cp "$CONFIG_EXAMPLE" "$stage_dir/etc/pipo/config.example.json"
cp "$TRANSPORTS_EXAMPLE" "$stage_dir/etc/pipo/transports.example.json"
chmod +x "$stage_dir/bin/pipo_supervisor" "$stage_dir/bin/pipo-transport"

sup_sha="$(sha256sum "$stage_dir/bin/pipo_supervisor" | awk '{print $1}')"
transport_sha="$(sha256sum "$stage_dir/bin/pipo-transport" | awk '{print $1}')"

cat > "$stage_dir/releases/manifest.json" <<MANIFEST
{
  "app_version": "$APP_VERSION",
  "protocol_version": "$PROTOCOL_VERSION",
  "target_triple": "$TARGET_TRIPLE",
  "build_timestamp": "$BUILD_TIMESTAMP",
  "git_sha": "$GIT_SHA",
  "sha256": {
    "bin/pipo_supervisor": "$sup_sha",
    "bin/pipo-transport": "$transport_sha"
  }
}
MANIFEST

cat > "$stage_dir/releases/SHA256SUMS" <<SUMS
$sup_sha  bin/pipo_supervisor
$transport_sha  bin/pipo-transport
SUMS

(
  cd "$stage_dir"
  tar -czf "$artifact_path" \
    bin/pipo_supervisor \
    bin/pipo-transport \
    etc/pipo/config.example.json \
    etc/pipo/transports.example.json \
    releases/manifest.json \
    releases/SHA256SUMS \
    README-runtime.md
)

echo "$artifact_path"
