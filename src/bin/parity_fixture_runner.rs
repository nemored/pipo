use anyhow::{anyhow, Context};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{env, fs};

#[derive(Debug, Deserialize)]
struct FixtureSet {
    config_cases: Vec<ConfigCase>,
    operation_cases: Vec<OperationCase>,
}

#[derive(Debug, Deserialize)]
struct ConfigCase {
    name: String,
    json: String,
}

#[derive(Debug, Deserialize)]
struct OperationCase {
    name: String,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
struct Step {
    op: String,
    slack_id: Option<String>,
    discord_id: Option<u64>,
    pipo_id: Option<i64>,
    emoji: Option<String>,
    remove: Option<bool>,
    thread_slack_ts: Option<String>,
    thread_discord_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ParsedConfig {
    buses: Vec<ConfigBus>,
    transports: Vec<ConfigTransport>,
}

#[derive(Debug, Deserialize)]
struct ConfigBus {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "transport")]
enum ConfigTransport {
    IRC {
        nickname: String,
        server: String,
        use_tls: bool,
        img_root: String,
        channel_mapping: std::collections::HashMap<String, String>,
    },
    Discord {
        token: String,
        guild_id: u64,
        channel_mapping: std::collections::HashMap<String, String>,
    },
    Slack {
        token: String,
        bot_token: String,
        channel_mapping: std::collections::HashMap<String, String>,
    },
    Minecraft {
        username: String,
        buses: Vec<String>,
    },
    Mumble {
        server: String,
        password: Option<String>,
        nickname: String,
        client_cert: Option<String>,
        server_cert: Option<String>,
        comment: Option<String>,
        channel_mapping: std::collections::HashMap<String, String>,
        voice_channel_mapping: std::collections::HashMap<String, String>,
    },
    Rachni {
        server: String,
        api_key: String,
        interval: u64,
        buses: Vec<String>,
    },
}

#[derive(Debug, Serialize)]
struct HarnessSummary {
    config_results: Vec<ConfigResult>,
    operation_results: Vec<OperationResult>,
}

#[derive(Debug, Serialize)]
struct ConfigResult {
    name: String,
    ok: bool,
    detail: String,
}

#[derive(Debug, Serialize)]
struct OperationResult {
    name: String,
    outputs: Vec<Value>,
    db_rows: Vec<DBRow>,
}

#[derive(Debug, Serialize)]
struct DBRow {
    id: i64,
    slack_id: Option<String>,
    discord_id: Option<u64>,
}

fn main() -> anyhow::Result<()> {
    let fixture_path = env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("usage: parity_fixture_runner <fixture.json>"))?;
    let raw = fs::read_to_string(&fixture_path).context("read fixture file")?;
    let fixture: FixtureSet = serde_json::from_str(&raw).context("parse fixture JSON")?;

    let mut config_results = Vec::new();
    for c in fixture.config_cases {
        let parsed = serde_json::from_str::<ParsedConfig>(&c.json);
        match parsed {
            Ok(cfg) => config_results.push(ConfigResult {
                name: c.name,
                ok: true,
                detail: format!(
                    "parsed buses={} transports={}",
                    cfg.buses.len(),
                    cfg.transports.len()
                ),
            }),
            Err(err) => config_results.push(ConfigResult {
                name: c.name,
                ok: false,
                detail: err.to_string(),
            }),
        }
    }

