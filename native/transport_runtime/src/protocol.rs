use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SUPPORTED_PROTOCOL_VERSION: &str = "v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameMeta {
    pub protocol_version: String,
    pub transport: String,
    pub instance_id: String,
    pub timestamp: String,
}

impl FrameMeta {
    pub fn new(protocol_version: &str, transport: &str, instance_id: &str) -> Self {
        Self {
            protocol_version: protocol_version.to_owned(),
            transport: transport.to_owned(),
            instance_id: instance_id.to_owned(),
            timestamp: utc_timestamp(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum InboundFrame {
    #[serde(rename = "shutdown")]
    Shutdown,
    #[serde(rename = "inject_message")]
    InjectMessage {
        bus: String,
        pipo_id: i64,
        origin_transport: String,
        payload: Value,
    },
    #[serde(rename = "health_check")]
    HealthCheck,
    #[serde(rename = "reload_config")]
    ReloadConfig,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum OutboundFrame {
    #[serde(rename = "ready")]
    Ready {
        #[serde(flatten)]
        meta: FrameMeta,
    },
    #[serde(rename = "message")]
    Message {
        #[serde(flatten)]
        meta: FrameMeta,
        bus: String,
        pipo_id: Option<i64>,
        payload: Value,
    },
    #[serde(rename = "health")]
    Health {
        #[serde(flatten)]
        meta: FrameMeta,
        status: &'static str,
    },
    #[serde(rename = "warn")]
    Warn {
        #[serde(flatten)]
        meta: FrameMeta,
        warn_type: &'static str,
        message: String,
    },
    #[serde(rename = "fatal")]
    Fatal {
        #[serde(flatten)]
        meta: FrameMeta,
        message: String,
    },
}

pub fn parse_ndjson_line(line: &str) -> Result<InboundFrame, serde_json::Error> {
    serde_json::from_str::<InboundFrame>(line)
}

pub fn encode_ndjson_frame(frame: &OutboundFrame) -> Result<String, serde_json::Error> {
    let mut out = serde_json::to_string(frame)?;
    out.push('\n');
    Ok(out)
}

pub fn utc_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndjson_encode_ends_with_newline() {
        let frame = OutboundFrame::Health {
            meta: FrameMeta::new(SUPPORTED_PROTOCOL_VERSION, "slack", "slack_0"),
            status: "ok",
        };
        let encoded = encode_ndjson_frame(&frame).expect("encode");
        assert!(encoded.ends_with('\n'));
    }

    #[test]
    fn utc_timestamp_uses_zulu_with_millis() {
        let ts = utc_timestamp();
        assert!(ts.ends_with('Z'));
        assert!(ts.contains('.'));
    }
}
