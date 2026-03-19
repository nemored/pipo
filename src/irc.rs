use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::anyhow;
use deadpool_sqlite::Pool;
use irc::{
    client::prelude::{Client, Command, Config, Prefix},
    proto::{caps::Capability, command::CapSubCommand, message::Tag, Message as IrcMessage},
};
use lazy_static::lazy_static;
use regex::Regex;
use rusqlite::params;
use serde::Deserialize;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::{wrappers::BroadcastStream, StreamMap};

use crate::{
    load_irc_presence, upsert_irc_presence, upsert_remote_actor, Attachment, Message, RemoteActor,
    ThreadRef,
};

const TRANSPORT_NAME: &str = "IRC";
const DEFAULT_THREAD_EXCERPT_LEN: usize = 120;
const REPLY_TOKEN_TTL: Duration = Duration::from_secs(60 * 60 * 6);
const THREAD_LIST_LIMIT: usize = 8;
const ACTOR_SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(60 * 5);
const ACTOR_SESSION_MIN_LIFETIME: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
struct ReplyTokenEntry {
    thread_ref: ThreadRef,
    nickname: Option<String>,
    created_at: Instant,
}

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

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThreadContextRepeat {
    #[default]
    FirstSeen,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum IrcPresenceMode {
    #[default]
    SingleClient,
    PerActor,
}

#[derive(Debug)]
struct ThreadPresentation {
    reply_target: Option<String>,
    plaintext_prefix: Option<String>,
    mode_used: &'static str,
}

#[derive(Clone, Debug, Default)]
struct IrcCapabilityState {
    supports_message_tags: bool,
    supports_reply_tags: bool,
}

#[derive(Clone)]
struct OutboundContext {
    channel: String,
    reply_target: Option<String>,
    irc_message_id: Option<String>,
}

#[derive(Clone)]
struct SessionCommand {
    outbound: OutboundContext,
    payload: SessionPayload,
}

#[derive(Clone)]
enum SessionPayload {
    Privmsg(String),
    Action(String),
}

#[derive(Clone)]
struct SessionManager {
    inner: Arc<SessionManagerInner>,
}

struct SessionManagerInner {
    config: Config,
    transport_id: usize,
    presence_prefix: String,
    capabilities: Arc<Mutex<IrcCapabilityState>>,
    pool: Pool,
    state: Mutex<HashMap<String, ActorSessionHandle>>,
}

struct ActorSessionHandle {
    sender: mpsc::Sender<SessionCommand>,
    channels: HashMap<String, usize>,
    last_used: Instant,
}

impl SessionManager {
    fn new(
        config: Config,
        transport_id: usize,
        capabilities: Arc<Mutex<IrcCapabilityState>>,
        presence_prefix: String,
        pool: Pool,
    ) -> Self {
        Self {
            inner: Arc::new(SessionManagerInner {
                config,
                transport_id,
                presence_prefix,
                capabilities,
                pool,
                state: Mutex::new(HashMap::new()),
            }),
        }
    }

    async fn send_for_actor(
        &self,
        actor: &RemoteActor,
        outbound: OutboundContext,
        payload: SessionPayload,
    ) -> anyhow::Result<()> {
        let actor_key = format!("{}:{}", actor.transport(), actor.remote_id());
        let channel = outbound.channel.clone();
        let initial_nick = self.nick_from_actor(actor, 0);
        let sender = {
            let mut state = self.inner.state.lock().unwrap();
            let handle = state.entry(actor_key.clone()).or_insert_with(|| {
                let sender = self.spawn_session_task(actor.clone(), initial_nick.clone());
                ActorSessionHandle {
                    sender,
                    channels: HashMap::new(),
                    last_used: Instant::now(),
                }
            });
            *handle.channels.entry(channel).or_insert(0) = 1;
            handle.last_used = Instant::now();
            handle.sender.clone()
        };

        if let Err(err) = sender.send(SessionCommand { outbound, payload }).await {
            let mut state = self.inner.state.lock().unwrap();
            state.remove(&actor_key);
            return Err(anyhow!(
                "failed to send IRC session command for {}: {}",
                initial_nick,
                err
            ));
        }

        Ok(())
    }

    fn spawn_session_task(
        &self,
        actor: RemoteActor,
        initial_nick: String,
    ) -> mpsc::Sender<SessionCommand> {
        let (tx, mut rx) = mpsc::channel::<SessionCommand>(32);
        let manager = self.clone();
        tokio::spawn(async move {
            let (mut nick_attempt, mut current_nick) =
                match load_irc_presence(&manager.inner.pool, &actor).await {
                    Ok(Some((preferred_nick, last_successful_nick, collision_suffix))) => {
                        let nick_attempt = collision_suffix.max(0) as usize;
                        let nick = last_successful_nick
                            .or(preferred_nick)
                            .unwrap_or(initial_nick.clone());
                        (nick_attempt, nick)
                    }
                    Ok(None) | Err(_) => (0usize, initial_nick),
                };
            let _ = upsert_irc_presence(
                &manager.inner.pool,
                &actor,
                Some(&current_nick),
                None,
                nick_attempt as i64,
                false,
            )
            .await;
            let mut connected: Option<(Client, irc::client::ClientStream)> = None;
            let born_at = Instant::now();
            let mut idle_deadline = tokio::time::Instant::now() + ACTOR_SESSION_IDLE_TIMEOUT;

            loop {
                tokio::select! {
                    Some(command) = rx.recv() => {
                        idle_deadline = tokio::time::Instant::now() + ACTOR_SESSION_IDLE_TIMEOUT;
                        if connected.is_none() {
                            match manager.connect_actor_client(&current_nick).await {
                                Ok(client_stream) => {
                                    let _ = upsert_irc_presence(&manager.inner.pool, &actor, Some(&current_nick), Some(&current_nick), nick_attempt as i64, true).await;
                                    connected = Some(client_stream)
                                },
                                Err(err) => {
                                    eprintln!("Failed to connect actor IRC session {}: {:#}", current_nick, err);
                                    nick_attempt += 1;
                                    current_nick = manager.nick_from_actor(&actor, nick_attempt);
                                    let _ = upsert_irc_presence(&manager.inner.pool, &actor, Some(&current_nick), None, nick_attempt as i64, false).await;
                                    continue;
                                }
                            }
                        }

                        let mut reconnect = false;
                        if let Some((client, _)) = connected.as_mut() {
                            if let Err(err) = manager.send_session_command(client, &command).await {
                                eprintln!("Actor IRC session send failure for {}: {:#}", current_nick, err);
                                reconnect = true;
                            }
                        }
                        if reconnect {
                            connected = None;
                            nick_attempt += 1;
                            current_nick = manager.nick_from_actor(&actor, nick_attempt);
                            let _ = upsert_irc_presence(&manager.inner.pool, &actor, Some(&current_nick), None, nick_attempt as i64, false).await;
                        }
                    }
                    _ = tokio::time::sleep_until(idle_deadline) => {
                        if born_at.elapsed() < ACTOR_SESSION_MIN_LIFETIME {
                            idle_deadline = tokio::time::Instant::now() + (ACTOR_SESSION_MIN_LIFETIME - born_at.elapsed());
                            continue;
                        }
                        break;
                    }
                    maybe_msg = async {
                        if let Some((_, stream)) = connected.as_mut() {
                            tokio_stream::StreamExt::next(stream).await
                        } else {
                            None
                        }
                    }, if connected.is_some() => {
                        match maybe_msg {
                            Some(Ok(message)) => {
                                manager.update_capabilities_from_message(&message);
                                if matches!(message.command, Command::Response(code, _) if code == irc::proto::Response::ERR_NICKNAMEINUSE) {
                                    connected = None;
                                    nick_attempt += 1;
                                    current_nick = manager.nick_from_actor(&actor, nick_attempt);
                                    let _ = upsert_irc_presence(&manager.inner.pool, &actor, Some(&current_nick), None, nick_attempt as i64, false).await;
                                }
                            }
                            Some(Err(err)) => {
                                eprintln!("Actor IRC session stream error for {}: {}", current_nick, err);
                                connected = None;
                                nick_attempt += 1;
                                current_nick = manager.nick_from_actor(&actor, nick_attempt);
                            }
                            None => {
                                connected = None;
                            }
                        }
                    }
                    else => break,
                }
            }

            if let Some((client, _)) = connected.as_ref() {
                if let Err(err) = client.send_quit("idle actor session") {
                    eprintln!(
                        "Failed to quit actor IRC session {}: {:#}",
                        current_nick, err
                    );
                }
            }
        });
        tx
    }

    async fn connect_actor_client(
        &self,
        nickname: &str,
    ) -> anyhow::Result<(Client, irc::client::ClientStream)> {
        let mut config = self.inner.config.clone();
        config.nickname = Some(nickname.to_string());
        let mut client = Client::from_config(config).await?;
        client.send_cap_req(&[
            Capability::MultiPrefix,
            Capability::Custom("message-tags"),
            Capability::Custom("draft/reply"),
            Capability::ServerTime,
            Capability::EchoMessage,
        ])?;
        client.identify()?;
        let stream = client.stream()?;
        Ok((client, stream))
    }

    async fn send_session_command(
        &self,
        client: &Client,
        command: &SessionCommand,
    ) -> irc::error::Result<()> {
        client.send_join(&command.outbound.channel)?;
        let tags = self.tags_for_outbound_message(
            command.outbound.reply_target.as_deref(),
            command.outbound.irc_message_id.as_deref(),
        );
        let body = match &command.payload {
            SessionPayload::Privmsg(text) => text.clone(),
            SessionPayload::Action(text) => format!("\x01ACTION {}\x01", text),
        };

        if let Some(tags) = tags {
            client.send(IrcMessage {
                tags: Some(tags),
                prefix: None,
                command: Command::PRIVMSG(command.outbound.channel.clone(), body),
            })
        } else {
            client.send_privmsg(&command.outbound.channel, body)
        }
    }

    fn tags_for_outbound_message(
        &self,
        reply_target: Option<&str>,
        irc_message_id: Option<&str>,
    ) -> Option<Vec<Tag>> {
        let capabilities = self.inner.capabilities.lock().unwrap().clone();
        if !capabilities.supports_message_tags {
            return None;
        }

        let mut tags = Vec::new();
        if let Some(irc_message_id) = irc_message_id {
            tags.push(Tag(
                "draft/msgid".to_string(),
                Some(irc_message_id.to_string()),
            ));
        }
        if capabilities.supports_reply_tags {
            if let Some(reply_target) = reply_target {
                tags.push(Tag(
                    "+draft/reply".to_string(),
                    Some(reply_target.to_string()),
                ));
            }
        }

        if tags.is_empty() {
            None
        } else {
            Some(tags)
        }
    }

    fn nick_from_actor(&self, actor: &RemoteActor, attempt: usize) -> String {
        let prefix = self.inner.presence_prefix.as_str();
        let display = actor
            .display_name()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .take(15)
            .collect::<String>();
        let stable_suffix = actor
            .remote_id()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(6)
            .collect::<String>();
        let display = if display.is_empty() {
            "user"
        } else {
            display.as_str()
        };
        let stable_suffix = if stable_suffix.is_empty() {
            "anon"
        } else {
            stable_suffix.as_str()
        };
        let base = format!("{}{}-{}", prefix, display, stable_suffix);
        if attempt == 0 {
            base
        } else {
            format!("{}-{}", base.chars().take(26).collect::<String>(), attempt)
        }
    }

    fn update_capabilities_from_message(&self, message: &IrcMessage) {
        let Command::CAP(_, subcommand, _, Some(extensions)) = &message.command else {
            return;
        };
        if *subcommand != CapSubCommand::ACK {
            return;
        }
        let mut caps = self.inner.capabilities.lock().unwrap();
        for capability in extensions.split_whitespace() {
            match capability {
                "message-tags" => caps.supports_message_tags = true,
                "draft/reply" | "reply" => caps.supports_reply_tags = true,
                _ => continue,
            }
        }
    }
}

pub(crate) struct IRC {
    transport_id: usize,
    config: Config,
    img_root: String,
    channels: HashMap<String, broadcast::Sender<Message>>,
    pool: Pool,
    pipo_id: Arc<Mutex<i64>>,
    capabilities: Arc<Mutex<IrcCapabilityState>>,
    thread_presentation_mode: ThreadPresentationMode,
    thread_fallback_style: ThreadFallbackStyle,
    thread_context_repeat: ThreadContextRepeat,
    thread_excerpt_len: usize,
    show_thread_root_marker: bool,
    seen_thread_tokens: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    reply_tokens: Arc<Mutex<HashMap<(String, String), ReplyTokenEntry>>>,
    presence_mode: IrcPresenceMode,
    session_manager: Option<SessionManager>,
}

impl IRC {
    #[allow(clippy::too_many_arguments)]
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
        presence_mode: IrcPresenceMode,
        transport_id: usize,
    ) -> anyhow::Result<IRC> {
        let channels = channel_mapping
            .iter()
            .filter_map(|(channelname, busname)| {
                bus_map
                    .get(busname.as_ref())
                    .map(|sender| (channelname.as_ref().clone(), sender.clone()))
                    .or_else(|| {
                        eprintln!("No bus named '{}' in configuration file.", busname);
                        None
                    })
            })
            .collect();
        let config = if let Some((server_addr, server_port)) = server.rsplit_once(':') {
            Config {
                nickname: Some(nickname.clone()),
                server: Some(server_addr.to_string()),
                port: Some(server_port.parse()?),
                use_tls: Some(use_tls),
                ..Config::default()
            }
        } else {
            Config {
                nickname: Some(nickname.clone()),
                server: Some(server.clone()),
                use_tls: Some(use_tls),
                ..Config::default()
            }
        };
        let capabilities = Arc::new(Mutex::new(IrcCapabilityState::default()));
        let session_manager = if presence_mode == IrcPresenceMode::PerActor {
            Some(SessionManager::new(
                config.clone(),
                transport_id,
                capabilities.clone(),
                nickname
                    .chars()
                    .take(2)
                    .collect::<String>()
                    .to_ascii_uppercase(),
                pool.clone(),
            ))
        } else {
            None
        };

        Ok(IRC {
            transport_id,
            config,
            img_root: img_root.to_string(),
            channels,
            pool,
            pipo_id,
            capabilities,
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
            reply_tokens: Arc::new(Mutex::new(HashMap::new())),
            presence_mode,
            session_manager,
        })
    }

    pub async fn connect(&mut self) -> anyhow::Result<()> {
        loop {
            let (client, mut irc_stream, mut input_buses) = self.connect_irc().await?;
            loop {
                tokio::select! {
                    Some((channel, message)) = tokio_stream::StreamExt::next(&mut input_buses) => {
                        let message = message.unwrap();
                        match message {
                            Message::Action { sender, pipo_id, actor, thread, message, attachments, is_edit, irc_flag } if sender != self.transport_id => {
                                self.handle_action_message(&client, &channel, pipo_id, actor, thread, message, attachments, is_edit, irc_flag).await;
                            }
                            Message::Bot { sender, transport, message, attachments, is_edit, .. } if sender != self.transport_id => {
                                self.handle_bot_message(&client, &channel, transport, message, attachments, is_edit);
                            }
                            Message::Names { sender, actor, message } if sender != self.transport_id => {
                                self.handle_names_message(&client, &channel, actor, message);
                            }
                            Message::Text { sender, pipo_id, actor, thread, message, attachments, is_edit, irc_flag } if sender != self.transport_id => {
                                self.handle_text_message(&client, &channel, pipo_id, actor, thread, message, attachments, is_edit, irc_flag).await;
                            }
                            _ => {}
                        }
                    }
                    Some(message) = tokio_stream::StreamExt::next(&mut irc_stream) => {
                        if let Err(e) = message {
                            eprintln!("IRC Error: {}", e);
                            break;
                        }
                        let message = message.unwrap();
                        let nickname = match message.prefix {
                            Some(Prefix::Nickname(ref nickname, _, _)) => nickname.to_string(),
                            Some(Prefix::ServerName(ref servername)) => servername.to_string(),
                            None => String::new(),
                        };
                        self.update_capabilities_from_message(&message);
                        let irc_message_id = IRC::parse_message_id_tag(&message);
                        match message.command {
                            Command::PRIVMSG(channel, message) => {
                                if let Err(e) = self.handle_priv_msg(&client, nickname, channel, message, irc_message_id).await {
                                    eprintln!("Error handling PRIVMSG: {}", e);
                                }
                            }
                            Command::NOTICE(channel, message) => {
                                if let Err(e) = self.handle_notice(nickname, channel, message, irc_message_id).await {
                                    eprintln!("Error handling NOTICE: {}", e);
                                }
                            }
                            _ => {}
                        }
                    }
                    else => break,
                }
            }
        }
    }

    async fn handle_action_message(
        &self,
        client: &Client,
        channel: &str,
        pipo_id: i64,
        actor: RemoteActor,
        thread: Option<ThreadRef>,
        message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        is_edit: bool,
        irc_flag: bool,
    ) {
        self.handle_actor_outbound(
            client,
            channel,
            pipo_id,
            actor,
            thread,
            message,
            attachments,
            is_edit,
            irc_flag,
            true,
        )
        .await;
    }

    async fn handle_text_message(
        &self,
        client: &Client,
        channel: &str,
        pipo_id: i64,
        actor: RemoteActor,
        thread: Option<ThreadRef>,
        message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        is_edit: bool,
        irc_flag: bool,
    ) {
        self.handle_actor_outbound(
            client,
            channel,
            pipo_id,
            actor,
            thread,
            message,
            attachments,
            is_edit,
            irc_flag,
            false,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_actor_outbound(
        &self,
        client: &Client,
        channel: &str,
        pipo_id: i64,
        actor: RemoteActor,
        thread: Option<ThreadRef>,
        message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        is_edit: bool,
        irc_flag: bool,
        is_action: bool,
    ) {
        let irc_message_id = self.ensure_ircid_for_pipo_id(pipo_id).await;
        let mut message = message;
        if irc_flag && is_edit {
            message = None;
        }
        if let Some(message) = message {
            let mut is_edit_line = is_edit;
            let thread_presentation = self
                .resolve_thread_presentation(channel, pipo_id, &thread)
                .await;
            if thread.is_some() {
                self.log_thread_presentation(channel, pipo_id, &thread_presentation);
            }
            let base_ctx = OutboundContext {
                channel: channel.to_string(),
                reply_target: thread_presentation.reply_target.clone(),
                irc_message_id: irc_message_id.clone(),
            };
            if let Some(prefix) = thread_presentation.plaintext_prefix.as_ref() {
                let prefix_payload = if self.presence_mode == IrcPresenceMode::PerActor {
                    SessionPayload::Privmsg(prefix.clone())
                } else if is_action {
                    SessionPayload::Action(format!(
                        "\x02* \x02{}\x02 {}",
                        self.irc_nick_from_actor(&actor),
                        prefix
                    ))
                } else {
                    SessionPayload::Action(format!(
                        "<{}> {}",
                        self.irc_nick_from_actor(&actor),
                        prefix
                    ))
                };
                self.dispatch_actor_message(client, &actor, base_ctx.clone(), prefix_payload)
                    .await;
            }
            for msg in message.split('\n').filter(|msg| !msg.is_empty()) {
                let payload = if self.presence_mode == IrcPresenceMode::PerActor {
                    if is_action {
                        SessionPayload::Action(if is_edit_line {
                            format!("{}*", msg)
                        } else {
                            msg.to_string()
                        })
                    } else {
                        SessionPayload::Privmsg(if is_edit_line {
                            format!("EDIT: {}", msg)
                        } else {
                            msg.to_string()
                        })
                    }
                } else if is_action {
                    SessionPayload::Action(if is_edit_line {
                        format!(
                            "\x02* \x02{}\x02 {}*",
                            self.irc_nick_from_actor(&actor),
                            msg
                        )
                    } else {
                        format!("\x02* \x02{}\x02 {}", self.irc_nick_from_actor(&actor), msg)
                    })
                } else {
                    SessionPayload::Action(if is_edit_line {
                        format!(
                            "<{}> \x02EDIT:\x02 {}",
                            self.irc_nick_from_actor(&actor),
                            msg
                        )
                    } else {
                        format!("<{}> {}", self.irc_nick_from_actor(&actor), msg)
                    })
                };
                self.dispatch_actor_message(client, &actor, base_ctx.clone(), payload)
                    .await;
                is_edit_line = false;
            }
        }
        if let Some(attachments) = attachments {
            IRC::handle_attachments(client, channel, attachments);
        }
    }

    async fn dispatch_actor_message(
        &self,
        client: &Client,
        actor: &RemoteActor,
        outbound: OutboundContext,
        payload: SessionPayload,
    ) {
        if self.presence_mode == IrcPresenceMode::PerActor {
            if let Some(manager) = &self.session_manager {
                if let Err(e) = manager
                    .send_for_actor(actor, outbound.clone(), payload.clone())
                    .await
                {
                    eprintln!(
                        "Per-actor IRC send failed, falling back to singleton client: {:#}",
                        e
                    );
                } else {
                    return;
                }
            }
        }
        let message = match payload {
            SessionPayload::Privmsg(text) => text,
            SessionPayload::Action(text) => format!("\x01ACTION {}\x01", text),
        };
        if let Err(e) = self
            .send_privmsg_with_tags(
                client,
                &outbound.channel,
                message.clone(),
                outbound.reply_target.as_deref(),
                outbound.irc_message_id.as_deref(),
            )
            .await
        {
            eprintln!(
                "Failed to send message '{}' channel {}: {:#}",
                message, outbound.channel, e
            );
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
        if is_edit {
            if let Some(attachment) = attachments {
                IRC::handle_attachments(client, channel, attachment);
            }
        }
    }

    fn handle_names_message(
        &self,
        client: &Client,
        channel: &str,
        actor: RemoteActor,
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
                    actor,
                    message: Some(serde_json::json!(users).to_string()),
                };
                if let Some(sender) = self.channels.get(channel) {
                    let _ = sender.send(message);
                }
            }
        }
    }

    fn handle_attachments(client: &Client, channel: &str, attachments: Vec<Attachment>) {
        for attachment in attachments {
            let service_name = attachment
                .service_name
                .unwrap_or_else(|| "Unknown".to_string());
            let author_name = attachment.author_name.unwrap_or_default();
            let text = attachment.text.or(attachment.fallback);
            let Some(text) = text else {
                continue;
            };
            for (line_counter, msg) in text.split('\n').filter(|msg| !msg.is_empty()).enumerate() {
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
                let _ = client.send_privmsg(channel, message);
                if line_counter > 5 {
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
        *self.capabilities.lock().unwrap() = IrcCapabilityState::default();
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
        for (channel_name, channel) in &self.channels {
            input_buses.insert(
                channel_name.clone(),
                BroadcastStream::new(channel.subscribe()),
            );
            let _ = client.send_join(channel_name);
        }
        Ok((client, irc_stream, input_buses))
    }

    fn update_capabilities_from_message(&self, message: &IrcMessage) {
        let Command::CAP(_, subcommand, _, Some(extensions)) = &message.command else {
            return;
        };
        if *subcommand != CapSubCommand::ACK {
            return;
        }
        let mut caps = self.capabilities.lock().unwrap();
        for capability in extensions.split_whitespace() {
            match capability {
                "message-tags" => caps.supports_message_tags = true,
                "draft/reply" | "reply" => caps.supports_reply_tags = true,
                _ => {}
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
        let capabilities = self.capabilities.lock().unwrap().clone();
        if !capabilities.supports_message_tags {
            return None;
        }
        let mut tags = Vec::new();
        if let Some(irc_message_id) = irc_message_id {
            tags.push(Tag(
                "draft/msgid".to_string(),
                Some(irc_message_id.to_string()),
            ));
        }
        if capabilities.supports_reply_tags {
            if let Some(reply_target) = reply_target {
                tags.push(Tag(
                    "+draft/reply".to_string(),
                    Some(reply_target.to_string()),
                ));
            }
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
            return ThreadPresentation::default();
        }
        self.remember_reply_token(channel, thread, None);
        let caps = self.capabilities.lock().unwrap().clone();
        let can_use_reply_tags = caps.supports_message_tags
            && caps.supports_reply_tags
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
            ThreadPresentationMode::Auto if reply_target.is_some() => ThreadPresentation {
                reply_target,
                plaintext_prefix: None,
                mode_used: "ircv3_tag",
            },
            ThreadPresentationMode::Ircv3Only if reply_target.is_some() => ThreadPresentation {
                reply_target,
                plaintext_prefix: None,
                mode_used: "ircv3_tag",
            },
            ThreadPresentationMode::Ircv3Only => ThreadPresentation {
                reply_target: None,
                plaintext_prefix: None,
                mode_used: "ircv3_unavailable",
            },
            _ => ThreadPresentation {
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
            if self.thread_context_repeat == ThreadContextRepeat::FirstSeen {
                Some(format!(
                    "{} (reply with: >>{} <message>)",
                    expanded_prefix, thread_token
                ))
            } else {
                Some(expanded_prefix)
            }
        } else if self.thread_fallback_style == ThreadFallbackStyle::Verbose {
            Some(expanded_prefix)
        } else {
            Some(compact_prefix)
        }
    }

    fn mark_thread_token_seen(&self, channel: &str, token: &str) -> bool {
        let mut seen = self.seen_thread_tokens.lock().unwrap();
        seen.entry(channel.to_string())
            .or_default()
            .insert(token.to_string())
    }

    fn remember_reply_token(
        &self,
        channel: &str,
        thread: &Option<ThreadRef>,
        nickname: Option<&str>,
    ) {
        let Some(thread_ref) = thread.as_ref() else {
            return;
        };
        let token = IRC::thread_token(thread_ref);
        let mut tokens = self.reply_tokens.lock().unwrap();
        IRC::cleanup_expired_reply_tokens(&mut tokens);
        tokens.insert(
            (channel.to_string(), token),
            ReplyTokenEntry {
                thread_ref: thread_ref.clone(),
                nickname: nickname.map(str::to_string),
                created_at: Instant::now(),
            },
        );
    }

    fn cleanup_expired_reply_tokens(tokens: &mut HashMap<(String, String), ReplyTokenEntry>) {
        let now = Instant::now();
        tokens.retain(|_, entry| now.duration_since(entry.created_at) < REPLY_TOKEN_TTL);
    }

    fn resolve_reply_token(
        &self,
        channel: &str,
        nickname: Option<&str>,
        token: &str,
    ) -> Option<ThreadRef> {
        let mut tokens = self.reply_tokens.lock().unwrap();
        IRC::cleanup_expired_reply_tokens(&mut tokens);
        let entry = tokens.get(&(channel.to_string(), token.to_uppercase()))?;
        if let (Some(expected), Some(actual)) = (entry.nickname.as_deref(), nickname) {
            if !expected.eq_ignore_ascii_case(actual) {
                return None;
            }
        }
        Some(entry.thread_ref.clone())
    }

    fn active_reply_tokens_for_channel(&self, channel: &str) -> Vec<String> {
        let mut tokens = self.reply_tokens.lock().unwrap();
        IRC::cleanup_expired_reply_tokens(&mut tokens);
        tokens
            .keys()
            .filter(|(token_channel, _)| token_channel == channel)
            .map(|(_, token)| token.clone())
            .collect()
    }

    fn active_reply_token_entries_for_channel(
        &self,
        channel: &str,
    ) -> Vec<(String, ReplyTokenEntry)> {
        let mut tokens = self.reply_tokens.lock().unwrap();
        IRC::cleanup_expired_reply_tokens(&mut tokens);
        let mut entries = tokens
            .iter()
            .filter(|((token_channel, _), _)| token_channel == channel)
            .map(|((_, token), entry)| (token.clone(), entry.clone()))
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at));
        entries
    }

    fn thread_root_summary(&self, thread_ref: &ThreadRef) -> String {
        let author = IRC::sanitize_thread_context_text(thread_ref.root_author.as_deref())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unknown".to_string());
        let excerpt = IRC::sanitize_thread_context_text(thread_ref.root_excerpt.as_deref())
            .filter(|value| !value.is_empty())
            .map(|value| IRC::truncate_with_ellipsis(value, 40))
            .unwrap_or_else(|| "…".to_string());
        format!("{}: {}", author, excerpt)
    }

    async fn handle_local_thread_command(
        &self,
        client: &Client,
        channel: &str,
        message: &str,
    ) -> anyhow::Result<bool> {
        let trimmed = message.trim();
        if trimmed.eq_ignore_ascii_case("/threads") {
            let entries = self.active_reply_token_entries_for_channel(channel);
            let rendered = if entries.is_empty() {
                "none cached yet".to_string()
            } else {
                entries
                    .into_iter()
                    .take(THREAD_LIST_LIMIT)
                    .map(|(token, entry)| {
                        format!(
                            "{} ({})",
                            token,
                            self.thread_root_summary(&entry.thread_ref)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            };
            client.send_notice(
                channel,
                format!(
                    "Recent thread tokens (latest {}): {}",
                    THREAD_LIST_LIMIT, rendered
                ),
            )?;
            return Ok(true);
        }
        if trimmed.eq_ignore_ascii_case("/threadhelp") || trimmed.eq_ignore_ascii_case("/help") {
            client.send_notice(channel, "Thread replies: >>TOKEN your reply (example: >>K7F2 thanks) or /reply TOKEN your reply. Use /threads to list recent tokens.")?;
            return Ok(true);
        }
        Ok(false)
    }

    fn parse_reply_command(message: &str) -> Option<(String, String)> {
        let trimmed = message.trim_start();
        let (token, remaining) = if let Some(command) = trimmed.strip_prefix(">>") {
            let mut parts = command.splitn(2, char::is_whitespace);
            (parts.next()?.trim(), parts.next()?.trim_start())
        } else if let Some(command) = trimmed.strip_prefix("/reply") {
            let command = command.trim_start();
            let mut parts = command.splitn(2, char::is_whitespace);
            (parts.next()?.trim(), parts.next()?.trim_start())
        } else {
            return None;
        };
        if token.is_empty()
            || remaining.is_empty()
            || !token.chars().all(|ch| ch.is_ascii_alphanumeric())
        {
            return None;
        }
        Some((token.to_uppercase(), remaining.to_string()))
    }

    async fn send_reply_token_usage_notice(&self, client: &Client, channel: &str) {
        let mut active = self.active_reply_tokens_for_channel(channel);
        active.sort();
        let sample = if active.is_empty() {
            "none currently cached".to_string()
        } else {
            active.into_iter().take(8).collect::<Vec<_>>().join(", ")
        };
        let _ = client.send_notice(channel, format!("Unknown or expired reply token. Usage: >>TOKEN your reply or /reply TOKEN your reply. Discover tokens from [t:TOKEN] thread markers in recent bridged messages. Active tokens: {}", sample));
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
        IRC::to_base36(hash.max(1)).chars().take(4).collect()
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
        self.select_slackid_from_messages(pipo_id)
            .await
            .is_some_and(|slackid| thread_root_id == slackid)
    }

    fn sanitize_thread_context_text(value: Option<&str>) -> Option<String> {
        let value = value?;
        let collapsed = value
            .chars()
            .map(|ch| if ch.is_ascii_control() { ' ' } else { ch })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
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
            input
        } else {
            format!(
                "{}…",
                input
                    .chars()
                    .take(max_len.saturating_sub(1))
                    .collect::<String>()
            )
        }
    }

    fn irc_nick_from_actor(&self, actor: &RemoteActor) -> String {
        let transport_prefix = actor
            .transport()
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or('U');
        let display = actor
            .display_name()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .take(20)
            .collect::<String>();
        let stable_suffix = actor
            .remote_id()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(8)
            .collect::<String>();
        format!(
            "{}{}-{}",
            transport_prefix,
            if display.is_empty() { "user" } else { &display },
            if stable_suffix.is_empty() {
                "anon"
            } else {
                &stable_suffix
            }
        )
    }

    fn parse_message_id_tag(message: &IrcMessage) -> Option<String> {
        message.tags.as_ref()?.iter().find_map(|Tag(key, value)| {
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
        match client.head(url.clone()).send().await {
            Ok(response) => {
                if let Some(etag) = response
                    .headers()
                    .get(reqwest::header::ETAG)
                    .and_then(|v| v.to_str().ok())
                {
                    format!("{}/{}.png?{}", self.img_root, nickname, etag)
                } else {
                    url
                }
            }
            Err(_) => format!("{}/irc.png", self.img_root),
        }
    }

    async fn handle_priv_msg(
        &self,
        client: &Client,
        nickname: String,
        channel: String,
        message: String,
        irc_message_id: Option<String>,
    ) -> anyhow::Result<()> {
        if self
            .handle_local_thread_command(client, &channel, &message)
            .await?
        {
            return Ok(());
        }
        let Some(sender) = self.channels.get(&channel) else {
            return Err(anyhow!("Could not get sender for channel {}", channel));
        };
        lazy_static! {
            static ref RE: Regex = Regex::new("^\x01ACTION (.*)\x01\r?$").unwrap();
        }
        let pipo_id = self.insert_into_messages_table().await?;
        if let Some(irc_message_id) = irc_message_id {
            self.update_messages_ircid(pipo_id, Some(irc_message_id))
                .await?;
        }
        let avatar_url = self.get_avatar_url(&nickname).await;
        let mut thread = None;
        let content = if let Some(message) = RE.captures(&message) {
            message.get(1).unwrap().as_str().to_string()
        } else {
            message.to_string()
        };
        let mut content = content;
        if let Some((token, parsed_message)) = IRC::parse_reply_command(&content) {
            if let Some(thread_ref) = self.resolve_reply_token(&channel, Some(&nickname), &token) {
                thread = Some(thread_ref);
                content = parsed_message;
            } else {
                self.send_reply_token_usage_notice(client, &channel).await;
                return Ok(());
            }
        }
        let actor = RemoteActor::new(
            TRANSPORT_NAME,
            nickname.clone(),
            nickname.clone(),
            Some(avatar_url),
        );
        if let Err(e) = upsert_remote_actor(&self.pool, &actor).await {
            eprintln!("Failed to upsert IRC actor: {}", e);
        }
        let _ = upsert_irc_presence(
            &self.pool,
            &actor,
            Some(&self.irc_nick_from_actor(&actor)),
            None,
            0,
            true,
        )
        .await;
        let outbound = if RE.is_match(&message) {
            Message::Action {
                sender: self.transport_id,
                pipo_id,
                actor: actor.clone(),
                thread,
                message: Some(content),
                attachments: None,
                is_edit: false,
                irc_flag: false,
            }
        } else {
            Message::Text {
                sender: self.transport_id,
                pipo_id,
                actor,
                thread,
                message: Some(content),
                attachments: None,
                is_edit: false,
                irc_flag: false,
            }
        };
        sender
            .send(outbound)
            .map(|_| ())
            .map_err(|e| anyhow!("Couldn't send message: {:#}", e))
    }

    async fn insert_into_messages_table(&self) -> anyhow::Result<i64> {
        let conn = self.pool.get().await.unwrap();
        let pipo_id = *self.pipo_id.lock().unwrap();
        conn.interact(move |conn| -> anyhow::Result<usize> {
            Ok(conn.execute(
                "INSERT OR REPLACE INTO messages (id) VALUES (?1)",
                params![pipo_id],
            )?)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("Interact Error")))?;
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
        self.update_messages_ircid(pipo_id, Some(generated.clone()))
            .await
            .ok()
            .map(|_| generated)
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
        let Some(sender) = self.channels.get(&channel) else {
            return Err(anyhow!("Could not get sender for channel {}", channel));
        };
        lazy_static! {
            static ref RE: Regex = Regex::new("^\x01ACTION (.*)\x01\r?$").unwrap();
        }
        let pipo_id = self.insert_into_messages_table().await?;
        if let Some(irc_message_id) = irc_message_id {
            self.update_messages_ircid(pipo_id, Some(irc_message_id))
                .await?;
        }
        let avatar_url = self.get_avatar_url(&nickname).await;
        let actor = RemoteActor::new(
            TRANSPORT_NAME,
            nickname.clone(),
            nickname.clone(),
            Some(avatar_url),
        );
        if let Err(e) = upsert_remote_actor(&self.pool, &actor).await {
            eprintln!("Failed to upsert IRC actor: {}", e);
        }
        let _ = upsert_irc_presence(
            &self.pool,
            &actor,
            Some(&self.irc_nick_from_actor(&actor)),
            None,
            0,
            true,
        )
        .await;
        let outbound = if let Some(message) = RE.captures(&message) {
            Message::Action {
                sender: self.transport_id,
                pipo_id,
                actor: actor.clone(),
                thread: None,
                message: Some(format!("```{}```", message.get(1).unwrap().as_str())),
                attachments: None,
                is_edit: false,
                irc_flag: false,
            }
        } else {
            Message::Text {
                sender: self.transport_id,
                pipo_id,
                actor,
                thread: None,
                message: Some(format!("```{}```", message)),
                attachments: None,
                is_edit: false,
                irc_flag: false,
            }
        };
        sender
            .send(outbound)
            .map(|_| ())
            .map_err(|e| anyhow!("Couldn't send message: {:#}", e))
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
