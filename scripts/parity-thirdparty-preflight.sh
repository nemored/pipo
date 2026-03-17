#!/usr/bin/env bash
set -euo pipefail

required_vars=(
  CONFIG_PATH
  DB_PATH
  SLACK_TEST_BOT_TOKEN
  SLACK_TEST_APP_TOKEN
  DISCORD_TEST_BOT_TOKEN
)

missing=()
for var_name in "${required_vars[@]}"; do
  if [[ -z "${!var_name:-}" ]]; then
    missing+=("$var_name")
  fi
done

ci_mode="${CI:-}"
strict=false
if [[ "$ci_mode" == "true" || "$ci_mode" == "1" ]]; then
  strict=true
fi

if [[ ${#missing[@]} -gt 0 ]]; then
  if [[ "$strict" == "true" ]]; then
    echo "[parity preflight] missing required environment variables: ${missing[*]}" >&2
    echo "[parity preflight] CI mode is strict; refusing to run third-party scenarios." >&2
    exit 1
  fi

  echo "[parity preflight] skipping third-party scenarios in local run; missing: ${missing[*]}" >&2
  exit 0
fi

config_dir="$(dirname "$CONFIG_PATH")"
db_dir="$(dirname "$DB_PATH")"
mkdir -p "$config_dir" "$db_dir"

touch "$CONFIG_PATH"

if [[ ! -w "$config_dir" ]]; then
  echo "[parity preflight] CONFIG_PATH directory is not writable: $config_dir" >&2
  exit 1
fi
if [[ ! -w "$db_dir" ]]; then
  echo "[parity preflight] DB_PATH directory is not writable: $db_dir" >&2
  exit 1
fi

echo "[parity preflight] ok: required env vars and writable paths are present."
