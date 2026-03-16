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
struct IntegrationFixtureSet {
    scenarios: Vec<IntegrationScenario>,
}

#[derive(Debug, Deserialize)]
struct IntegrationScenario {
    name: String,
    config_json: String,
    actions: Vec<IntegrationAction>,
}

#[derive(Debug, Deserialize)]
struct IntegrationAction {
    #[serde(default)]
    driver: String,
    #[serde(default)]
    method: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    body: Option<Value>,
    #[serde(default)]
    expected_status: i64,
    #[serde(flatten)]
    step: Step,
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
#[serde(deny_unknown_fields)]
struct ParsedConfig {
    buses: Vec<ConfigBus>,
    transports: Vec<ConfigTransport>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ConfigBus {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "transport", deny_unknown_fields)]
#[allow(dead_code)]
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

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct HarnessSummary {
    config_results: Vec<ConfigResult>,
    operation_results: Vec<OperationResult>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct ConfigResult {
    name: String,
    ok: bool,
    detail: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct OperationResult {
    name: String,
    outputs: Vec<Value>,
    db_rows: Vec<DBRow>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct DBRow {
    id: i64,
    slack_id: Option<String>,
    discord_id: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct IntegrationScenarioResult {
    name: String,
    config: ConfigResult,
    outputs: Vec<Value>,
    db_rows: Vec<DBRow>,
    action_results: Vec<ActionResult>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct ActionResult {
    index: usize,
    driver: String,
    op: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let mut args = env::args().skip(1);
    let mut mode = String::from("fixture");
    let first = args.next().ok_or_else(|| {
        anyhow!("usage: parity_fixture_runner [--mode fixture|integration] <fixture.json>")
    })?;
    let fixture_path = if first == "--mode" {
        mode = args
            .next()
            .ok_or_else(|| anyhow!("missing mode value after --mode"))?;
        args.next().ok_or_else(|| anyhow!("missing fixture path"))?
    } else {
        first
    };

    if mode == "integration" {
        let results = run_integration(&fixture_path)?;
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }

    let summary = run_fixture(&fixture_path)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn run_fixture(path: &str) -> anyhow::Result<HarnessSummary> {
    let raw = fs::read_to_string(path).context("read fixture file")?;
    let fixture: FixtureSet = serde_json::from_str(&raw).context("parse fixture JSON")?;

    let mut config_results = Vec::new();
    for c in fixture.config_cases {
        config_results.push(run_config_case(c.name, &c.json));
    }

    let mut operation_results = Vec::new();
    for c in fixture.operation_cases {
        let conn = Connection::open_in_memory().context("open sqlite in memory")?;
        bootstrap(&conn)?;
        let mut next_id: i64 = 1;
        let mut outputs: Vec<Value> = Vec::new();

        for step in c.steps {
            outputs.push(apply_step(&conn, &mut next_id, &step)?);
        }

        operation_results.push(OperationResult {
            name: c.name,
            outputs,
            db_rows: dump_rows(&conn)?,
        });
    }

    Ok(HarnessSummary {
        config_results,
        operation_results,
    })
}

fn run_integration(path: &str) -> anyhow::Result<Vec<IntegrationScenarioResult>> {
    let raw = fs::read_to_string(path).context("read fixture file")?;
    let fixture: IntegrationFixtureSet =
        serde_json::from_str(&raw).context("parse integration JSON")?;
    let mut out = Vec::new();

    for scenario in fixture.scenarios {
        let conn = Connection::open_in_memory().context("open sqlite in memory")?;
        bootstrap(&conn)?;
        let mut next_id: i64 = 1;
        let config = run_config_case(format!("{}-config", scenario.name), &scenario.config_json);
        let mut outputs = Vec::new();
        let mut action_results = Vec::new();

        for (index, action) in scenario.actions.iter().enumerate() {
            let driver = if action.driver.is_empty() {
                "local".to_string()
            } else {
                action.driver.clone()
            };
            let mut result = ActionResult {
                index,
                driver: driver.clone(),
                op: action.step.op.clone(),
                ok: false,
                output: None,
                error: None,
            };

            match driver.as_str() {
                "local" => match apply_step(&conn, &mut next_id, &action.step) {
                    Ok(v) => {
                        result.ok = true;
                        result.output = Some(v.clone());
                        outputs.push(v);
                    }
                    Err(err) => result.error = Some(err.to_string()),
                },
                "api" => match run_api_action(action) {
                    Ok(v) => {
                        result.ok = true;
                        result.output = Some(v.clone());
                        outputs.push(v);
                    }
                    Err(err) => result.error = Some(err.to_string()),
                },
                _ => result.error = Some(format!("unsupported driver: {}", driver)),
            }

            action_results.push(result);
        }

        out.push(IntegrationScenarioResult {
            name: scenario.name,
            config,
            outputs,
            db_rows: dump_rows(&conn)?,
            action_results,
        });
    }

    Ok(out)
}

fn run_api_action(action: &IntegrationAction) -> anyhow::Result<Value> {
    let method = if action.method.is_empty() {
        "POST"
    } else {
        action.method.as_str()
    };
    let method = reqwest::Method::from_bytes(method.as_bytes()).context("invalid method")?;
    let rt = tokio::runtime::Runtime::new().context("build tokio runtime")?;
    let (status, text) = rt.block_on(async {
        let client = reqwest::Client::new();
        let mut req = client.request(method, &action.url);
        if let Some(body) = &action.body {
            req = req
                .header("content-type", "application/json")
                .body(serde_json::to_vec(body).context("encode api body")?);
        }
        let resp = req.send().await.context("send api request")?;
        let status = resp.status().as_u16() as i64;
        let text = resp.text().await.unwrap_or_default();
        Ok::<(i64, String), anyhow::Error>((status, text))
    })?;
    if action.expected_status > 0 && status != action.expected_status {
        return Err(anyhow!(
            "unexpected status got={} want={}",
            status,
            action.expected_status
        ));
    }
    Ok(serde_json::json!({"op": action.step.op, "status": status, "body": text}))
}

fn run_config_case(name: String, json: &str) -> ConfigResult {
    match serde_json::from_str::<ParsedConfig>(json) {
        Ok(cfg) => ConfigResult {
            name,
            ok: true,
            detail: format!(
                "parsed buses={} transports={}",
                cfg.buses.len(),
                cfg.transports.len()
            ),
        },
        Err(err) => ConfigResult {
            name,
            ok: false,
            detail: normalize_config_error(&err.to_string()),
        },
    }
}

fn apply_step(conn: &Connection, next_id: &mut i64, step: &Step) -> anyhow::Result<Value> {
    match step.op.as_str() {
        "insert_slack" => {
            let id = alloc_and_insert_slack(conn, next_id, need_str(&step.slack_id, "slack_id")?)?;
            Ok(serde_json::json!({"op":"insert_slack","pipo_id":id}))
        }
        "lookup_id_by_slack" => {
            let id = select_id_by_slack(conn, need_str(&step.slack_id, "slack_id")?)?;
            Ok(serde_json::json!({"op":"lookup_id_by_slack","pipo_id":id}))
        }
        "update_discord" => {
            conn.execute(
                "UPDATE messages SET discordid = ?2 WHERE id = ?1",
                params![
                    need_i64(step.pipo_id, "pipo_id")?,
                    need_u64(step.discord_id, "discord_id")?
                ],
            )?;
            Ok(serde_json::json!({"op":"update_discord","ok":true}))
        }
        "lookup_discord_by_slack" => {
            let discord = select_discord_by_slack(conn, need_str(&step.slack_id, "slack_id")?)?;
            Ok(serde_json::json!({"op":"lookup_discord_by_slack","discord_id":discord}))
        }
        "insert_discord" => {
            let id =
                alloc_and_insert_discord(conn, next_id, need_u64(step.discord_id, "discord_id")?)?;
            Ok(serde_json::json!({"op":"insert_discord","pipo_id":id}))
        }
        "lookup_id_by_discord" => {
            let id = select_id_by_discord(conn, need_u64(step.discord_id, "discord_id")?)?;
            Ok(serde_json::json!({"op":"lookup_id_by_discord","pipo_id":id}))
        }
        "update_slack" => {
            conn.execute(
                "UPDATE messages SET slackid = ?2 WHERE id = ?1",
                params![
                    need_i64(step.pipo_id, "pipo_id")?,
                    need_str(&step.slack_id, "slack_id")?
                ],
            )?;
            Ok(serde_json::json!({"op":"update_slack","ok":true}))
        }
        "route_edit_slack" => {
            let pipo = select_id_by_slack(conn, need_str(&step.slack_id, "slack_id")?)?;
            Ok(
                serde_json::json!({"op":"route_edit_slack","kind":"Text","is_edit":true,"pipo_id":pipo,"thread":{"slack_thread_ts":step.thread_slack_ts}}),
            )
        }
        "route_delete_discord" => {
            let pipo = select_id_by_discord(conn, need_u64(step.discord_id, "discord_id")?)?;
            Ok(
                serde_json::json!({"op":"route_delete_discord","kind":"Delete","pipo_id":pipo,"thread":{"discord_thread":step.thread_discord_id}}),
            )
        }
        "route_reaction_slack" => {
            let pipo = select_id_by_slack(conn, need_str(&step.slack_id, "slack_id")?)?;
            Ok(
                serde_json::json!({"op":"route_reaction_slack","kind":"Reaction","pipo_id":pipo,"emoji":need_str(&step.emoji, "emoji")?,"remove":step.remove.unwrap_or(false),"thread":{"slack_thread_ts":step.thread_slack_ts}}),
            )
        }
        "route_reaction_discord" => {
            let pipo = select_id_by_discord(conn, need_u64(step.discord_id, "discord_id")?)?;
            Ok(
                serde_json::json!({"op":"route_reaction_discord","kind":"Reaction","pipo_id":pipo,"emoji":need_str(&step.emoji, "emoji")?,"remove":step.remove.unwrap_or(false),"thread":{"discord_thread":step.thread_discord_id}}),
            )
        }
        _ => Err(anyhow!("unsupported op: {}", step.op)),
    }
}

fn normalize_config_error(msg: &str) -> String {
    if msg.contains("unknown field") {
        return "config parse error: unknown field".to_string();
    }
    "config parse error".to_string()
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
