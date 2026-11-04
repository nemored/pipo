//! Live Slack connect smoke test.
//!
//! As with Discord, pipo drops bot-authored inbound messages, so a real bridge
//! assertion would need a user token. Instead we assert the connection path: on
//! valid tokens pipo completes `apps.connections.open` (app-level token) plus
//! `conversations.list` (bot token) and then stays in the Socket Mode read loop
//! indefinitely; with bad tokens/scopes the sole transport task errors out and
//! the process exits. So "still running after a warm-up window" is a meaningful
//! success signal.
//!
//! Self-skips unless `PIPO_IT_SLACK_APP_TOKEN`, `PIPO_IT_SLACK_BOT_TOKEN` and
//! `PIPO_IT_SLACK_CHANNEL` are all set.

mod common;

use common::{env_or_skip, spawn_pipo, unique_tmp, write_config};
use std::time::Duration;

#[tokio::test]
async fn slack_connect_stays_alive() {
    let vars = match env_or_skip(&[
        "PIPO_IT_SLACK_APP_TOKEN",
        "PIPO_IT_SLACK_BOT_TOKEN",
        "PIPO_IT_SLACK_CHANNEL",
    ]) {
        Some(v) => v,
        None => return,
    };
    let (app_token, bot_token, channel) = (&vars[0], &vars[1], &vars[2]);

    // channel_mapping key is the "#channel-name" form Slack uses internally.
    let config_json = format!(
        r#"{{
  "buses": [{{ "id": "main" }}],
  "transports": [
    {{ "transport": "Slack", "token": "{app_token}", "bot_token": "{bot_token}",
       "channel_mapping": {{ "{channel}": "main" }} }}
  ]
}}"#
    );
    let config = write_config(&config_json);
    let db = unique_tmp("db.sqlite3");
    let mut pipo = spawn_pipo(&config, &db);

    // Give pipo time to open the socket and enumerate channels.
    tokio::time::sleep(Duration::from_secs(20)).await;

    assert!(
        pipo.is_running(),
        "pipo exited during Slack connect (likely an auth/scope failure); \
         pipo stderr:\n{}",
        pipo.stderr()
    );
}
