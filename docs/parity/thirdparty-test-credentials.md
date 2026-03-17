# Third-party parity test credentials (Slack + Discord)

This document defines how parity tests consume credentials for **real Slack/Discord API scenarios** without storing secrets in the repository.

## Credential and scope policy

- Use **short-lived tokens** wherever the provider supports them.
- Create **dedicated test apps** in Slack and Discord with **least-privilege scopes only** for scenarios in `docs/parity/integration-matrix.md`.
- Do not reuse production bots/apps/workspaces/guilds for parity testing.

### Slack minimum guidance

- Use a dedicated Slack app for parity runs.
- Restrict OAuth scopes to only what the matrix requires (message/reaction/thread and read scopes needed for hydration).
- Prefer token rotation automation from a secret manager over manually copied long-lived tokens.

### Discord minimum guidance

- Use a dedicated Discord application/bot for parity runs.
- Grant only intents/permissions required by parity scenarios.
- Keep bot token lifecycle managed in secrets infrastructure; rotate routinely.

## Secret storage requirements

- Credentials must be stored only in:
  - GitHub Actions encrypted secrets (CI usage), and/or
  - External secret managers (Vault, 1Password Connect, AWS/GCP/Azure secret stores).
- Credentials must **never** be committed to:
  - `docs/parity/fixtures/*.json`,
  - checked-in `.env` files,
  - repository config fixtures or example config files with real values.

## Dedicated test tenant isolation

- Slack: use a dedicated parity workspace.
- Discord: use a dedicated parity guild.
- Maintain channel isolation conventions:
  - one channel per scenario family when practical (e.g. `parity-msg`, `parity-react`, `parity-thread`), or
  - one channel per CI run with cleanup automation.
- Prevent cross-talk with production channels and bots.

## Environment-based injection contract

Third-party runs must inject configuration through environment variables instead of checked-in fixture credentials.

Required variables:

- `CONFIG_PATH` - path to runtime config generated at execution time.
- `DB_PATH` - sqlite path generated per run.
- `SLACK_TEST_BOT_TOKEN`
- `SLACK_TEST_APP_TOKEN`
- `DISCORD_TEST_BOT_TOKEN`

Recommended optional metadata:

- `SLACK_TEST_WORKSPACE_ID`
- `SLACK_TEST_CHANNEL_ID`
- `DISCORD_TEST_GUILD_ID`
- `DISCORD_TEST_CHANNEL_ID`

## Rotation policy

- Rotate Slack and Discord credentials on a fixed cadence (for example, every 30 days) and immediately after any suspected leak.
- Rotation updates must happen in secret managers/CI secrets only.
- Any invalid/expired token should fail preflight before long-running integration steps.

## Preflight behavior

Use `scripts/parity-thirdparty-preflight.sh` before third-party scenarios:

- In CI (`CI=true`), preflight is **strict**:
  - fails fast if required secrets or `CONFIG_PATH` / `DB_PATH` are missing.
- In local runs (`CI` unset/false), preflight is **non-strict**:
  - prints why third-party scenarios are skipped,
  - exits successfully to allow local-only parity checks to continue.

## CI wiring

The CI workflow runs preflight with environment-based injection from GitHub Secrets:

- `SLACK_TEST_BOT_TOKEN` <- `${{ secrets.SLACK_TEST_BOT_TOKEN }}`
- `SLACK_TEST_APP_TOKEN` <- `${{ secrets.SLACK_TEST_APP_TOKEN }}`
- `DISCORD_TEST_BOT_TOKEN` <- `${{ secrets.DISCORD_TEST_BOT_TOKEN }}`
- `CONFIG_PATH` / `DB_PATH` generated in `$RUNNER_TEMP` per run

This ensures credentials are never read from repository fixtures and are validated before third-party execution starts.