    let mut operation_results = Vec::new();
    for c in fixture.operation_cases {
        let conn = Connection::open_in_memory().context("open sqlite in memory")?;
        bootstrap(&conn)?;
        let mut next_id: i64 = 1;
        let mut outputs: Vec<Value> = Vec::new();

        for step in c.steps {
            match step.op.as_str() {
                "insert_slack" => {
                    let id = alloc_and_insert_slack(
                        &conn,
                        &mut next_id,
                        need_str(&step.slack_id, "slack_id")?,
                    )?;
                    outputs.push(serde_json::json!({"op":"insert_slack","pipo_id":id}));
                }
                "lookup_id_by_slack" => {
                    let id = select_id_by_slack(&conn, need_str(&step.slack_id, "slack_id")?)?;
                    outputs.push(serde_json::json!({"op":"lookup_id_by_slack","pipo_id":id}));
                }
                "update_discord" => {
                    conn.execute(
                        "UPDATE messages SET discordid = ?2 WHERE id = ?1",
                        params![
                            need_i64(step.pipo_id, "pipo_id")?,
                            need_u64(step.discord_id, "discord_id")?
                        ],
                    )?;
                    outputs.push(serde_json::json!({"op":"update_discord","ok":true}));
                }
                "lookup_discord_by_slack" => {
                    let discord =
                        select_discord_by_slack(&conn, need_str(&step.slack_id, "slack_id")?)?;
                    outputs.push(
                        serde_json::json!({"op":"lookup_discord_by_slack","discord_id":discord}),
                    );
                }
                "insert_discord" => {
                    let id = alloc_and_insert_discord(
                        &conn,
                        &mut next_id,
                        need_u64(step.discord_id, "discord_id")?,
                    )?;
                    outputs.push(serde_json::json!({"op":"insert_discord","pipo_id":id}));
                }
                "lookup_id_by_discord" => {
                    let id = select_id_by_discord(&conn, need_u64(step.discord_id, "discord_id")?)?;
                    outputs.push(serde_json::json!({"op":"lookup_id_by_discord","pipo_id":id}));
                }
                "update_slack" => {
                    conn.execute(
                        "UPDATE messages SET slackid = ?2 WHERE id = ?1",
                        params![
                            need_i64(step.pipo_id, "pipo_id")?,
                            need_str(&step.slack_id, "slack_id")?
                        ],
                    )?;
                    outputs.push(serde_json::json!({"op":"update_slack","ok":true}));
                }
                "route_edit_slack" => {
                    let pipo = select_id_by_slack(&conn, need_str(&step.slack_id, "slack_id")?)?;
                    outputs.push(serde_json::json!({
                        "op":"route_edit_slack",
                        "kind":"Text",
                        "is_edit":true,
                        "pipo_id":pipo,
                        "thread":{"slack_thread_ts":step.thread_slack_ts}
                    }));
                }
                "route_delete_discord" => {
                    let pipo =
                        select_id_by_discord(&conn, need_u64(step.discord_id, "discord_id")?)?;
                    outputs.push(serde_json::json!({
                        "op":"route_delete_discord",
                        "kind":"Delete",
                        "pipo_id":pipo,
                        "thread":{"discord_thread":step.thread_discord_id}
                    }));
                }
                "route_reaction_slack" => {
                    let pipo = select_id_by_slack(&conn, need_str(&step.slack_id, "slack_id")?)?;
                    outputs.push(serde_json::json!({
                        "op":"route_reaction_slack",
                        "kind":"Reaction",
                        "pipo_id":pipo,
                        "emoji":need_str(&step.emoji, "emoji")?,
                        "remove":step.remove.unwrap_or(false),
                        "thread":{"slack_thread_ts":step.thread_slack_ts}
                    }));
                }
                "route_reaction_discord" => {
                    let pipo =
                        select_id_by_discord(&conn, need_u64(step.discord_id, "discord_id")?)?;
                    outputs.push(serde_json::json!({
                        "op":"route_reaction_discord",
                        "kind":"Reaction",
                        "pipo_id":pipo,
                        "emoji":need_str(&step.emoji, "emoji")?,
                        "remove":step.remove.unwrap_or(false),
                        "thread":{"discord_thread":step.thread_discord_id}
                    }));
                }
                _ => return Err(anyhow!("unsupported op: {}", step.op)),
            }
        }

        operation_results.push(OperationResult {
            name: c.name,
            outputs,
            db_rows: dump_rows(&conn)?,
        });
    }

    let summary = HarnessSummary {
        config_results,
        operation_results,
    };
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn bootstrap(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE messages (
            id INTEGER PRIMARY KEY,
            slackid TEXT,
            discordid INTEGER,
            modtime DEFAULT (strftime('%Y-%m-%d %H:%M:%S:%s', 'now', 'localtime'))
        );",
    )?;
    Ok(())
}

fn alloc(next_id: &mut i64) -> i64 {
    let id = *next_id;
    *next_id += 1;
    if *next_id > 40000 {
        *next_id = 0;
    }
    id
}

fn alloc_and_insert_slack(
    conn: &Connection,
    next_id: &mut i64,
    slack_id: &str,
) -> anyhow::Result<i64> {
    let id = alloc(next_id);
    conn.execute(
        "INSERT OR REPLACE INTO messages (id, slackid) VALUES (?1, ?2)",
        params![id, slack_id],
    )?;
    Ok(id)
}

fn alloc_and_insert_discord(
    conn: &Connection,
    next_id: &mut i64,
    discord_id: u64,
) -> anyhow::Result<i64> {
    let id = alloc(next_id);
    conn.execute(
        "INSERT OR REPLACE INTO messages (id, discordid) VALUES (?1, ?2)",
        params![id, discord_id],
    )?;
    Ok(id)
}

fn select_id_by_slack(conn: &Connection, slack_id: &str) -> anyhow::Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM messages WHERE slackid = ?1",
            params![slack_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?)
}

fn select_id_by_discord(conn: &Connection, discord_id: u64) -> anyhow::Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM messages WHERE discordid = ?1",
            params![discord_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?)
}

fn select_discord_by_slack(conn: &Connection, slack_id: &str) -> anyhow::Result<Option<u64>> {
    Ok(conn
        .query_row(
            "SELECT discordid FROM messages WHERE slackid = ?1",
            params![slack_id],
            |r| r.get::<_, u64>(0),
        )
        .optional()?)
}

fn dump_rows(conn: &Connection) -> anyhow::Result<Vec<DBRow>> {
    let mut stmt = conn.prepare("SELECT id, slackid, discordid FROM messages ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok(DBRow {
            id: r.get(0)?,
            slack_id: r.get(1)?,
            discord_id: r.get(2)?,
        })
    })?;
    let mut out = vec![];
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn need_str<'a>(v: &'a Option<String>, name: &str) -> anyhow::Result<&'a str> {
    v.as_deref().ok_or_else(|| anyhow!("missing {}", name))
}

fn need_u64(v: Option<u64>, name: &str) -> anyhow::Result<u64> {
    v.ok_or_else(|| anyhow!("missing {}", name))
}

fn need_i64(v: Option<i64>, name: &str) -> anyhow::Result<i64> {
    v.ok_or_else(|| anyhow!("missing {}", name))
}
