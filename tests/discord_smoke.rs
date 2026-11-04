//! Live Discord connect + startup smoke test.
//!
//! pipo ignores bot-authored inbound messages, so a bot token cannot originate
//! a message pipo would relay — a full bridge assertion would need a human/user
//! token (which for Discord violates its ToS). Instead we assert pipo's real,
//! observable startup side-effect: on connect it creates a webhook named
//! `PIPO <channel_id>` in each mapped channel. The test reads that channel's
//! webhooks via the Discord REST API and waits for it to appear.
//!
//! Self-skips unless `PIPO_IT_DISCORD_TOKEN`, `PIPO_IT_DISCORD_GUILD_ID` and
//! `PIPO_IT_DISCORD_CHANNEL_ID` are all set (the workflow maps repo secrets to
//! these), so local runs and fork PRs stay green.

mod common;

use common::{env_or_skip, spawn_pipo, unique_tmp, write_config};
use std::time::{Duration, Instant};

#[tokio::test]
async fn discord_connect_creates_webhook() {
    let vars = match env_or_skip(&[
        "PIPO_IT_DISCORD_TOKEN",
        "PIPO_IT_DISCORD_GUILD_ID",
        "PIPO_IT_DISCORD_CHANNEL_ID",
    ]) {
        Some(v) => v,
        None => return,
    };
    let (token, guild_id, channel_id) = (&vars[0], &vars[1], &vars[2]);

    // channel_mapping key must be a numeric channel-id string; guild_id is a number.
    let config_json = format!(
        r#"{{
  "buses": [{{ "id": "main" }}],
  "transports": [
    {{ "transport": "Discord", "token": "{token}", "guild_id": {guild_id},
       "channel_mapping": {{ "{channel_id}": "main" }} }}
  ]
}}"#
    );
    let config = write_config(&config_json);
    let db = unique_tmp("db.sqlite3");
    let mut pipo = spawn_pipo(&config, &db);

    let http = reqwest::Client::new();
    let url = format!("https://discord.com/api/v10/channels/{channel_id}/webhooks");
    let expected = format!("PIPO {channel_id}");

    let deadline = Instant::now() + Duration::from_secs(45);
    let mut found = false;
    while Instant::now() < deadline {
        if let Ok(resp) = http
            .get(&url)
            .header("Authorization", format!("Bot {token}"))
            .send()
            .await
        {
            if let Ok(body) = resp.text().await {
                if body.contains(&expected) {
                    found = true;
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    let running = pipo.is_running();
    assert!(
        found,
        "expected pipo to create the Discord webhook '{expected}' \
         (pipo still running: {running}); pipo stderr:\n{}",
        pipo.stderr()
    );
}
