use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use deadpool_sqlite::Pool;
use irc::{
    client::prelude::{Client, Command, Config, Prefix},
    proto::{caps::Capability, command::CapSubCommand, message::Tag, Message as IrcMessage},
};
use lazy_static::lazy_static;
use regex::Regex;
use rusqlite::params;
use serde::Deserialize;
use tokio::sync::broadcast;
use tokio_stream::{wrappers::BroadcastStream, StreamMap};

use crate::{Attachment, Message, ThreadRef};
use anyhow::anyhow;

const TRANSPORT_NAME: &'static str = "IRC";
const DEFAULT_THREAD_EXCERPT_LEN: usize = 120;

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThreadPresentationMode {
    #[default]
    Auto,
    Ircv3Only,
    PlaintextOnly,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThreadFallbackStyle {
    #[default]
    Compact,
    Verbose,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThreadContextRepeat {
    #[default]
    FirstSeen,
    Always,
    Never,
}

#[derive(Debug)]
struct ThreadPresentation {
    reply_target: Option<String>,
    plaintext_prefix: Option<String>,
    mode_used: &'static str,
}

pub(crate) struct IRC {
    transport_id: usize,
    config: Config,
    img_root: String,
    channels: HashMap<String, broadcast::Sender<Message>>,
    pool: Pool,
    pipo_id: Arc<Mutex<i64>>,
    capabilities: IrcCapabilityState,
    thread_presentation_mode: ThreadPresentationMode,
    thread_fallback_style: ThreadFallbackStyle,
    thread_context_repeat: ThreadContextRepeat,
    thread_excerpt_len: usize,
    show_thread_root_marker: bool,
    seen_thread_tokens: Arc<Mutex<HashMap<String, HashSet<String>>>>,
}

#[derive(Clone, Debug, Default)]
struct IrcCapabilityState {
    supports_message_tags: bool,
    supports_reply_tags: bool,
}

impl IRC {
    pub async fn new(
        bus_map: &HashMap<String, broadcast::Sender<Message>>,
        pipo_id: Arc<Mutex<i64>>,
        pool: Pool,
        nickname: String,
        server: String,
        use_tls: bool,
        img_root: &str,
        channel_mapping: &HashMap<Arc<String>, Arc<String>>,
        thread_presentation_mode: ThreadPresentationMode,
        thread_fallback_style: ThreadFallbackStyle,
        thread_context_repeat: ThreadContextRepeat,
        thread_excerpt_len: usize,
        show_thread_root_marker: bool,
        transport_id: usize,
    ) -> anyhow::Result<IRC> {
        let channels = channel_mapping
            .iter()
            .filter_map(|(channelname, busname)| {
                if let Some(sender) = bus_map.get(busname.as_ref()) {
                    Some((channelname.as_ref().clone(), sender.clone()))
                } else {
                    eprintln!("No bus named '{}' in configuration file.", busname);
                    None
                }
            })
            .collect();
        // Can this be done without an if/else?
        let config = if let Some((server_addr, server_port)) = server.rsplit_once(':') {
            Config {
                nickname: Some(nickname.clone()).to_owned(),
                server: Some(server_addr.to_string()).to_owned(),
                port: Some(server_port.parse()?).to_owned(),
                use_tls: Some(use_tls).to_owned(),
                ..Config::default()
            }
        } else {
            Config {
                nickname: Some(nickname.clone()).to_owned(),
                server: Some(server.clone()).to_owned(),
                use_tls: Some(use_tls).to_owned(),
                ..Config::default()
            }
        };

        Ok(IRC {
            config,
            img_root: img_root.to_string(),
            channels,
            transport_id,
            pool,
            pipo_id,
            capabilities: IrcCapabilityState::default(),
            thread_presentation_mode,
            thread_fallback_style,
            thread_context_repeat,
            thread_excerpt_len: if thread_excerpt_len == 0 {
                DEFAULT_THREAD_EXCERPT_LEN
            } else {
                thread_excerpt_len
            },
            show_thread_root_marker,
            seen_thread_tokens: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn connect(&mut self) -> anyhow::Result<()> {
        loop {
            let (client, mut irc_stream, mut input_buses) = self.connect_irc().await?;

            loop {
                // stupid sexy infinite loop
                tokio::select! {
                Some((channel, message))
                    = tokio_stream::StreamExt::next(&mut input_buses) => {
                    let message = message.unwrap();
                    match message {
                        Message::Action {
                        sender,
                        pipo_id,
                        transport,
                        username,
                        avatar_url: _,
                        thread,
                        message,
                        attachments,
                        is_edit,
                        irc_flag,
                        } => {
                        if sender != self.transport_id {
                            self.handle_action_message(&client,
                                           &channel,
                                           pipo_id,
                                           transport,
                                           username,
                                           thread,
                                           message,
                                           attachments,
                                           is_edit,
                                           irc_flag).await;
                        }
                        },
                        Message::Bot {
                        sender,
                        pipo_id: _,
                        transport,
                        message,
                        attachments,
                        is_edit,
                        } => {
                        if sender != self.transport_id {
                            self.handle_bot_message(&client,
                                        &channel,
                                        transport,
                                        message,
                                        attachments,
                                        is_edit);
                        }
                        },
                        Message::Delete {
                        sender: _,
                        pipo_id: _,
                        transport: _,
                        } => {
                        continue
                        },
                        Message::Names {
                        sender,
                        transport: _,
                        username,
                        message,
                        } => {
                        if sender != self.transport_id {
                            self.handle_names_message(&client,
                                          &channel,
                                          username,
                                          message);
                        }
                        },
                        Message::Pin {
                        sender: _,
                        pipo_id: _,
                        remove: _,
                        } => {
                        continue
                        },
                        Message::Reaction {
                        sender: _,
                        pipo_id: _,
                        transport: _,
                        emoji: _,
                        remove: _,
                        username: _,
                        avatar_url: _,
                        thread: _,
                        } => {
                        continue
                        },
                        Message::Text {
                        sender,
                        pipo_id,
                        transport,
                        username,
                        avatar_url: _,
                        thread,
                        message,
                        attachments,
                        is_edit,
                        irc_flag,
                        } => {
                        if sender != self.transport_id {
                            self.handle_text_message(&client,
                                         &channel,
                                         pipo_id,
                                         transport,
                                         username,
                                         thread,
                                         message,
                                         attachments,
                                         is_edit,
                                         irc_flag).await;
                        }
                        },
                    }
                    }
                Some(message)
                    = tokio_stream::StreamExt::next(&mut irc_stream) => {
                    if let Err(e) = message {
                        eprintln!("IRC Error: {}", e);

                        break
                    }
                    let message = message.unwrap();
                    let nickname = match message.prefix {
                        Some(Prefix::Nickname(ref nickname, _, _)) => nickname.to_string(),
                        Some(Prefix::ServerName(ref servername)) => servername.to_string(),
                        None => "".to_string(),
                    };
                    self.update_capabilities_from_message(&message);

                    let irc_message_id = IRC::parse_message_id_tag(&message);

                    if let Command::PRIVMSG(channel, message)
                        = message.command {
                        if let Err(e) = self.handle_priv_msg(nickname,
                                             channel,
                                             message,
                                             irc_message_id)
                            .await {
                            eprintln!("Error handling PRIVMSG: {}",
                                  e);
                            }
                        }
                    else if let Command::NOTICE(channel, message)
                        = message.command {
                        if let Err(e) = self.handle_notice(nickname,
                                           channel,
                                           message,
                                           irc_message_id)
                            .await {
                            eprintln!("Error handling NOTICE: {}",
                                  e);
                            }
                        }
                    }
                   else => break
                }
            }
        }
    }

    async fn handle_action_message(
        &self,
        client: &Client,
        channel: &str,
        pipo_id: i64,
        transport: String,
        username: String,
        thread: Option<crate::ThreadRef>,
        message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        is_edit: bool,
        irc_flag: bool,
    ) {
        let irc_message_id = self.ensure_ircid_for_pipo_id(pipo_id).await;
        let mut message = message;

        if irc_flag && is_edit {
            message = None
        }
        if let Some(message) = message {
            let mut is_edit = is_edit;
            let thread_presentation = self
                .resolve_thread_presentation(channel, pipo_id, &thread)
                .await;

            if thread.is_some() {
                self.log_thread_presentation(channel, pipo_id, &thread_presentation);
            }

            if let Some(prefix) = thread_presentation.plaintext_prefix.as_ref() {
                let prefix_message = format!(
                    "\x01ACTION \x02* \x02{}!\x02{}\x02 {}\x01",
                    &transport[..1].to_uppercase(),
                    username,
                    prefix
                );

                if let Err(e) = self
                    .send_privmsg_with_tags(
                        client,
                        channel,
                        prefix_message.clone(),
                        thread_presentation.reply_target.as_deref(),
                        irc_message_id.as_deref(),
                    )
                    .await
                {
                    eprintln!(
                        "Failed to send message '{}' channel {}: {:#}",
                        prefix_message, channel, e
                    );
                }
            }

            for msg in message.split("\n") {
                if msg == "" {
                    continue;
                }

                let message = if is_edit {
                    is_edit = false;

                    format!(
                        "\x01ACTION \x02* \x02{}!\x02{}\x02 {}*\x01",
                        &transport[..1].to_uppercase(),
                        username,
                        msg
                    )
                } else {
                    format!(
                        "\x01ACTION \x02* \x02{}!\x02{}\x02 {}\x01",
                        &transport[..1].to_uppercase(),
                        username,
                        msg
                    )
                };

                if let Err(e) = self
                    .send_privmsg_with_tags(
                        client,
                        channel,
                        message.clone(),
                        thread_presentation.reply_target.as_deref(),
                        irc_message_id.as_deref(),
                    )
                    .await
                {
                    eprintln!(
                        "Failed to send message '{}' channel {}: {:#}",
                        message, channel, e
                    );
                };
            }
        }

        if let Some(attachments) = attachments {
            IRC::handle_attachments(client, channel, attachments);
        }
    }

    fn handle_bot_message(
        &self,
        client: &Client,
        channel: &str,
        _transport: String,
        _message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        is_edit: bool,
    ) {
        if !is_edit {
            return;
        }

        if let Some(attachment) = attachments {
            IRC::handle_attachments(client, channel, attachment);
        }
    }

    fn handle_names_message(
        &self,
        client: &Client,
        channel: &str,
        username: String,
        message: Option<String>,
    ) {
        if message == Some("/names".to_string()) {
            if let Some(users) = client.list_users(channel) {
                let users: Vec<String> = users
                    .into_iter()
                    .map(|user| user.get_nickname().to_string())
                    .collect();
                let message = Message::Names {
                    sender: self.transport_id,
                    transport: TRANSPORT_NAME.to_string(),
                    username: username.to_string(),
                    message: Some(serde_json::json!(users).to_string()),
                };

                if let Some(sender) = self.channels.get(channel) {
                    eprintln!("Sending message: {:#}", message);
                    if let Err(e) = sender.send(message) {
                        eprintln!("Couldn't send message: {:#}", e);
                    }
                }
            }
        }
    }

    async fn handle_text_message(
        &self,
        client: &Client,
        channel: &str,
        pipo_id: i64,
        transport: String,
        username: String,
        thread: Option<crate::ThreadRef>,
        message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        is_edit: bool,
        irc_flag: bool,
    ) {
        let irc_message_id = self.ensure_ircid_for_pipo_id(pipo_id).await;
        let mut message = message;

        if irc_flag && is_edit {
            message = None
        }
        if let Some(message) = message {
            let mut is_edit = is_edit;
            let thread_presentation = self
                .resolve_thread_presentation(channel, pipo_id, &thread)
                .await;

            if thread.is_some() {
                self.log_thread_presentation(channel, pipo_id, &thread_presentation);
            }

            if let Some(prefix) = thread_presentation.plaintext_prefix.as_ref() {
                let prefix_message = format!(
                    "\x01ACTION <{}!\x02{}\x02> {}\x01",
                    &transport[..1].to_uppercase(),
                    username,
                    prefix
                );

                if let Err(e) = self
                    .send_privmsg_with_tags(
                        client,
                        channel,
                        prefix_message.clone(),
                        thread_presentation.reply_target.as_deref(),
                        irc_message_id.as_deref(),
                    )
                    .await
                {
                    eprintln!(
                        "Failed to send message '{}' channel {}: {:#}",
                        prefix_message, channel, e
                    );
                }
            }

            for msg in message.split("\n") {
                if msg == "" {
                    continue;
                }

                let message = if is_edit {
                    is_edit = false;

                    format!(
                        "\x01ACTION <{}!\x02{}\x02> \x02EDIT:\x02 {}\x01",
                        &transport[..1].to_uppercase(),
                        username,
                        msg
                    )
                } else {
                    format!(
                        "\x01ACTION <{}!\x02{}\x02> {}\x01",
                        &transport[..1].to_uppercase(),
                        username,
                        msg
                    )
                };

                if let Err(e) = self
                    .send_privmsg_with_tags(
                        client,
                        channel,
                        message.clone(),
                        thread_presentation.reply_target.as_deref(),
                        irc_message_id.as_deref(),
                    )
                    .await
                {
                    eprintln!(
                        "Failed to send message '{}' channel {}: {:#}",
                        message, channel, e
                    );
                }
            }
        }

        if let Some(attachment) = attachments {
            IRC::handle_attachments(client, channel, attachment);
        }
    }

    fn handle_attachments(client: &Client, channel: &str, attachments: Vec<Attachment>) {
        for attachment in attachments {
            let has_text = attachment.text.is_some();
            let has_fallback = attachment.fallback.is_some();
            let service_name = match attachment.service_name {
                Some(s) => s,
                None => String::from("Unknown"),
            };
            let author_name = match attachment.author_name {
                Some(s) => s,
                None => String::new(),
            };
            let text = match attachment.text {
                Some(s) => s,
                None => match attachment.fallback {
                    Some(s) => s,
                    None => continue,
                },
            };

            if !has_text && !has_fallback {
                continue;
            }

            let mut line_counter = 0;

            for msg in text.split("\n") {
                line_counter += 1;

                if msg == "" {
                    continue;
                }

                let message = if author_name.is_empty() {
                    format!("\x01ACTION [\x02{}\x02] {}\x01", service_name, msg)
                } else {
                    format!(
                        "\x01ACTION [{}!\x02{}\x02] {}\x01",
                        &service_name[..1].to_uppercase(),
                        author_name,
                        msg
                    )
                };

                if let Err(e) = client.send_privmsg(channel.clone(), message.clone()) {
                    eprintln!(
                        "Failed to send message '{}' channel {}: {:#}",
                        message, channel, e
                    );
                }

                if line_counter > 6 {
                    break;
                }
            }
        }
    }

    async fn connect_irc(
        &mut self,
    ) -> anyhow::Result<(
        Client,
        irc::client::ClientStream,
        StreamMap<String, BroadcastStream<Message>>,
    )> {
        let mut client = Client::from_config(self.config.clone()).await?;

        self.capabilities = IrcCapabilityState::default();

        client.send_cap_req(&[
            Capability::MultiPrefix,
            Capability::Custom("message-tags"),
            Capability::Custom("draft/reply"),
            Capability::ServerTime,
            Capability::EchoMessage,
        ])?;
        client.identify()?;

        let irc_stream = client.stream()?;
        let mut input_buses = StreamMap::new();
        for (channel_name, channel) in self.channels.iter() {
            input_buses.insert(
                channel_name.clone(),
                BroadcastStream::new(channel.subscribe()),
            );
            if let Err(e) = client.send_join(channel_name) {
                eprintln!("Failed to join channel {}: {:#}", channel_name, e);
            }
        }

        Ok((client, irc_stream, input_buses))
    }

    fn update_capabilities_from_message(&mut self, message: &IrcMessage) {
        let Command::CAP(_, subcommand, _, Some(extensions)) = &message.command else {
            return;
        };

        if *subcommand != CapSubCommand::ACK {
            return;
        }

        for capability in extensions.split_whitespace() {
            match capability {
                "message-tags" => self.capabilities.supports_message_tags = true,
                "draft/reply" | "reply" => self.capabilities.supports_reply_tags = true,
                _ => continue,
            }
        }
    }

    async fn send_privmsg_with_tags(
        &self,
        client: &Client,
        channel: &str,
        message: String,
        reply_target: Option<&str>,
        irc_message_id: Option<&str>,
    ) -> irc::error::Result<()> {
        let tags = self.tags_for_outbound_message(reply_target, irc_message_id);

        if let Some(tags) = tags {
            return client.send(IrcMessage {
                tags: Some(tags),
                prefix: None,
                command: Command::PRIVMSG(channel.to_string(), message),
            });
        }

        client.send_privmsg(channel, message)
    }

    fn tags_for_outbound_message(
        &self,
        reply_target: Option<&str>,
        irc_message_id: Option<&str>,
    ) -> Option<Vec<Tag>> {
        if !self.capabilities.supports_message_tags {
            return None;
        }

        let mut tags = Vec::new();

        if let Some(irc_message_id) = irc_message_id {
            tags.push(Tag(
                "draft/msgid".to_string(),
                Some(irc_message_id.to_string()),
            ));
        }

        if !self.capabilities.supports_reply_tags {
            return if tags.is_empty() { None } else { Some(tags) };
        }

        if let Some(reply_target) = reply_target {
            tags.push(Tag(
                "+draft/reply".to_string(),
                Some(reply_target.to_string()),
            ));
        }

        if tags.is_empty() {
            None
        } else {
            Some(tags)
        }
    }

    async fn resolve_thread_presentation(
        &self,
        channel: &str,
        pipo_id: i64,
        thread: &Option<ThreadRef>,
    ) -> ThreadPresentation {
        if thread.is_none() {
            return ThreadPresentation {
                reply_target: None,
                plaintext_prefix: None,
                mode_used: "none",
            };
        }

        let can_use_reply_tags = self.capabilities.supports_message_tags
            && self.capabilities.supports_reply_tags
            && !matches!(
                self.thread_presentation_mode,
                ThreadPresentationMode::PlaintextOnly
            );
        let reply_target = if can_use_reply_tags {
            self.resolve_irc_reply_target(thread).await
        } else {
            None
        };

        match self.thread_presentation_mode {
            ThreadPresentationMode::Auto => {
                if reply_target.is_some() {
                    ThreadPresentation {
                        reply_target,
                        plaintext_prefix: None,
                        mode_used: "ircv3_tag",
                    }
                } else {
                    ThreadPresentation {
                        reply_target: None,
                        plaintext_prefix: self
                            .outbound_thread_fallback_prefix(channel, pipo_id, thread)
                            .await,
                        mode_used: "plaintext_fallback",
                    }
                }
            }
            ThreadPresentationMode::Ircv3Only => {
                if reply_target.is_some() {
                    ThreadPresentation {
                        reply_target,
                        plaintext_prefix: None,
                        mode_used: "ircv3_tag",
                    }
                } else {
                    ThreadPresentation {
                        reply_target: None,
                        plaintext_prefix: None,
                        mode_used: "ircv3_unavailable",
                    }
                }
            }
            ThreadPresentationMode::PlaintextOnly => ThreadPresentation {
                reply_target: None,
                plaintext_prefix: self
                    .outbound_thread_fallback_prefix(channel, pipo_id, thread)
                    .await,
                mode_used: "plaintext_fallback",
            },
        }
    }

    fn log_thread_presentation(
        &self,
        channel: &str,
        pipo_id: i64,
        thread_presentation: &ThreadPresentation,
    ) {
        eprintln!(
            "IRC threaded message routing: transport_id={} channel={} pipo_id={} mode={}",
            self.transport_id, channel, pipo_id, thread_presentation.mode_used
        );
    }

    async fn resolve_irc_reply_target(&self, thread: &Option<ThreadRef>) -> Option<String> {
        let thread_ref = thread.as_ref()?;

        if let Some(thread_root_id) = thread_ref.thread_root_id.clone() {
            if let Some(ircid) = self.select_ircid_by_slackid(thread_root_id.clone()).await {
                return Some(ircid);
            }
            if let Some(ircid) = self.select_ircid_by_ircid(thread_root_id.clone()).await {
                return Some(ircid);
            }
            if let Ok(id) = thread_root_id.parse::<i64>() {
                if let Some(ircid) = self.select_ircid_from_messages(id).await {
                    return Some(ircid);
                }
            }
        }

        if let Some(reply_target_id) = thread_ref.reply_target_id {
            if let Some(ircid) = self.select_ircid_by_discordid(reply_target_id).await {
                return Some(ircid);
            }
        }

        None
    }

    async fn outbound_thread_fallback_prefix(
        &self,
        channel: &str,
        pipo_id: i64,
        thread: &Option<ThreadRef>,
    ) -> Option<String> {
        let thread_ref = thread.as_ref()?;

        if self.is_thread_root_message(pipo_id, thread_ref).await {
            return if self.show_thread_root_marker {
                Some("[thread]".to_string())
            } else {
                None
            };
        }

        let root_author = IRC::sanitize_thread_context_text(thread_ref.root_author.as_deref())
            .filter(|author| !author.is_empty())
            .unwrap_or_else(|| "unknown".to_string());
        let root_excerpt = IRC::sanitize_thread_context_text(thread_ref.root_excerpt.as_deref())
            .filter(|excerpt| !excerpt.is_empty())
            .map(|excerpt| IRC::truncate_with_ellipsis(excerpt, self.thread_excerpt_len))
            .unwrap_or_else(|| "…".to_string());
        let thread_token = IRC::thread_token(thread_ref);
        let compact_prefix = format!("↪ [t:{}] {}", thread_token, root_author);
        let expanded_prefix = format!("↪ [t:{}] {}: {}", thread_token, root_author, root_excerpt);

        let emit_expanded = match self.thread_context_repeat {
            ThreadContextRepeat::Always => true,
            ThreadContextRepeat::Never => false,
            ThreadContextRepeat::FirstSeen => self.mark_thread_token_seen(channel, &thread_token),
        };

        if emit_expanded {
            Some(expanded_prefix)
        } else if self.thread_fallback_style == ThreadFallbackStyle::Verbose {
            Some(expanded_prefix)
        } else {
            Some(compact_prefix)
        }
    }

    fn mark_thread_token_seen(&self, channel: &str, token: &str) -> bool {
        let mut seen = self.seen_thread_tokens.lock().unwrap();
        let channel_seen = seen.entry(channel.to_string()).or_default();
        channel_seen.insert(token.to_string())
    }

    fn thread_token(thread_ref: &ThreadRef) -> String {
        let token_input = thread_ref
            .thread_root_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .or_else(|| thread_ref.reply_target_id.map(|id| id.to_string()))
            .or_else(|| IRC::sanitize_thread_context_text(thread_ref.root_author.as_deref()))
            .unwrap_or_else(|| "thread".to_string());

        let mut hash: u32 = 0x811c9dc5;
        for byte in token_input.as_bytes() {
            hash ^= u32::from(*byte);
            hash = hash.wrapping_mul(0x01000193);
        }

        let base36 = IRC::to_base36(hash.max(1));
        base36.chars().take(4).collect::<String>()
    }

    fn to_base36(mut value: u32) -> String {
        let alphabet = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let mut out = Vec::new();

        while value > 0 {
            out.push(alphabet[(value % 36) as usize] as char);
            value /= 36;
        }

        out.iter().rev().collect()
    }

    async fn is_thread_root_message(&self, pipo_id: i64, thread_ref: &ThreadRef) -> bool {
        let Some(thread_root_id) = thread_ref.thread_root_id.as_deref() else {
            return false;
        };

        if let Some(slackid) = self.select_slackid_from_messages(pipo_id).await {
            if thread_root_id == slackid {
                return true;
            }
        }

        false
    }

    fn sanitize_thread_context_text(value: Option<&str>) -> Option<String> {
        let value = value?;
        let collapsed = value
            .chars()
            .map(|ch| if ch.is_ascii_control() { ' ' } else { ch })
            .collect::<String>();

        let collapsed = collapsed
            .split_whitespace()
            .collect::<Vec<&str>>()
            .join(" ");

        if collapsed.is_empty() {
            None
        } else {
            Some(collapsed)
        }
    }

    fn truncate_with_ellipsis(input: String, max_len: usize) -> String {
        let char_count = input.chars().count();
        if char_count <= max_len {
            return input;
        }

        let truncated: String = input.chars().take(max_len.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }

    fn parse_message_id_tag(message: &IrcMessage) -> Option<String> {
        let tags = message.tags.as_ref()?;

        tags.iter().find_map(|Tag(key, value)| {
            if key == "msgid" || key == "+draft/msgid" {
                value.clone()
            } else {
                None
            }
        })
    }

    async fn get_avatar_url(&self, nickname: &str) -> String {
        let client = reqwest::Client::new();
        let url = format!("{}/{}.png", self.img_root, nickname);

        let response = match client.head(url).send().await {
            Ok(response) => response,
            Err(_) => return format!("{}/irc.png", self.img_root),
        };

        if let Some(etag) = response.headers().get(reqwest::header::ETAG) {
            if let Ok(etag) = etag.to_str() {
                return format!("{}/{}.png?{}", self.img_root, nickname, etag);
            }
        }

        return format!("{}/{}.png", self.img_root, nickname);
    }

    async fn handle_priv_msg(
        &self,
        nickname: String,
        channel: String,
        message: String,
        irc_message_id: Option<String>,
    ) -> anyhow::Result<()> {
        if let Some(sender) = self.channels.get(&channel) {
            lazy_static! {
                static ref RE: Regex = Regex::new("^\x01ACTION (.*)\x01\r?$").unwrap();
            }
            let pipo_id = self.insert_into_messages_table().await?;
            if let Some(irc_message_id) = irc_message_id {
                self.update_messages_ircid(pipo_id, Some(irc_message_id))
                    .await?;
            }

            let avatar_url = self.get_avatar_url(&nickname).await;

            eprintln!("IRC PIPO ID: {}", pipo_id);

            if let Some(message) = RE.captures(&message) {
                let message = message.get(1).unwrap().as_str();
                let message = Message::Action {
                    sender: self.transport_id,
                    pipo_id,
                    transport: TRANSPORT_NAME.to_string(),
                    username: nickname.clone(),
                    avatar_url: Some(avatar_url),
                    thread: None,
                    message: Some(message.to_string()),
                    attachments: None,
                    is_edit: false,
                    irc_flag: false,
                };
                return match sender.send(message) {
                    Ok(_) => Ok(()),
                    Err(e) => Err(anyhow!("Couldn't send message: {:#}", e)),
                };
            } else {
                let message = Message::Text {
                    sender: self.transport_id,
                    pipo_id,
                    transport: TRANSPORT_NAME.to_string(),
                    username: nickname.clone(),
                    avatar_url: Some(avatar_url),
                    thread: None,
                    message: Some(message.to_string()),
                    attachments: None,
                    is_edit: false,
                    irc_flag: false,
                };
                return match sender.send(message) {
                    Ok(_) => Ok(()),
                    Err(e) => Err(anyhow!("Couldn't send message: {:#}", e)),
                };
            }
        } else {
            return Err(anyhow!("Could not get sender for channel {}", channel));
        }
    }

    async fn insert_into_messages_table(&self) -> anyhow::Result<i64> {
        let conn = self.pool.get().await.unwrap();
        let pipo_id = *self.pipo_id.lock().unwrap();

        // TODO: ugly error handling needs fixing
        match conn
            .interact(move |conn| -> anyhow::Result<usize> {
                Ok(conn.execute(
                    "INSERT OR REPLACE INTO messages (id) 
                                 VALUES (?1)",
                    params![pipo_id],
                )?)
            })
            .await
        {
            Ok(res) => res,
            Err(_) => Err(anyhow!("Interact Error")),
        }?;

        let ret = pipo_id;
        let mut pipo_id = self.pipo_id.lock().unwrap();
        *pipo_id += 1;
        if *pipo_id > 40000 {
            *pipo_id = 0
        }

        Ok(ret)
    }

    fn generated_irc_message_id(pipo_id: i64) -> String {
        format!("pipo-{}", pipo_id)
    }

    async fn ensure_ircid_for_pipo_id(&self, pipo_id: i64) -> Option<String> {
        if let Some(ircid) = self.select_ircid_from_messages(pipo_id).await {
            return Some(ircid);
        }

        let generated = IRC::generated_irc_message_id(pipo_id);
        if self
            .update_messages_ircid(pipo_id, Some(generated.clone()))
            .await
            .is_ok()
        {
            Some(generated)
        } else {
            None
        }
    }

    async fn update_messages_ircid(
        &self,
        pipo_id: i64,
        irc_message_id: Option<String>,
    ) -> anyhow::Result<()> {
        let conn = self.pool.get().await.unwrap();

        conn.interact(move |conn| -> anyhow::Result<usize> {
            Ok(conn.execute(
                "UPDATE messages SET ircid = ?2 WHERE id = ?1",
                params![pipo_id, irc_message_id],
            )?)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("Interact Error")))?;

        Ok(())
    }

    async fn select_ircid_from_messages(&self, pipo_id: i64) -> Option<String> {
        let conn = self.pool.get().await.unwrap();

        conn.interact(move |conn| -> anyhow::Result<Option<String>> {
            Ok(conn.query_row(
                "SELECT ircid FROM messages WHERE id = ?1",
                params![pipo_id],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("Interact Error")))
        .ok()
        .flatten()
    }

    async fn select_ircid_by_slackid(&self, slackid: String) -> Option<String> {
        let conn = self.pool.get().await.unwrap();

        conn.interact(move |conn| -> anyhow::Result<Option<String>> {
            Ok(conn.query_row(
                "SELECT ircid FROM messages WHERE slackid = ?1",
                params![slackid],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("Interact Error")))
        .ok()
        .flatten()
    }

    async fn select_ircid_by_discordid(&self, discordid: u64) -> Option<String> {
        let conn = self.pool.get().await.unwrap();

        conn.interact(move |conn| -> anyhow::Result<Option<String>> {
            Ok(conn.query_row(
                "SELECT ircid FROM messages WHERE discordid = ?1",
                params![discordid],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("Interact Error")))
        .ok()
        .flatten()
    }

    async fn select_ircid_by_ircid(&self, ircid: String) -> Option<String> {
        let conn = self.pool.get().await.unwrap();

        conn.interact(move |conn| -> anyhow::Result<Option<String>> {
            Ok(conn.query_row(
                "SELECT ircid FROM messages WHERE ircid = ?1",
                params![ircid],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("Interact Error")))
        .ok()
        .flatten()
    }

    async fn select_slackid_from_messages(&self, pipo_id: i64) -> Option<String> {
        let conn = self.pool.get().await.unwrap();

        conn.interact(move |conn| -> anyhow::Result<Option<String>> {
            Ok(conn.query_row(
                "SELECT slackid FROM messages WHERE id = ?1",
                params![pipo_id],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("Interact Error")))
        .ok()
        .flatten()
    }

    async fn handle_notice(
        &self,
        nickname: String,
        channel: String,
        message: String,
        irc_message_id: Option<String>,
    ) -> anyhow::Result<()> {
        if let Some(sender) = self.channels.get(&channel) {
            lazy_static! {
                static ref RE: Regex = Regex::new("^\x01ACTION (.*)\x01\r?$").unwrap();
            }
            let pipo_id = self.insert_into_messages_table().await?;
            if let Some(irc_message_id) = irc_message_id {
                self.update_messages_ircid(pipo_id, Some(irc_message_id))
                    .await?;
            }

            let avatar_url = self.get_avatar_url(&nickname).await;

            if let Some(message) = RE.captures(&message) {
                let message = format!("```{}```", message.get(1).unwrap().as_str());
                let message = Message::Action {
                    sender: self.transport_id,
                    pipo_id,
                    transport: TRANSPORT_NAME.to_string(),
                    username: nickname.clone(),
                    avatar_url: Some(avatar_url),
                    thread: None,
                    message: Some(message.to_string()),
                    attachments: None,
                    is_edit: false,
                    irc_flag: false,
                };
                return match sender.send(message) {
                    Ok(_) => Ok(()),
                    Err(e) => Err(anyhow!("Couldn't send message: {:#}", e)),
                };
            } else {
                let message = Message::Text {
                    sender: self.transport_id,
                    pipo_id,
                    transport: TRANSPORT_NAME.to_string(),
                    username: nickname.clone(),
                    avatar_url: Some(avatar_url),
                    thread: None,
                    message: Some(format!("```{}```", message.to_string())),
                    attachments: None,
                    is_edit: false,
                    irc_flag: false,
                };
                return match sender.send(message) {
                    Ok(_) => Ok(()),
                    Err(e) => Err(anyhow!("Couldn't send message: {:#}", e)),
                };
            }
        } else {
            return Err(anyhow!("Could not get sender for channel {}", channel));
        }
    }
}

impl Default for ThreadPresentation {
    fn default() -> Self {
        Self {
            reply_target: None,
            plaintext_prefix: None,
            mode_used: "none",
        }
    }
}
