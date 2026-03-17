use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result};
use pipo::{
    objects::{Block, Event, EventPayload},
    slack::rich_text_renderer::{self, RenderOptions, RichTextResolver},
};
use rusqlite::{params, Connection};
use serenity::async_trait;

struct DebugResolver;

#[async_trait]
impl RichTextResolver for DebugResolver {
    async fn resolve_user_display_name(&mut self, user_id: &str) -> Result<String> {
        Ok(format!("user-{user_id}"))
    }

    fn resolve_channel_name(&self, channel_id: &str) -> Option<String> {
        Some(format!("chan-{channel_id}"))
    }
}

fn append_fragment(acc: &mut String, fragment: &str) {
    if fragment.is_empty() {
        return;
    }
    if !acc.is_empty() && !acc.ends_with('\n') && !fragment.starts_with('\n') {
        acc.push('\n');
    }
    acc.push_str(fragment);
}

fn parse_args() -> Result<(PathBuf, Option<PathBuf>)> {
    let mut args = env::args().skip(1);
    let db_path = args.next().context(
        "usage: cargo run --bin slack_rich_text_debug -- <db-path> [fixtures-output-dir]",
    )?;
    Ok((PathBuf::from(db_path), args.next().map(PathBuf::from)))
}

async fn render_payload(payload: &EventPayload) -> Result<Option<String>> {
    let EventPayload::EventCallback { event, .. } = payload else {
        return Ok(None);
    };

    let Event::Message(message) = event else {
        return Ok(None);
    };

    let Some(blocks) = message.blocks.as_ref() else {
        return Ok(None);
    };

    let mut resolver = DebugResolver;
    let mut out = String::new();

    for block in blocks {
        if let Block::RichText { elements, .. } = block {
            for element in elements {
                let fragment = rich_text_renderer::render(
                    &mut resolver,
                    element,
                    RenderOptions {
                        irc_formatting_enabled: true,
                    },
                )
                .await?;
                append_fragment(&mut out, &fragment);
            }
        }
    }

    if out.is_empty() {
        Ok(None)
    } else {
        Ok(Some(out))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let (db_path, fixtures_out_dir) = parse_args()?;
    let conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open sqlite db at {}", db_path.display()))?;

    let mut stmt = conn.prepare(
        "SELECT id, event_id, payload_json FROM slack_event_log ORDER BY id DESC LIMIT 200",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    for row in rows {
        let (id, event_id, payload_json) = row?;
        let payload: EventPayload = match serde_json::from_str(&payload_json) {
            Ok(payload) => payload,
            Err(err) => {
                eprintln!("row {id}: skipped (deserialize failed): {err}");
                continue;
            }
        };

        let Some(actual) = render_payload(&payload).await? else {
            continue;
        };

        let expected = event_id
            .as_ref()
            .and_then(|eid| {
                conn.query_row(
                    "SELECT rendered_text FROM slack_render_log WHERE event_id = ? ORDER BY id DESC LIMIT 1",
                    params![eid],
                    |r| r.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten()
            })
            .unwrap_or_else(|| "(no matching slack_render_log row)".to_string());

        println!(
            "=== row:{id} event:{} ===",
            event_id.as_deref().unwrap_or("(none)")
        );
        println!("expected:\n{expected}");
        println!("actual:\n{actual}");

        if let Some(dir) = fixtures_out_dir.as_ref() {
            fs::create_dir_all(dir)?;
            let stem = event_id.clone().unwrap_or_else(|| format!("row_{id}"));
            let payload_path = dir.join(format!("{stem}.payload.json"));
            let golden_path = dir.join(format!("{stem}.golden"));
            fs::write(payload_path, serde_json::to_string_pretty(&payload)?)?;
            fs::write(golden_path, format!("{actual}\n"))?;
        }
    }

    Ok(())
}
