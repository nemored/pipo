//! Live IRC bridging test. Stands up nothing itself — the workflow provides a
//! bare ircd (a GitHub Actions service container) and sets `PIPO_IT_IRC_SERVER`
//! to its `host:port`. The test self-skips when that env var is absent, so a
//! plain `cargo test` (locally / on fork PRs) stays green.
//!
//! Topology: one pipo process with TWO IRC transports on a single bus. Two
//! transports are required because every outbound handler skips messages it
//! originated (`sender == self.transport_id`), so a single transport cannot
//! bridge its own channels. An independent `irc`-crate client posts into
//! transport A's channel and asserts pipo relays it into transport B's channel
//! as a CTCP ACTION of the form `<I!nick> message`.

mod common;

use common::{spawn_pipo, unique_tmp, write_config};

use futures::StreamExt;
use irc::client::prelude::{Client, Command, Config};
use std::time::Duration;
use tokio::time::{timeout, Instant};

/// Parse `host:port` (plaintext) from the `PIPO_IT_IRC_SERVER` value.
fn parse_server(server: &str) -> (String, u16) {
    match server.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse().unwrap_or(6667)),
        None => (server.to_string(), 6667),
    }
}

#[tokio::test]
async fn bridges_message_between_two_irc_channels() {
    let server = match std::env::var("PIPO_IT_IRC_SERVER") {
        Ok(s) if !s.is_empty() => s,
        _ => {
            eprintln!("SKIP: PIPO_IT_IRC_SERVER is not set; skipping IRC bridge test");
            return;
        }
    };

    // pipo: two IRC transports, one bus, plaintext, distinct nicks.
    let config_json = format!(
        r##"{{
  "buses": [{{ "id": "main" }}],
  "transports": [
    {{ "transport": "IRC", "nickname": "pipo-a", "server": "{server}", "use_tls": false,
       "img_root": "images", "channel_mapping": {{ "#pipo-a": "main" }} }},
    {{ "transport": "IRC", "nickname": "pipo-b", "server": "{server}", "use_tls": false,
       "img_root": "images", "channel_mapping": {{ "#pipo-b": "main" }} }}
  ]
}}"##
    );
    let config = write_config(&config_json);
    let db = unique_tmp("db.sqlite3");
    let mut pipo = spawn_pipo(&config, &db);

    // Independent observer/injector client that joins both channels.
    let (host, port) = parse_server(&server);
    let client_config = Config {
        nickname: Some("tester".to_string()),
        server: Some(host),
        port: Some(port),
        use_tls: Some(false),
        channels: vec!["#pipo-a".to_string(), "#pipo-b".to_string()],
        ..Config::default()
    };
    let mut client = Client::from_config(client_config)
        .await
        .expect("failed to build test IRC client");
    client.identify().expect("failed to identify test IRC client");
    let mut stream = client.stream().expect("failed to open test IRC stream");

    let marker = format!("bridge-marker-{}", std::process::id());

    // Resend the stimulus periodically to tolerate pipo's connect/join latency,
    // and watch #pipo-b for the relayed CTCP ACTION.
    let found = timeout(Duration::from_secs(60), async {
        let mut last_send = Instant::now() - Duration::from_secs(10);
        loop {
            if last_send.elapsed() >= Duration::from_secs(3) {
                let _ = client.send_privmsg("#pipo-a", &marker);
                last_send = Instant::now();
            }
            // Poll with a short timeout so the resend loop keeps ticking and the
            // client's outgoing queue gets flushed.
            match timeout(Duration::from_secs(1), stream.next()).await {
                Ok(Some(Ok(message))) => {
                    if let Command::PRIVMSG(target, content) = &message.command {
                        if target == "#pipo-b"
                            && content.contains(&marker)
                            && content.contains("tester")
                        {
                            return true;
                        }
                    }
                }
                Ok(Some(Err(_))) | Ok(None) => return false, // stream error/end
                Err(_) => {}                                  // 1s read timeout
            }
        }
    })
    .await;

    let running = pipo.is_running();
    assert!(
        matches!(found, Ok(true)),
        "expected the message posted to #pipo-a to be bridged to #pipo-b \
         (pipo still running: {running}); pipo stderr:\n{}",
        pipo.stderr()
    );
}
