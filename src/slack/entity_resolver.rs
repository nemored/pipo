#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum EntityReference {
    User {
        id: String,
        label: Option<String>,
    },
    Channel {
        id: String,
        label: Option<String>,
    },
    Team {
        id: String,
        label: Option<String>,
    },
    Usergroup {
        id: String,
        label: Option<String>,
    },
    Broadcast {
        range: String,
        label: Option<String>,
    },
    Link {
        url: String,
        label: Option<String>,
    },
    Unknown {
        raw: String,
    },
}

pub(crate) fn parse_entity_reference(raw: &str) -> EntityReference {
    if let Some(rest) = raw.strip_prefix('@') {
        let (id, label) = split_id_and_label(rest);
        return EntityReference::User { id, label };
    }

    if let Some(rest) = raw.strip_prefix('#') {
        let (id, label) = split_id_and_label(rest);
        return EntityReference::Channel { id, label };
    }

    if let Some(rest) = raw.strip_prefix("!subteam^") {
        let (id, label) = split_id_and_label(rest);
        return EntityReference::Usergroup { id, label };
    }

    if let Some(rest) = raw.strip_prefix("!team^") {
        let (id, label) = split_id_and_label(rest);
        return EntityReference::Team { id, label };
    }

    if let Some(rest) = raw.strip_prefix('!') {
        let (range, label) = split_id_and_label(rest);
        return EntityReference::Broadcast { range, label };
    }

    let (url, label) = split_id_and_label(raw);
    if url.contains("://") || url.starts_with("mailto:") {
        return EntityReference::Link { url, label };
    }

    EntityReference::Unknown {
        raw: raw.to_string(),
    }
}

pub(crate) fn format_user(name: Option<&str>, user_id: &str) -> String {
    format!("@{}", name.unwrap_or(&unknown_placeholder("user", user_id)))
}

pub(crate) fn format_channel(name: Option<&str>, channel_id: &str) -> String {
    format!(
        "#{}",
        name.unwrap_or(&unknown_placeholder("channel", channel_id))
    )
}

pub(crate) fn format_team(name: Option<&str>, team_id: &str) -> String {
    format!("@{}", name.unwrap_or(&unknown_placeholder("team", team_id)))
}

pub(crate) fn format_usergroup(name: Option<&str>, usergroup_id: &str) -> String {
    format!(
        "@{}",
        name.unwrap_or(&unknown_placeholder("usergroup", usergroup_id))
    )
}

pub(crate) fn format_broadcast(range: &str) -> String {
    match range {
        "here" | "channel" | "everyone" => format!("@{range}"),
        _ => format!("@{}", unknown_placeholder("broadcast", range)),
    }
}

pub(crate) fn format_link(url: &str, label: Option<&str>) -> String {
    match label.map(str::trim).filter(|label| !label.is_empty()) {
        Some(label) if label != url => format!("{} ({})", label, url),
        _ => url.to_string(),
    }
}

pub(crate) fn unknown_placeholder(entity: &str, id: &str) -> String {
    format!("unknown-{entity}:{id}")
}

fn split_id_and_label(value: &str) -> (String, Option<String>) {
    match value.split_once('|') {
        Some((id, label)) => (id.to_string(), Some(label.to_string())),
        None => (value.to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_links_with_and_without_label() {
        assert_eq!(
            format_link("https://example.com", Some("Example")),
            "Example (https://example.com)"
        );
        assert_eq!(
            format_link("https://example.com", None),
            "https://example.com"
        );
    }

    #[test]
    fn parses_entity_types() {
        assert_eq!(
            parse_entity_reference("@U123|alice"),
            EntityReference::User {
                id: "U123".to_string(),
                label: Some("alice".to_string())
            }
        );
        assert_eq!(format_broadcast("here"), "@here");
        assert_eq!(format_broadcast("unknown"), "@unknown-broadcast:unknown");
    }
}
