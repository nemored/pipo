#!/usr/bin/env bash
set -euo pipefail

: "${SLACK_TEST_BOT_TOKEN:?SLACK_TEST_BOT_TOKEN is required}"
: "${DISCORD_TEST_BOT_TOKEN:?DISCORD_TEST_BOT_TOKEN is required}"

slack_status=$(curl -sS -o /tmp/slack_auth_test.json -w '%{http_code}' \
  -H "Authorization: Bearer ${SLACK_TEST_BOT_TOKEN}" \
  'https://slack.com/api/auth.test')

if [[ "$slack_status" != "200" ]]; then
  echo "[live-api-check] Slack auth.test returned HTTP ${slack_status}" >&2
  cat /tmp/slack_auth_test.json >&2
  exit 1
fi

if ! jq -e '.ok == true' /tmp/slack_auth_test.json >/dev/null; then
  echo "[live-api-check] Slack auth.test returned ok=false" >&2
  cat /tmp/slack_auth_test.json >&2
  exit 1
fi

discord_status=$(curl -sS -o /tmp/discord_me.json -w '%{http_code}' \
  -H "Authorization: Bot ${DISCORD_TEST_BOT_TOKEN}" \
  'https://discord.com/api/v10/users/@me')

if [[ "$discord_status" != "200" ]]; then
  echo "[live-api-check] Discord users/@me returned HTTP ${discord_status}" >&2
  cat /tmp/discord_me.json >&2
  exit 1
fi

if ! jq -e '.id and .username' /tmp/discord_me.json >/dev/null; then
  echo "[live-api-check] Discord users/@me response missing expected fields" >&2
  cat /tmp/discord_me.json >&2
  exit 1
fi

echo "[live-api-check] Slack and Discord API credential checks passed."
