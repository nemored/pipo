use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use anyhow::anyhow;
use async_recursion::async_recursion;
use deadpool_sqlite::Pool;
use futures::{
    stream::{SplitSink, SplitStream, StreamExt as FuturesStreamExt},
    SinkExt,
};
use lazy_static::lazy_static;
use regex::{Captures, Regex};
use reqwest::{
    header::{self, HeaderMap},
    multipart::Form,
    Client as HttpClient, Method,
};
use rusqlite::params;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::{net::TcpStream, sync::broadcast};
use tokio_stream::{wrappers::BroadcastStream, StreamExt, StreamMap};
use tokio_tungstenite::*;

use crate::Message;

mod entity_resolver;
pub mod objects;
mod rich_text_renderer;
use entity_resolver::{
    format_broadcast, format_channel, format_link, format_team, format_user, format_usergroup,
    parse_entity_reference, EntityReference,
};
use objects::{Message as SlackMessage, *};
use rich_text_renderer::{RenderOptions, RichTextResolver};
//mod parse;
//use parse::*;

const TRANSPORT_NAME: &'static str = "Slack";

pub(crate) struct Slack {
    transport_id: usize,
    http: HttpClient,
    websocket: WebSocket,
    token: String,
    bot_token: String,
    pool: Pool,
    pipo_id: Arc<Mutex<i64>>,
    channels: HashMap<String, broadcast::Sender<Message>>,
    channel_map: HashMap<String, String>,
    id_map: HashMap<String, String>,
    users: HashMap<String, User>,
    user_id_map: HashMap<String, String>,
    seen_event_ids: VecDeque<String>,
    irc_formatting_enabled: bool,
}

struct WebSocket {
    endpoint: Option<reqwest::Url>,
    ws_sink: Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, tungstenite::Message>>,
    ws_stream: StreamMap<usize, SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>,
    next_connection_id: usize,
}

impl Slack {
    fn append_rich_text_fragment(acc: &mut String, fragment: &str) {
        if fragment.is_empty() {
            return;
        }

        if !acc.is_empty() && !acc.ends_with('\n') && !fragment.starts_with('\n') {
            acc.push('\n');
        }

        acc.push_str(fragment);
    }

    pub async fn new(
        transport_id: usize,
        bus_map: &HashMap<String, broadcast::Sender<Message>>,
        pipo_id: Arc<Mutex<i64>>,
        pool: Pool,
        token: String,
        bot_token: String,
        irc_formatting_enabled: bool,
        channel_mapping: &HashMap<Arc<String>, Arc<String>>,
    ) -> anyhow::Result<Slack> {
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

        Ok(Slack {
            transport_id,
            http: HttpClient::new(),
            websocket: WebSocket {
                endpoint: None,
                ws_sink: None,
                ws_stream: StreamMap::new(),
                next_connection_id: 0,
            },
            token,
            bot_token,
            channels,
            pool,
            pipo_id,
            channel_map: HashMap::new(),
            id_map: HashMap::new(),
            users: HashMap::new(),
            user_id_map: HashMap::new(),
            seen_event_ids: VecDeque::with_capacity(50),
            irc_formatting_enabled,
        })
    }

    pub async fn connect(&mut self) -> anyhow::Result<()> {
        self.connect_websocket().await?;

        let mut headers = HeaderMap::new();

        headers.insert(
            header::AUTHORIZATION,
            ("Bearer ".to_owned() + &self.bot_token).parse()?,
        );

        let response = self
            .http
            .request(Method::POST, "https://slack.com/api/conversations.list")
            .headers(headers)
            .send()
            .await?;
        let json: Value = serde_json::from_str(response.text().await?.as_str())?;

        if json["ok"] == false {
            return Err(anyhow!("{:?}", json));
        }

        for channel in json["channels"].as_array().unwrap().iter() {
            let name = format!("#{}", channel.get("name").unwrap().as_str().unwrap());
            let id = channel.get("id").unwrap().as_str().unwrap().to_string();
            self.channel_map.insert(name.clone(), id.clone());
            self.id_map.insert(id, name);
        }

        let mut input_buses = StreamMap::new();

        for (channel_name, channel) in self.channels.iter() {
            input_buses.insert(
                channel_name.clone(),
                BroadcastStream::new(channel.subscribe()),
            );
        }

        self.get_users_list().await?;

        loop {
            tokio::select! {
            Some((channel, message))
                = StreamExt::next(&mut input_buses) => {
                let message = message.unwrap();
                match message {
                    Message::Action {
                    sender,
                    pipo_id,
                    transport,
                    username,
                    avatar_url,
                    thread,
                    message,
                    attachments,
                    is_edit,
                    irc_flag: _,
                    } => {
                    if sender != self.transport_id {
                        if let Err(e)
                        = self.post_action_message(pipo_id,
                                       &channel,
                                       transport,
                                       username,
                                       avatar_url,
                                       thread,
                                       message,
                                       attachments,
                                       is_edit)
                        .await {
                            eprintln!("Failed to post message:\
                                   {}", e);
                        }
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
                        if let Err(e)
                        = self.post_bot_message(&channel,
                                    transport,
                                    message,
                                    attachments,
                                    is_edit)
                        .await {
                            eprintln!("Failed to post message:\
                                   {}", e);
                        }
                    }
                    },
                    Message::Delete {
                    sender,
                    pipo_id,
                    transport: _,
                    } => {
                    if sender != self.transport_id {
                        if let Err(e)
                        = self.delete_message(pipo_id,
                                      &channel).await {
                            eprintln!("Couldn't delete \
                                   message: {}", e);
                        }
                    }
                    },
                    Message::Names {
                    sender,
                    transport,
                    username,
                    message,
                    } => {
                    if sender != self.transport_id {
                        if let Err(e)
                        = self.post_names_message(&channel,
                                      transport,
                                      username,
                                      message)
                        .await {
                            eprintln!("Failed to post message:\
                                   {}", e);
                        }
                    }
                    },
                    Message::Pin {
                    sender,
                    pipo_id,
                    remove,
                    } => if sender != self.transport_id {
                    if !remove {
                        if let Err(e) = self.pins_add(&channel,
                                      pipo_id)
                        .await {
                            eprintln!("Failed to remove pin: \
                                   {}", e)
                        }
                    }
                    else {
                        if let Err(e) = self.pins_remove(&channel,
                                         pipo_id)
                        .await {
                            eprintln!("Failed to add pin: {}",
                                  e)
                        }
                    }
                    },
                    Message::Reaction {
                    sender,
                    pipo_id,
                    transport,
                    emoji,
                    remove,
                    username,
                    avatar_url,
                    thread
                    } => if sender != self.transport_id {
                    if !remove {
                        if let Err(e)
                        = self.add_reaction(pipo_id,
                                    &channel,
                                    transport,
                                    emoji,
                                    username,
                                    avatar_url,
                                    thread).await {
                            eprintln!("Failed to add \
                                   reaction: {}", e)
                        }
                    }
                    else {
                        if let Err(e)
                        = self.remove_reaction(pipo_id,
                                       &channel,
                                       transport,
                                       emoji,
                                       username,
                                       avatar_url,
                                       thread)
                        .await {
                            eprintln!("Failed to remove \
                                   reaction: {}", e)
                        }
                    }
                    }
                    Message::Text {
                    sender,
                    pipo_id,
                    transport,
                    username,
                    avatar_url,
                    thread,
                    message,
                    attachments,
                    is_edit,
                    irc_flag: _,
                    } => {
                    if sender != self.transport_id {
                        if let Err(e)
                        = self.post_text_message(pipo_id,
                                     &channel,
                                     transport,
                                     username,
                                     avatar_url,
                                     thread,
                                     message,
                                     attachments,
                                     is_edit)
                        .await {
                            eprintln!("Failed to post message:\
                                   {}", e);
                        }
                    }
                    }
                }
                }
            message
                = StreamExt::next(&mut self.websocket.ws_stream) => {
                // eprintln!("WS Message: {:?}", message);
                match message {
                    Some((cid, Ok(message))) => {
                    if message.is_text() {
                        // eprintln!("<- Slack: {:?}", message);
                        let response: Response
                        = serde_json::from_str(&message
                                       .to_text()
                                       .unwrap())
                        .unwrap_or_else(|e| {
                            Response::Unhandled {
                            error: e.to_string(),
                            }
                        });
                        if let Err(e)
                        = self.handle_response(cid,
                                       response).await
                        {
                        eprintln!("{}\n{}", e, message);
                        }
                    }
                    },
                    Some((cid, Err(e))) => {
                    eprintln!("WebSocket error: {:#}", e);
                    if let Some(s) = self.websocket.ws_stream
                        .remove(&cid) {
                        s.reunite(self.websocket.ws_sink.take()
                              .unwrap()).unwrap()
                            .close(None).await
                            .unwrap_or_else(|_| ());
                        }
                    self.connect_websocket().await?;
                    },
                    None => return Ok(()),
                }
                }
            else => { break }
            }
            match self.websocket.ws_sink.as_mut() {
                Some(ws) => ws.flush().await?,
                None => self.connect_websocket().await?,
            }
        }

        Ok(())
    }

    async fn connect_websocket(&mut self) -> anyhow::Result<()> {
        let mut headers = HeaderMap::new();

        headers.insert(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse()?,
        );
        headers.insert(
            header::AUTHORIZATION,
            ("Bearer ".to_owned() + &self.token).parse()?,
        );

        let response = self
            .http
            .request(Method::POST, "https://slack.com/api/apps.connections.open")
            .headers(headers)
            .send()
            .await?;
        let json: Value = serde_json::from_str(response.text().await?.as_str())?;
        if json["ok"] == false {
            return Err(anyhow!("{:?}", json));
        }

        let url = format!(
            "{}{}",
            json["url"].as_str().unwrap(),
            "&debug_reconnects=false"
        );

        // eprintln!("WS URL: {}", url);

        self.websocket.endpoint = Some(reqwest::Url::parse(&url).unwrap());

        let (ws, _) = connect_async(self.websocket.endpoint.as_ref().unwrap().to_string()).await?;

        let (sink, stream) = ws.split();
        self.websocket.ws_sink = Some(sink);
        self.websocket
            .ws_stream
            .insert(self.websocket.next_connection_id, stream);
        self.websocket.next_connection_id += 1;

        Ok(())
    }

    async fn acknowledge(&mut self, envelope_id: &str) -> anyhow::Result<()> {
        let ack: Value = serde_json::json!({"envelope_id": envelope_id});
        self.websocket
            .ws_sink
            .as_mut()
            .unwrap()
            .feed(tungstenite::Message::text(ack.to_string() + "\n"))
            .await?;

        Ok(())
    }

    async fn add_reaction(
        &mut self,
        pipo_id: i64,
        channel: &str,
        _transport: String,
        emoji: String,
        _username: Option<String>,
        _avatar_url: Option<String>,
        _thread: Option<(Option<String>, Option<u64>)>,
    ) -> anyhow::Result<()> {
        let mut headers = HeaderMap::new();
        let channel = match self.channel_map.get(channel) {
            Some(s) => s,
            None => return Err(anyhow!("Could not find id for channel {}", channel)),
        };
        let ts = match self.select_slackid_from_messages(pipo_id).await? {
            Some(ts) => ts,
            None => return Err(anyhow!("No slack_id for given pipo_id")),
        };

        let mut shortcode = None;
        if let Some(emoji) = emojis::get_by_shortcode(&emoji) {
            if let Some(s) = emoji.shortcode() {
                shortcode = Some(s.to_string());
            }
        }

        let emoji = match shortcode {
            Some(s) => s,
            None => emoji,
        };

        let body = serde_json::json!({
            "channel":channel,
            "name":emoji,
            "timestamp":ts
        })
        .to_string();

        headers.insert(header::CONTENT_TYPE, "application/json".parse()?);
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.bot_token).parse()?,
        );

        let response = self
            .http
            .request(Method::POST, "https://slack.com/api/reactions.add")
            .headers(headers)
            .body(body)
            .send()
            .await?;

        let json: Value = serde_json::from_str(response.text().await?.as_str())?;

        if json["ok"] == false {
            return Err(anyhow!(
                "E: slack.rs:Slack::add_reaction(): {}",
                json["error"]
            ));
        }

        Ok(())
    }

    async fn delete_message(&mut self, pipo_id: i64, channel: &str) -> anyhow::Result<()> {
        let mut headers = HeaderMap::new();
        let channel = match self.channel_map.get(channel) {
            Some(s) => s,
            None => return Err(anyhow!("Could not find id for channel {}", channel)),
        };
        let ts = match self.select_slackid_from_messages(pipo_id).await? {
            Some(ts) => ts,
            None => return Err(anyhow!("No slack_id for given pipo_id")),
        };
        let body = serde_json::json!({
        "channel":channel,
        "ts":ts})
        .to_string();

        headers.insert(header::CONTENT_TYPE, "application/json".parse()?);
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.bot_token).parse()?,
        );

        let response = self
            .http
            .request(Method::POST, "https://slack.com/api/chat.delete")
            .headers(headers)
            .body(body)
            .send()
            .await?;
        let json: Value = serde_json::from_str(response.text().await?.as_str())?;
        if json["ok"] == false {
            return Err(anyhow!(
                "E: slack.rs:Slack::delete_message(): {}",
                json["error"]
            ));
        }

        Ok(())
    }

    async fn pins_add(&mut self, channel: &str, pipo_id: i64) -> anyhow::Result<()> {
        let mut headers = HeaderMap::new();
        let channel = match self.channel_map.get(channel) {
            Some(s) => s,
            None => return Err(anyhow!("Could not find id for channel {}", channel)),
        };
        let ts = match self.select_slackid_from_messages(pipo_id).await? {
            Some(ts) => ts,
            None => return Err(anyhow!("No slack_id for given pipo_id")),
        };

        let body = serde_json::json!({
            "channel":channel,
            "timestamp":ts
        })
        .to_string();

        headers.insert(header::CONTENT_TYPE, "application/json".parse()?);
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.bot_token).parse()?,
        );

        let response = self
            .http
            .request(Method::POST, "https://slack.com/api/pins.add")
            .headers(headers)
            .body(body)
            .send()
            .await?;

        let json: Value = serde_json::from_str(response.text().await?.as_str())?;

        if json["ok"] == false {
            return Err(anyhow!(
                "E: slack.rs:Slack::pins_remove(): {}",
                json["error"]
            ));
        }

        Ok(())
    }

    async fn pins_remove(&mut self, channel: &str, pipo_id: i64) -> anyhow::Result<()> {
        let mut headers = HeaderMap::new();
        let channel = match self.channel_map.get(channel) {
            Some(s) => s,
            None => return Err(anyhow!("Could not find id for channel {}", channel)),
        };
        let ts = match self.select_slackid_from_messages(pipo_id).await? {
            Some(ts) => ts,
            None => return Err(anyhow!("No slack_id for given pipo_id")),
        };

        let body = serde_json::json!({
            "channel":channel,
            "timestamp":ts
        })
        .to_string();

        headers.insert(header::CONTENT_TYPE, "application/json".parse()?);
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.bot_token).parse()?,
        );

        let response = self
            .http
            .request(Method::POST, "https://slack.com/api/pins.remove")
            .headers(headers)
            .body(body)
            .send()
            .await?;

        let json: Value = serde_json::from_str(response.text().await?.as_str())?;

        if json["ok"] == false {
            return Err(anyhow!(
                "E: slack.rs:Slack::pins_remove(): {}",
                json["error"]
            ));
        }

        Ok(())
    }

    async fn post_action_message(
        &mut self,
        pipo_id: i64,
        channel: &str,
        transport: String,
        username: String,
        avatar_url: Option<String>,
        thread: Option<(Option<String>, Option<u64>)>,
        message: Option<String>,
        attachments: Option<Vec<crate::Attachment>>,
        is_edit: bool,
    ) -> anyhow::Result<()> {
        let thread_ts = match thread {
            Some((s, _)) => s,
            None => None,
        };
        let message = message.map(|s| format!("_{}_", s));

        if is_edit {
            if let Some(ts) = self.select_slackid_from_messages(pipo_id).await? {
                return self
                    .update(
                        ts,
                        channel,
                        transport,
                        username,
                        avatar_url,
                        message,
                        attachments,
                    )
                    .await;
            }
        }

        return self
            .post_message(
                pipo_id,
                channel,
                transport,
                username,
                avatar_url,
                thread_ts,
                message,
                attachments,
            )
            .await;
    }

    async fn post_bot_message(
        &mut self,
        _channel: &str,
        _transport: String,
        _message: Option<String>,
        _attachments: Option<Vec<crate::Attachment>>,
        _is_edit: bool,
    ) -> anyhow::Result<()> {
        Err(anyhow!(
            "slack.rs:Slack::post_bot_message() not yet implemented."
        ))
    }

    async fn post_names_message(
        &mut self,
        channel: &str,
        transport: String,
        username: String,
        message: Option<String>,
    ) -> anyhow::Result<()> {
        let message = message.unwrap();
        if message.as_str() == "/names" {
            let users: Vec<String> = self.users.iter().map(|(user, _)| user.clone()).collect();
            let message = Message::Names {
                sender: self.transport_id,
                transport: TRANSPORT_NAME.to_string(),
                username: username,
                message: Some(serde_json::json!(users).to_string()),
            };

            return match self.id_map.get(channel) {
                Some(channel) => {
                    let channel = channel.clone();
                    self.send_message(&channel, message).await
                }
                None => Err(anyhow!(
                    "Couldn't find channel name for channel \
                     id: {}",
                    channel
                )),
            };
        } else {
            let mut headers = HeaderMap::new();
            let json: Value = serde_json::from_str(&message)?;
            let username = match self.users.get(&username) {
                Some(user) => match &user.id {
                    Some(s) => s,
                    None => {
                        return Err(anyhow!(
                            "Couldn't find user id for \
                        user: {}",
                            username
                        ))
                    }
                },
                None => {
                    return Err(anyhow!(
                        "Couldn't find user {} in local \
                        cache",
                        username
                    ))
                }
            };
            let users = match json.as_array() {
                Some(users) => {
                    let users = users
                        .into_iter()
                        .filter_map(|user| {
                            user.as_str().map(|s| Element::RichTextSection {
                                elements: vec![Element::Text {
                                    text: s.to_string(),
                                    style: None,
                                }],
                            })
                        })
                        .collect();

                    Element::RichTextList {
                        elements: users,
                        style: "bullet".to_string(),
                        indent: 0,
                        border: None,
                    }
                }
                None => Element::Text {
                    text: "No users.".to_string(),
                    style: None,
                },
            };
            let blocks = vec![Block::RichText {
                //block_id: "response_block".to_string(),
                block_id: None,
                elements: vec![
                    Element::RichTextSection {
                        elements: vec![
                            Element::Text {
                                text: transport,
                                style: Some(Style {
                                    bold: Some(true),
                                    code: None,
                                    italic: None,
                                    strike: None,
                                }),
                            },
                            Element::Text {
                                text: "\n".to_string(),
                                style: None,
                            },
                        ],
                    },
                    users,
                ],
            }];
            let body = serde_json::json!({
            "channel":channel,
            "user":username,
            "blocks":blocks,
            })
            .to_string();

            headers.insert(header::CONTENT_TYPE, "application/json".parse()?);
            headers.insert(
                header::AUTHORIZATION,
                format!("Bearer {}", self.bot_token).parse()?,
            );

            self.http
                .request(Method::POST, "https://slack.com/api/chat.postEphemeral")
                .body(body)
                .send()
                .await?;

            return Ok(());
        }
    }

    async fn post_text_message(
        &mut self,
        pipo_id: i64,
        channel: &str,
        transport: String,
        username: String,
        avatar_url: Option<String>,
        thread: Option<(Option<String>, Option<u64>)>,
        message: Option<String>,
        attachments: Option<Vec<crate::Attachment>>,
        is_edit: bool,
    ) -> anyhow::Result<()> {
        let thread_ts = match thread {
            Some((_, d)) => match d {
                Some(d) => self.get_slackid_from_discordid(d).await?,
                None => None,
            },
            None => None,
        };

        if is_edit {
            if let Some(ts) = self.select_slackid_from_messages(pipo_id).await? {
                return self
                    .update(
                        ts,
                        channel,
                        transport,
                        username,
                        avatar_url,
                        message,
                        attachments,
                    )
                    .await;
            }
        }

        return self
            .post_message(
                pipo_id,
                channel,
                transport,
                username,
                avatar_url,
                thread_ts,
                message,
                attachments,
            )
            .await;
    }

    async fn post_message(
        &mut self,
        pipo_id: i64,
        channel: &str,
        transport: String,
        username: String,
        avatar_url: Option<String>,
        thread_ts: Option<String>,
        message: Option<String>,
        attachments: Option<Vec<crate::Attachment>>,
    ) -> anyhow::Result<()> {
        let mut headers = HeaderMap::new();
        let channel = match self.channel_map.get(channel) {
            Some(s) => s.clone(),
            None => return Err(anyhow!("Could not find id for channel {}", channel)),
        };
        let message = message.map(|s| self.insert_user_names(s));
        let icon_url = Slack::get_avatar_url(avatar_url);
        let username = format!("{} ({})", &username, transport);
        let attachments = match attachments {
            Some(a) => Some(self.prepare_attachments_for_slack(&channel, a).await),
            None => None,
        };
        let body = match thread_ts {
            Some(thread_ts) => serde_json::json!({
        "channel":channel,
        "text":message,
        "username":username,
        "icon_url":icon_url,
        "thread_ts":thread_ts,
        "attachments":attachments})
            .to_string(),
            None => serde_json::json!({
        "channel":channel,
        "text":message,
        "username":username,
        "icon_url":icon_url,
        "attachments":attachments})
            .to_string(),
        };

        headers.insert(header::CONTENT_TYPE, "application/json".parse()?);
        headers.insert(
            header::AUTHORIZATION,
            ("Bearer ".to_owned() + &self.bot_token).parse()?,
        );

        let response = self
            .http
            .request(Method::POST, "https://slack.com/api/chat.postMessage")
            .headers(headers)
            .body(body)
            .send()
            .await?;
        let json: Value = serde_json::from_str(response.text().await?.as_str())?;
        if json["ok"] == false {
            return Err(anyhow!(
                "E: slack.rs:Slack::post_message(): {}",
                json["error"]
            ));
        }

        if let Some(ts) = json["ts"].as_str() {
            self.update_messages(pipo_id, ts.to_string()).await?;
        }

        Ok(())
    }

    async fn remove_reaction(
        &mut self,
        pipo_id: i64,
        channel: &str,
        _transport: String,
        emoji: String,
        _username: Option<String>,
        _avatar_url: Option<String>,
        _thread: Option<(Option<String>, Option<u64>)>,
    ) -> anyhow::Result<()> {
        let mut headers = HeaderMap::new();
        let channel = match self.channel_map.get(channel) {
            Some(s) => s,
            None => return Err(anyhow!("Could not find id for channel {}", channel)),
        };
        let ts = match self.select_slackid_from_messages(pipo_id).await? {
            Some(ts) => ts,
            None => return Err(anyhow!("No slack_id for given pipo_id")),
        };

        let body = serde_json::json!({
            "name":emoji,
            "channel":channel,
            "timestamp":ts
        })
        .to_string();

        headers.insert(header::CONTENT_TYPE, "application/json".parse()?);
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.bot_token).parse()?,
        );

        let response = self
            .http
            .request(Method::POST, "https://slack.com/api/reactions.remove")
            .headers(headers)
            .body(body)
            .send()
            .await?;

        let json: Value = serde_json::from_str(response.text().await?.as_str())?;

        if json["ok"] == false {
            return Err(anyhow!(
                "E: slack.rs:Slack::remove_reaction(): {}",
                json["error"]
            ));
        }

        Ok(())
    }

    async fn update(
        &mut self,
        ts: String,
        channel: &str,
        transport: String,
        username: String,
        avatar_url: Option<String>,
        message: Option<String>,
        _attachments: Option<Vec<crate::Attachment>>,
    ) -> anyhow::Result<()> {
        let mut headers = HeaderMap::new();
        let channel = match self.channel_map.get(channel) {
            Some(s) => s,
            None => return Err(anyhow!("Could not find id for channel {}", channel)),
        };
        let message = message.map(|s| self.insert_user_names(s));
        let icon_url = Slack::get_avatar_url(avatar_url);
        let username = format!("{} ({})", &username, transport);
        let body = serde_json::json!({
        "channel":channel,
        "ts":ts,
        "text":message,
        "username":username,
        "icon_url":icon_url})
        .to_string();

        headers.insert(header::CONTENT_TYPE, "application/json".parse()?);
        headers.insert(
            header::AUTHORIZATION,
            ("Bearer ".to_owned() + &self.bot_token).parse()?,
        );

        let response = self
            .http
            .request(Method::POST, "https://slack.com/api/chat.update")
            .headers(headers)
            .body(body)
            .send()
            .await?;
        let json: Value = serde_json::from_str(response.text().await?.as_str())?;
        if json["ok"] == false {
            return Err(anyhow!("E: slack.rs:Slack::update(): {}", json["error"]));
        }

        Ok(())
    }

    fn insert_user_names(&self, message: String) -> String {
        lazy_static! {
            //static ref USERNAME_MATCH: Regex
            //= Regex::new(r#"(?=(?:^| )(\w.*)@)"#);
            // "[...]... disturbing" - Solra Bizna
            static ref USERNAME_MATCH: Regex
            = Regex::new(r#"@([A-Za-z0-9_ ]*)"#).unwrap();
        }

        USERNAME_MATCH
            .replace_all(&message, |caps: &Captures| {
                let mut username = &caps[1];
                let mut remainder = String::new();
                let mut run_loop = true;
                let mut first_run = true;

                while run_loop {
                    if !first_run {
                        username = match username.rsplit_once(' ') {
                            Some((left, right)) => {
                                remainder.insert_str(0, &format!(" {}", right));
                                left
                            }
                            None => {
                                run_loop = false;
                                username
                            }
                        };
                    } else {
                        first_run = false
                    }
                    if username.len() > 0 {
                        for (usern, user) in self.users.iter() {
                            if username == usern.as_str() {
                                if let Some(id) = &user.id {
                                    return format!("<@{}>{}", id, remainder);
                                }
                                break;
                            }
                        }
                    }
                }

                caps[0].to_string()
            })
            .to_string()
    }

    fn get_avatar_url(avatar_url: Option<String>) -> Option<String> {
        if let Some(mut avatar_url) = avatar_url {
            lazy_static! {
                static ref RE: Regex =
                    Regex::new(r#"^(http[s]?://(?:.*/)+.*\.)(webp)(\?.*)?$"#).unwrap();
            }

            if RE.is_match(&avatar_url) {
                avatar_url = RE
                    .replace_all(&avatar_url, |captures: &Captures| -> String {
                        if let Some(query) = captures.get(3) {
                            format!("{}png{}", &captures[1], query.as_str())
                        } else {
                            format!("{}png", &captures[1])
                        }
                    })
                    .to_string();
            }

            return Some(avatar_url);
        } else {
            return None;
        }
    }

    async fn get_user_display_name(&mut self, user: Option<String>) -> anyhow::Result<String> {
        let Some(user_id) = user else {
            return Ok(String::new());
        };

        if let Some(name) = self.user_id_map.get(&user_id) {
            return Ok(name.clone());
        }

        for user in self.users.values() {
            if user.id.as_deref() == Some(user_id.as_str()) {
                if let Some(name) = user
                    .profile
                    .as_ref()
                    .and_then(|profile| profile.display_name.as_ref())
                    .filter(|name| !name.is_empty())
                    .cloned()
                    .or_else(|| user.real_name.clone())
                    .or_else(|| user.name.clone())
                {
                    self.user_id_map.insert(user_id.clone(), name.clone());
                    return Ok(name);
                }
            }
        }

        let mut headers = HeaderMap::new();

        headers.insert(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse()?,
        );
        headers.insert(
            header::AUTHORIZATION,
            ("Bearer ".to_owned() + &self.bot_token).parse()?,
        );

        let url = reqwest::Url::parse_with_params(
            "https://slack.com/api/users.profile.get",
            &[("user", user_id.clone())],
        )?;

        let response = self
            .http
            .request(Method::GET, url)
            .headers(headers)
            .send()
            .await?;
        let json: Value = serde_json::from_str(response.text().await?.as_str())?;
        if json["ok"] == false {
            return Err(anyhow!("get_user_display_name() {:?}", json));
        }

        let display_name = match json["profile"].get("display_name").unwrap().as_str() {
            Some("") => json["profile"]
                .get("real_name")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
            Some(s) => s.to_string(),
            None => json["profile"]
                .get("real_name")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
        };

        self.user_id_map.insert(user_id, display_name.clone());

        Ok(display_name)
    }

    async fn get_users_list(&mut self) -> anyhow::Result<()> {
        let mut paginate = true;
        let mut cursor = String::new();
        let mut headers = HeaderMap::new();

        headers.insert(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse()?,
        );
        headers.insert(
            header::AUTHORIZATION,
            ("Bearer ".to_owned() + &self.bot_token).parse()?,
        );

        while paginate {
            let mut query = vec![("limit", "100")];
            if !cursor.is_empty() {
                query.push(("cursor", cursor.as_str()));
            }

            let url = reqwest::Url::parse_with_params("https://slack.com/api/users.list", &query)?;

            let users_list: UsersList = serde_json::from_str(
                self.http
                    .request(Method::GET, url)
                    .headers(headers.clone())
                    .send()
                    .await?
                    .text()
                    .await?
                    .as_str(),
            )?;

            if let Some(members) = users_list.members {
                for user in members {
                    if let Some(id) = user.id.clone() {
                        if let Some(display_name) = user
                            .profile
                            .as_ref()
                            .and_then(|profile| profile.display_name.as_ref())
                            .filter(|name| !name.is_empty())
                            .cloned()
                            .or_else(|| user.real_name.clone())
                            .or_else(|| user.name.clone())
                        {
                            self.user_id_map.insert(id, display_name);
                        }
                    }

                    if let Some(ref name) = user.name {
                        self.users.insert(name.to_string(), user);
                    } else if let Some(ref name) = user.real_name {
                        self.users.insert(name.to_string(), user);
                    }
                }
            }

            paginate = match users_list.response_metadata {
                Some(ResponseMetadata { next_cursor }) => {
                    if let Some(next_cursor) = next_cursor {
                        if next_cursor.is_empty() {
                            false
                        } else {
                            cursor = next_cursor;
                            true
                        }
                    } else {
                        false
                    }
                }
                None => false,
            }
        }

        Ok(())
    }

    async fn handle_response(&mut self, cid: usize, response: Response) -> anyhow::Result<()> {
        match response {
            Response::Disconnect {
                reason,
                debug_info: _,
            } => match reason.as_str() {
                "link_disabled" => Err(anyhow!("Socket Mode disabled")),
                "warning" | "refresh_requested" => {
                    let mut prev_sock = self
                        .websocket
                        .ws_stream
                        .remove(&cid)
                        .unwrap()
                        .reunite(self.websocket.ws_sink.take().unwrap())
                        .unwrap();
                    self.connect_websocket().await?;
                    prev_sock.close(None).await?;
                    Ok(())
                }
                _ => Ok(()),
            },
            Response::EventsApi {
                envelope_id,
                accepts_response_payload: _,
                retry_attempt,
                retry_reason: _,
                payload,
            } => {
                self.acknowledge(&envelope_id).await?;

                let event_id = match &payload {
                    EventPayload::EventCallback { event_id, .. } => Some(event_id.clone()),
                    EventPayload::UrlVerification { .. } => None,
                };

                let is_duplicate = retry_attempt > 0
                    && event_id
                        .as_ref()
                        .is_some_and(|id| self.seen_event_ids.contains(id));

                if let Err(error) = self
                    .log_inbound_event_payload(
                        &envelope_id,
                        event_id.as_deref(),
                        retry_attempt,
                        &payload,
                        is_duplicate,
                    )
                    .await
                {
                    eprintln!("Failed to log Slack inbound payload: {error}");
                }

                if retry_attempt > 0 {
                    if let Some(event_id) = event_id {
                        if self.seen_event_ids.contains(&event_id) {
                            return Ok(());
                        }

                        if self.seen_event_ids.len() == 50 {
                            self.seen_event_ids.pop_front();
                        }

                        self.seen_event_ids.push_back(event_id);
                    }
                }

                self.handle_event_payload(payload).await
            }
            Response::Hello {
                num_connections: _,
                debug_info: _,
                connection_info: _,
            } => Ok(()),
            Response::SlashCommands {
                envelope_id,
                accepts_response_payload,
                payload,
            } => {
                self.acknowledge(&envelope_id).await?;
                self.handle_slash_command_payload(accepts_response_payload, payload)
                    .await
            }
            Response::Unhandled { error } => Err(anyhow!("Unhandled response type: {}", error)),
        }
    }

    async fn log_inbound_event_payload(
        &self,
        envelope_id: &str,
        event_id: Option<&str>,
        retry_attempt: u64,
        payload: &EventPayload,
        is_duplicate: bool,
    ) -> anyhow::Result<()> {
        let envelope_id = envelope_id.to_string();
        let event_id = event_id.map(str::to_string);
        let payload_json = serde_json::to_string(payload)?;
        let payload_hash = format!("{:x}", Sha256::digest(payload_json.as_bytes()));
        let (event_type, channel_id, user_id, ts, thread_ts) =
            Self::event_payload_metadata(payload);

        self.pool
            .get()
            .await?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO slack_event_log (
                        received_at,
                        envelope_id,
                        event_id,
                        retry_attempt,
                        event_type,
                        channel_id,
                        user_id,
                        ts,
                        thread_ts,
                        payload_json,
                        payload_hash,
                        is_duplicate
                    ) VALUES (
                        strftime('%Y-%m-%d %H:%M:%f', 'now'),
                        ?,
                        ?,
                        ?,
                        ?,
                        ?,
                        ?,
                        ?,
                        ?,
                        ?,
                        ?,
                        ?
                    )",
                    params![
                        envelope_id,
                        event_id,
                        retry_attempt as i64,
                        event_type,
                        channel_id,
                        user_id,
                        ts,
                        thread_ts,
                        payload_json,
                        payload_hash,
                        is_duplicate as i64,
                    ],
                )
            })
            .await
            .map_err(|e| anyhow!("interact error: {e}"))??;

        Ok(())
    }

    fn event_payload_metadata(
        payload: &EventPayload,
    ) -> (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        match payload {
            EventPayload::EventCallback {
                event, event_id: _, ..
            } => Self::event_metadata(event),
            EventPayload::UrlVerification { .. } => {
                (Some("url_verification".to_string()), None, None, None, None)
            }
        }
    }

    fn event_metadata(
        event: &Event,
    ) -> (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        match event {
            Event::Message(message) => (
                Some("message".to_string()),
                message.channel.clone(),
                message.user.clone(),
                message.ts.clone(),
                message.thread_ts.clone(),
            ),
            Event::PinAdded {
                channel_id,
                user,
                event_ts,
                ..
            } => (
                Some("pin_added".to_string()),
                Some(channel_id.clone()),
                user.clone(),
                Some(event_ts.clone()),
                None,
            ),
            Event::PinRemoved {
                channel_id,
                user,
                event_ts,
                ..
            } => (
                Some("pin_removed".to_string()),
                Some(channel_id.clone()),
                user.clone(),
                Some(event_ts.clone()),
                None,
            ),
            Event::ReactionAdded { user, event_ts, .. } => (
                Some("reaction_added".to_string()),
                None,
                user.clone(),
                Some(event_ts.0.clone()),
                None,
            ),
            Event::ReactionRemoved { user, event_ts, .. } => (
                Some("reaction_removed".to_string()),
                None,
                user.clone(),
                Some(event_ts.0.clone()),
                None,
            ),
        }
    }

    async fn handle_event_payload(&mut self, payload: EventPayload) -> anyhow::Result<()> {
        match payload {
            EventPayload::EventCallback {
                token: _,
                team_id: _,
                api_app_id: _,
                event,
                event_id,
                event_time: _,
                authorizations: _,
                is_ext_shared_channel: _,
                event_context: _,
            } => self.handle_event(event, false, Some(event_id)).await,
            EventPayload::UrlVerification {
                token: _,
                challenge: _,
            } => return Err(anyhow!("UrlVerification is not handled.")),
        }
    }

    async fn handle_slash_command_payload(
        &mut self,
        accepts_response: bool,
        payload: SlashCommandPayload,
    ) -> anyhow::Result<()> {
        match payload.command.as_str() {
            "/names" => {
                if accepts_response {
                    let message = Message::Names {
                        sender: self.transport_id,
                        transport: TRANSPORT_NAME.to_string(),
                        username: payload.user_name,
                        message: Some("/names".to_string()),
                    };
                    let channel = &format!("#{}", payload.channel_name);
                    if let Some(sender) = self.channels.get(channel) {
                        if let Err(e) = sender.send(message) {
                            eprintln!("Couldn't send message: {:#}", e);
                        }
                    }
                }
            }
            default => eprintln!("Unhandled slash command: {}", default),
        }

        Ok(())
    }

    #[async_recursion]
    async fn handle_event(
        &mut self,
        event: Event,
        is_edit: bool,
        event_id: Option<String>,
    ) -> anyhow::Result<()> {
        match event {
            Event::Message(SlackMessage {
                subtype,
                hidden,
                message,
                bot_id: _,
                client_msg_id: _,
                text,
                files,
                upload: _,
                user,
                display_as_bot: _,
                ts,
                deleted_ts,
                team: _,
                attachments,
                blocks,
                channel,
                previous_message: prev_message,
                event_ts: _,
                thread_ts,
                channel_type: _,
                edited,
            }) => {
                let channel_id = channel;
                let irc_flag = match edited {
                    Some(_) => false,
                    None => true,
                };
                let hidden = match hidden {
                    Some(b) => b,
                    None => false,
                };
                let channel = match channel_id.as_ref() {
                    Some(channel) => match self.id_map.get(&channel) {
                        Some(channel) => channel.to_owned(),
                        None => {
                            return Err(anyhow!(
                                "Channel name not found \
                            for channel id: {}",
                                channel
                            ))
                        }
                    },
                    None => {
                        return Err(anyhow!(
                            "Message does not contain a \
                        channel."
                        ))
                    }
                };

                let (rich_text, source_mode) = if let Some(blocks) = blocks {
                    let mut rich_text = String::new();
                    for block in blocks.iter() {
                        match block {
                            Block::RichText {
                                block_id: _,
                                elements,
                            } => {
                                for element in elements.iter() {
                                    let fragment = self.convert_element_to_string(element).await?;
                                    Slack::append_rich_text_fragment(&mut rich_text, &fragment);
                                }
                            }
                            _ => continue,
                        }
                    }

                    (Some(rich_text), "blocks_rich_text")
                } else {
                    (
                        match text {
                            Some(text) => Some(self.parse_usernames(&text).await?),
                            None => None,
                        },
                        "plain_text_fallback",
                    )
                };

                if let Err(error) = self
                    .log_rendered_event(
                        event_id.as_deref(),
                        ts.as_deref(),
                        source_mode,
                        rich_text.as_deref(),
                        attachments.as_ref().map_or(0, Vec::len),
                        subtype.as_deref(),
                        irc_flag,
                        is_edit,
                        channel_id.as_deref(),
                        Some(channel.as_str()),
                    )
                    .await
                {
                    eprintln!("Failed to log rendered Slack message: {error}");
                }

                match subtype {
                    Some(subtype) => match subtype.as_str() {
                        "bot_add" => return Ok(()),
                        "bot_message" => {
                            return self
                                .handle_bot_message(
                                    ts,
                                    thread_ts,
                                    &channel,
                                    rich_text,
                                    attachments,
                                    hidden,
                                    is_edit,
                                )
                                .await
                        }
                        "file_share" => {
                            return self
                                .handle_file_share(
                                    ts,
                                    thread_ts,
                                    &channel,
                                    user,
                                    rich_text,
                                    files,
                                    attachments,
                                    is_edit,
                                )
                                .await
                        }
                        "me_message" => {
                            return self
                                .handle_me_message(ts, &channel, user, rich_text, is_edit, irc_flag)
                                .await
                        }
                        "message_changed" => {
                            return self
                                .handle_message_changed(
                                    ts,
                                    thread_ts,
                                    &channel,
                                    message,
                                    prev_message,
                                    hidden,
                                    event_id,
                                )
                                .await
                        }
                        "message_deleted" => {
                            return self.handle_message_deleted(deleted_ts, &channel).await
                        }
                        _ => {
                            return self
                                .handle_message(
                                    ts,
                                    thread_ts,
                                    &channel,
                                    user,
                                    rich_text,
                                    attachments,
                                    is_edit,
                                    irc_flag,
                                )
                                .await
                        }
                    },
                    None => {
                        return self
                            .handle_message(
                                ts,
                                thread_ts,
                                &channel,
                                user,
                                rich_text,
                                attachments,
                                is_edit,
                                irc_flag,
                            )
                            .await
                    }
                }
            }
            Event::PinAdded {
                channel_id: _,
                item,
                pin_count: _,
                pinned_info: _,
                event_ts: _,
                user: _,
            } => return self.handle_pin(item, false).await,
            Event::PinRemoved {
                channel_id: _,
                item,
                pin_count: _,
                pinned_info: _,
                has_pins: _,
                event_ts: _,
                user: _,
            } => return self.handle_pin(item, true).await,
            Event::ReactionAdded {
                event_ts: _,
                item,
                reaction,
                user,
                item_user: _,
            } => return self.handle_reaction(item, reaction, user, false).await,
            Event::ReactionRemoved {
                event_ts: _,
                item,
                reaction,
                user,
                item_user: _,
            } => return self.handle_reaction(item, reaction, user, true).await,
        }
    }

    async fn handle_bot_message(
        &mut self,
        ts: Option<String>,
        thread_ts: Option<String>,
        channel: &str,
        message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        _hidden: bool,
        is_edit: bool,
    ) -> anyhow::Result<()> {
        let _thread = match thread_ts {
            Some(ts) => Some((
                Some(ts.clone()),
                self.select_discordid_from_messages(ts).await?,
            )),
            None => None,
        };
        let pipo_id = match ts {
            Some(ts) => match self.select_id_from_messages(&ts).await {
                Some(id) => id,
                None => self.insert_into_messages_table(&ts).await?,
            },
            None => return Err(anyhow!("Message has no timestamp.")),
        };
        let attachments = match attachments {
            Some(attachments) => Some(self.handle_attachments(attachments).await),
            None => None,
        };

        let message = Message::Bot {
            sender: self.transport_id,
            pipo_id,
            transport: TRANSPORT_NAME.to_string(),
            message: message,
            attachments,
            is_edit,
        };

        return self.send_message(channel, message).await;
    }

    async fn handle_file_share(
        &mut self,
        ts: Option<String>,
        thread_ts: Option<String>,
        channel: &str,
        user: Option<String>,
        message: Option<String>,
        files: Option<Vec<File>>,
        attachments: Option<Vec<Attachment>>,
        is_edit: bool,
    ) -> anyhow::Result<()> {
        let mut message = message;
        let mut file_urls = String::new();
        let mut is_first_line = true;

        if let Some(files) = files {
            for file in files {
                let file_url = file.permalink_public;

                if is_first_line {
                    file_urls.push_str(&format!("{}", file_url));
                    is_first_line = false;
                } else {
                    file_urls.push_str(&format!("\n{}", file_url));
                }
            }

            message = Some(match message {
                Some(s) => format!("{}\n{}", s, file_urls),
                None => file_urls,
            });
        }

        self.handle_message(
            ts,
            thread_ts,
            channel,
            user,
            message,
            attachments,
            is_edit,
            false,
        )
        .await
    }

    fn get_username(user: &User) -> anyhow::Result<String> {
        Ok(user
            .profile
            .as_ref()
            .and_then(|p| p.display_name.as_ref().filter(|s| !s.is_empty()))
            .or_else(|| user.name.as_ref().filter(|s| !s.is_empty()))
            .or_else(|| user.real_name.as_ref().filter(|s| !s.is_empty()))
            .ok_or_else(|| anyhow!("Couldn't get a name for user."))?
            .to_string())
    }

    fn get_avatar_url_for_user(user: &User) -> anyhow::Result<Option<String>> {
        Ok(user
            .profile
            .as_ref()
            .and_then(|p| {
                if let Some(url) = &p.image_original {
                    Some(url)
                } else if let Some(url) = &p.image_1024 {
                    Some(url)
                } else if let Some(url) = &p.image_512 {
                    Some(url)
                } else if let Some(url) = &p.image_192 {
                    Some(url)
                } else if let Some(url) = &p.image_72 {
                    Some(url)
                } else if let Some(url) = &p.image_48 {
                    Some(url)
                } else if let Some(url) = &p.image_32 {
                    Some(url)
                } else if let Some(url) = &p.image_24 {
                    Some(url)
                } else {
                    None
                }
            })
            .cloned())
    }

    async fn handle_me_message(
        &mut self,
        ts: Option<String>,
        channel: &str,
        user: Option<String>,
        message: Option<String>,
        is_edit: bool,
        irc_flag: bool,
    ) -> anyhow::Result<()> {
        let pipo_id = match ts {
            Some(ts) => match self.select_id_from_messages(&ts).await {
                Some(id) => id,
                None => self.insert_into_messages_table(&ts).await?,
            },
            None => return Err(anyhow!("Message has no timestamp.")),
        };
        let user = self
            .get_user_info(&user.ok_or_else(|| anyhow!("No user ID in message."))?)
            .await?;
        let username = Slack::get_username(&user)?;
        let avatar_url = Slack::get_avatar_url_for_user(&user)?;
        let message = Message::Action {
            sender: self.transport_id,
            pipo_id,
            transport: TRANSPORT_NAME.to_string(),
            username,
            avatar_url,
            thread: None,
            message: message,
            attachments: None,
            is_edit,
            irc_flag,
        };

        return self.send_message(channel, message).await;
    }

    async fn handle_message_changed(
        &mut self,
        _ts: Option<String>,
        _thread_ts: Option<String>,
        channel: &str,
        message: Option<Box<Event>>,
        _previous_message: Option<Box<Event>>,
        hidden: bool,
        event_id: Option<String>,
    ) -> anyhow::Result<()> {
        if message.is_none() {
            return Err(anyhow!("No updated message present in event."));
        }

        let msg = *message.unwrap();
        let channel = match self.channel_map.get(channel) {
            Some(s) => s,
            None => return Err(anyhow!("Could not find id for channel {}", channel)),
        };

        let event = match msg {
            Event::Message(SlackMessage {
                subtype,
                hidden: _,
                message,
                bot_id,
                client_msg_id,
                text,
                files,
                upload,
                user,
                display_as_bot,
                ts,
                deleted_ts,
                team,
                attachments,
                blocks,
                channel: _,
                previous_message,
                event_ts,
                thread_ts,
                channel_type,
                edited,
            }) => Event::Message(SlackMessage {
                channel: Some(String::from(channel)),
                hidden: Some(hidden),
                subtype,
                message,
                bot_id,
                client_msg_id,
                text,
                files,
                upload,
                user,
                display_as_bot,
                ts,
                deleted_ts,
                team,
                attachments,
                blocks,
                previous_message,
                event_ts,
                thread_ts,
                channel_type,
                edited,
            }),
            _ => return Err(anyhow!("message not an Event::Message")),
        };

        self.handle_event(event, true, event_id).await
    }

    async fn log_rendered_event(
        &self,
        event_id: Option<&str>,
        ts: Option<&str>,
        source_mode: &str,
        rendered_text: Option<&str>,
        attachment_count: usize,
        subtype: Option<&str>,
        irc_flag: bool,
        is_edit: bool,
        channel_id: Option<&str>,
        channel_name: Option<&str>,
    ) -> anyhow::Result<()> {
        let event_id = event_id.map(str::to_string);
        let ts = ts.map(str::to_string);
        let source_mode = source_mode.to_string();
        let rendered_text = rendered_text.map(str::to_string);
        let subtype = subtype.map(str::to_string);
        let channel_id = channel_id.map(str::to_string);
        let channel_name = channel_name.map(str::to_string);

        self.pool
            .get()
            .await?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO slack_render_log (
                        rendered_at,
                        event_id,
                        ts,
                        source_mode,
                        rendered_text,
                        attachment_count,
                        subtype,
                        irc_flag,
                        is_edit,
                        channel_id,
                        channel_name
                    ) VALUES (
                        strftime('%Y-%m-%d %H:%M:%f', 'now'),
                        ?,
                        ?,
                        ?,
                        ?,
                        ?,
                        ?,
                        ?,
                        ?,
                        ?,
                        ?
                    )",
                    params![
                        event_id,
                        ts,
                        source_mode,
                        rendered_text,
                        attachment_count as i64,
                        subtype,
                        irc_flag as i64,
                        is_edit as i64,
                        channel_id,
                        channel_name,
                    ],
                )
            })
            .await
            .map_err(|e| anyhow!("interact error: {e}"))??;

        Ok(())
    }

    async fn handle_message_deleted(
        &mut self,
        ts: Option<String>,
        channel: &str,
    ) -> anyhow::Result<()> {
        let pipo_id = match ts {
            Some(ts) => match self.select_id_from_messages(&ts).await {
                Some(id) => id,
                None => self.insert_into_messages_table(&ts).await?,
            },
            None => return Err(anyhow!("Message has no timestamp.")),
        };

        let message = Message::Delete {
            sender: self.transport_id,
            pipo_id,
            transport: TRANSPORT_NAME.to_string(),
        };

        return self.send_message(channel, message).await;
    }

    async fn insert_into_messages_table(&self, ts: &str) -> anyhow::Result<i64> {
        let conn = self.pool.get().await.unwrap();
        let pipo_id = *self.pipo_id.lock().unwrap();
        let ts = String::from(ts);

        // TODO: ugly error handling needs fixing
        conn.interact(move |conn| -> anyhow::Result<usize> {
            Ok(conn.execute(
                "INSERT OR REPLACE INTO messages (id, slackid) 
                                 VALUES (?1, ?2)",
                params![pipo_id, Some(ts.clone())],
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

    async fn update_messages(&self, pipo_id: i64, ts: String) -> anyhow::Result<()> {
        let conn = self.pool.get().await.unwrap();

        // TODO: ugly error handling needs fixing
        conn.interact(move |conn| -> anyhow::Result<usize> {
            Ok(conn.execute(
                "UPDATE messages SET slackid = ?2 
                                 WHERE id = ?1",
                params![pipo_id, Some(ts.clone())],
            )?)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("Interact Error")))?;

        Ok(())
    }

    #[allow(dead_code)]
    fn debug_print_messages(conn: &rusqlite::Connection) -> anyhow::Result<()> {
        #[derive(Debug)]
        struct MessageId {
            id: i64,
            slack_id: Option<String>,
            discord_id: Option<i64>,
        }

        let mut stmt = conn.prepare("SELECT id, slackid, discordid FROM messages")?;
        let message_iter = stmt.query_map([], |row| {
            Ok(MessageId {
                id: row.get(0)?,
                slack_id: row.get(1)?,
                discord_id: row.get(2)?,
            })
        })?;

        for message in message_iter {
            println!("Found message {:?}", message.unwrap());
        }

        Ok(())
    }

    async fn select_id_from_messages(&self, ts: &str) -> Option<i64> {
        let conn = self.pool.get().await.unwrap();
        let old_ts = ts;
        let ts = String::from(old_ts);

        // TODO: ugly error handling needs fixing
        let ret = match conn
            .interact(move |conn| -> anyhow::Result<i64> {
                // Slack::debug_print_messages(&conn)?;

                Ok(conn.query_row(
                    "SELECT id FROM messages WHERE slackid = ?1",
                    params![ts],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap_or_else(|_| Err(anyhow!("Interact Error")))
        {
            Ok(id) => Some(id),
            Err(e) => {
                eprintln!("Error#: {}", e);
                None
            }
        };

        ret
    }

    async fn select_slackid_from_messages(&self, pipo_id: i64) -> anyhow::Result<Option<String>> {
        let conn = self.pool.get().await.unwrap();

        // TODO: ugly error handling needs fixing
        let ret = conn
            .interact(move |conn| -> anyhow::Result<Option<String>> {
                Ok(conn.query_row(
                    "SELECT slackid FROM messages WHERE id = ?1",
                    params![pipo_id],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap_or_else(|_| Err(anyhow!("Interact Error")))?;

        Ok(ret)
    }

    async fn get_slackid_from_discordid(&self, discord_id: u64) -> anyhow::Result<Option<String>> {
        let conn = self.pool.get().await.unwrap();

        // TODO: ugly error handling needs fixing
        let ret = conn
            .interact(move |conn| -> anyhow::Result<Option<String>> {
                Ok(conn.query_row(
                    "SELECT slackid FROM messages 
                               WHERE discordid = ?1",
                    params![discord_id],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap_or_else(|_| Err(anyhow!("Interact Error")))?;

        Ok(ret)
    }

    async fn select_discordid_from_messages(
        &self,
        slack_id: String,
    ) -> anyhow::Result<Option<u64>> {
        let conn = self.pool.get().await.unwrap();

        // TODO: ugly error handling needs fixing
        let ret = conn
            .interact(move |conn| -> anyhow::Result<Option<u64>> {
                Ok(conn.query_row(
                    "SELECT discordid FROM messages 
                               WHERE slackid = ?1",
                    params![slack_id],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap_or_else(|_| Err(anyhow!("Interact Error")))?;

        Ok(ret)
    }

    async fn handle_message(
        &mut self,
        ts: Option<String>,
        thread_ts: Option<String>,
        channel: &str,
        user: Option<String>,
        message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        mut is_edit: bool,
        mut irc_flag: bool,
    ) -> anyhow::Result<()> {
        let has_message = message.is_some();
        let has_attachments = attachments.is_some();
        let thread = match thread_ts {
            Some(ts) => Some((Some(ts.clone()), None)),
            None => None,
        };
        let ts = ts.ok_or_else(|| anyhow!("Message has no timestamp."))?;
        let pipo_id = match self.select_id_from_messages(&ts).await {
            Some(id) => id,
            None => {
                if is_edit {
                    is_edit = false;
                    irc_flag = false;
                }
                self.insert_into_messages_table(&ts).await?
            }
        };
        let user = self
            .get_user_info(&user.ok_or_else(|| anyhow!("No user ID in message."))?)
            .await?;
        let username = Slack::get_username(&user)?;
        let avatar_url = Slack::get_avatar_url_for_user(&user)?;
        let attachments = match attachments {
            Some(attachments) => Some(self.handle_attachments(attachments).await),
            None => None,
        };

        if has_message || has_attachments {
            let message = Message::Text {
                pipo_id,
                sender: self.transport_id,
                transport: TRANSPORT_NAME.to_string(),
                username,
                avatar_url,
                thread,
                message: message,
                attachments,
                is_edit,
                irc_flag,
            };

            return self.send_message(channel, message).await;
        } else {
            return Err(anyhow!(
                "Message from {} on channel {} has no content",
                username,
                channel
            ));
        }
    }

    async fn handle_pin(&mut self, item: Box<Item>, remove: bool) -> anyhow::Result<()> {
        match *item {
            Item::Message {
                created: _,
                created_by: _,
                channel,
                message,
            } => match message {
                Event::Message(SlackMessage {
                    subtype: _,
                    hidden: _,
                    message: _,
                    bot_id: _,
                    client_msg_id: _,
                    text: _,
                    files: _,
                    upload: _,
                    user: _,
                    display_as_bot: _,
                    ts,
                    deleted_ts: _,
                    team: _,
                    attachments: _,
                    blocks: _,
                    channel: _,
                    previous_message: _,
                    event_ts: _,
                    thread_ts: _,
                    channel_type: _,
                    edited: _,
                }) => {
                    let channel = match self.id_map.get(&channel) {
                        Some(c) => c.to_string(),
                        None => {
                            return Err(anyhow!(
                                "Couldn't find channel \
                            name for channel_id: \
                            {}",
                                channel
                            ))
                        }
                    };
                    let pipo_id = match ts {
                        Some(ts) => match self.select_id_from_messages(&ts).await {
                            Some(id) => id,
                            None => {
                                return Err(anyhow!(
                                    "No pipo_id for \
                            timestamp."
                                ))
                            }
                        },
                        None => {
                            return Err(anyhow!(
                                "Pinned item has no \
                        timestamp."
                            ))
                        }
                    };

                    let message = Message::Pin {
                        sender: self.transport_id,
                        pipo_id,
                        remove,
                    };

                    return self.send_message(&channel, message).await;
                }
                _ => return Err(anyhow!("Unhandled pin item")),
            },
        }
    }

    async fn handle_reaction(
        &mut self,
        item: Box<Event>,
        reaction: String,
        user: Option<String>,
        remove: bool,
    ) -> anyhow::Result<()> {
        match *item {
            Event::Message(SlackMessage {
                subtype: _,
                hidden: _,
                message: _,
                bot_id: _,
                client_msg_id: _,
                text: _,
                files: _,
                upload: _,
                user: _,
                display_as_bot: _,
                ts,
                deleted_ts: _,
                team: _,
                attachments: _,
                blocks: _,
                channel,
                previous_message: _,
                event_ts: _,
                thread_ts: _,
                channel_type: _,
                edited: _,
            }) => {
                let channel = match channel {
                    Some(c) => match self.id_map.get(&c) {
                        Some(c) => c.to_string(),
                        None => {
                            return Err(anyhow!(
                                "Couldn't find channel \
                            name for channel_id: {}",
                                c
                            ))
                        }
                    },
                    None => return Err(anyhow!("Item has no channel")),
                };
                let pipo_id = match ts {
                    Some(ts) => match self.select_id_from_messages(&ts).await {
                        Some(id) => id,
                        None => return Err(anyhow!("No pipo_id for timestamp.")),
                    },
                    None => return Err(anyhow!("Reaction has no timestamp.")),
                };

                let message = Message::Reaction {
                    sender: self.transport_id,
                    pipo_id,
                    transport: TRANSPORT_NAME.to_string(),
                    emoji: reaction,
                    remove,
                    username: user,
                    avatar_url: None,
                    thread: None,
                };

                return self.send_message(&channel, message).await;
            }
            _ => return Err(anyhow!("Unhandled reaction item")),
        }
    }

    async fn handle_attachments(&mut self, attachments: Vec<Attachment>) -> Vec<crate::Attachment> {
        let mut ret = Vec::new();
        let mut id = 0;

        for attachment in attachments {
            let pipo_id = match attachment.ts {
                Some(ts) => self.select_id_from_messages(&ts.0).await,
                None => None,
            };

            ret.push(crate::Attachment {
                id,
                pipo_id,
                service_name: attachment.service_name.clone(),
                service_url: attachment.service_url.clone(),
                author_name: attachment.author_name.clone(),
                author_subname: attachment.author_subname.clone(),
                author_link: attachment.author_link.clone(),
                author_icon: attachment.author_icon.clone(),
                from_url: attachment.from_url.clone(),
                original_url: attachment.image_url.clone(),
                footer: attachment.footer.clone(),
                footer_icon: attachment.footer_icon.clone(),
                text: attachment.text.clone(),
                image_url: attachment.image_url.clone(),
                image_bytes: attachment.image_bytes.clone(),
                image_height: attachment.image_height.clone(),
                image_width: attachment.image_width.clone(),
                fallback: attachment.fallback.clone(),
                ..Default::default()
            });

            id += 1;
        }

        ret
    }

    async fn prepare_attachments_for_slack(
        &self,
        channel: &str,
        attachments: Vec<crate::Attachment>,
    ) -> Vec<Attachment> {
        let mut ret = Vec::new();

        for attachment in attachments {
            let ts = match attachment.pipo_id {
                Some(id) => match self.select_slackid_from_messages(id).await {
                    Ok(ts) => ts.map(|s| Timestamp(s)),
                    Err(_) => None,
                },
                None => None,
            };

            // Only fields we get from this are:
            // (*) client_msg_id
            // (*) text
            // (*) user (id)
            // (*) ts
            // (*) team
            // (*) attachments
            // (*) blocks
            // (*) maybe subtype?
            let message = match ts {
                Some(ref ts) => self.get_message(channel, ts).await.ok(),
                None => None,
            };

            if let Some(message) = message {
                let mut author_name = None;
                let mut author_icon = None;
                let footer = self.id_map.get(channel).map(|s| format!("Posted in {}", s));
                let _timestamp = ts.as_ref().unwrap().0.replace(".", "");
                let author_link = match message.user {
                    Some(ref u) => Some(format!("{}", u)),
                    None => None,
                };
                let user = match message.user {
                    Some(ref u) => self.get_user_info(&u).await.ok(),
                    None => None,
                };

                if let Some(user) = user {
                    if let Some(profile) = user.profile {
                        author_name = match profile.display_name {
                            Some(s) => match s.len() {
                                0 => user.name,
                                _ => Some(s),
                            },
                            None => user.name,
                        };
                        author_icon = profile.image_48;
                    }
                }

                if author_name.is_none() {
                    author_name = attachment.author_name;
                }
                if author_icon.is_none() {
                    author_icon = Slack::get_avatar_url(attachment.author_icon);
                }

                ret.push(Attachment {
                    mrkdwn_in: Some(vec![format!("text")]),
                    color: Some(String::from("#D0D0D0")),
                    author_name,
                    author_link,
                    author_icon,
                    text: message.text,
                    footer,
                    ts,
                    ..Default::default()
                });
            } else {
                ret.push(Attachment {
                    id: Some(attachment.id),
                    ts,
                    service_name: attachment.service_name.clone(),
                    service_url: attachment.service_url.clone(),
                    author_name: attachment.author_name.clone(),
                    author_subname: attachment.author_subname.clone(),
                    author_link: attachment.author_link.clone(),
                    author_icon: attachment.author_icon.clone(),
                    from_url: attachment.from_url.clone(),
                    footer: attachment.footer.clone(),
                    footer_icon: attachment.footer_icon.clone(),
                    text: attachment.text.clone(),
                    image_url: attachment.image_url.clone(),
                    image_bytes: attachment.image_bytes.clone(),
                    image_height: attachment.image_height.clone(),
                    image_width: attachment.image_width.clone(),
                    fallback: attachment.fallback.clone(),
                    ..Default::default()
                });
            }
        }

        ret
    }

    async fn send_message(&mut self, channel: &str, message: Message) -> anyhow::Result<()> {
        match self.channels.get(channel) {
            Some(bus) => {
                if let Err(e) = bus.send(message) {
                    return Err(anyhow!(
                        "Couldn't send message to channel {}: \
                    {:#}",
                        channel,
                        e
                    ));
                }
            }
            None => return Err(anyhow!("No bus for channel {}", channel)),
        }

        Ok(())
    }

    #[allow(dead_code)]
    async fn make_file_public(&self, file_id: &str) -> anyhow::Result<()> {
        let mut headers = HeaderMap::new();

        headers.insert(header::CONTENT_TYPE, "application/json".parse()?);
        headers.insert(
            header::AUTHORIZATION,
            ("Bearer ".to_owned() + &self.bot_token).parse()?,
        );

        let body = serde_json::json!({"file":file_id}).to_string();

        let response = self
            .http
            .request(Method::POST, "https://slack.com/api/files.sharedPublicURL")
            .headers(headers)
            .body(body)
            .send()
            .await?;
        let json: Value = serde_json::from_str(response.text().await?.as_str())?;
        if json["ok"] == false {
            return Err(anyhow!("get_user_display_name() {:?}", json));
        }

        Ok(())
    }

    async fn convert_element_to_string(&mut self, element: &Element) -> anyhow::Result<String> {
        rich_text_renderer::render(
            self,
            element,
            RenderOptions {
                irc_formatting_enabled: self.irc_formatting_enabled,
            },
        )
        .await
    }

    async fn parse_usernames(&mut self, text: &str) -> anyhow::Result<String> {
        lazy_static! {
            static ref ENTITY_RE: Regex = Regex::new("<([^>]+)>").unwrap();
        }

        let mut output = String::with_capacity(text.len());
        let mut last = 0;

        for captures in ENTITY_RE.captures_iter(text) {
            let full = captures.get(0).unwrap();
            let inner = captures.get(1).unwrap().as_str();

            output.push_str(&text[last..full.start()]);
            last = full.end();

            let replacement = match parse_entity_reference(inner) {
                EntityReference::User { id, label } => {
                    let label = match label.filter(|label| !label.is_empty()) {
                        Some(label) => Some(label),
                        None => self.get_user_display_name(Some(id.clone())).await.ok(),
                    };
                    format_user(label.as_deref(), &id)
                }
                EntityReference::Channel { id, label } => {
                    let channel_name = label.or_else(|| self.id_map.get(&id).cloned());
                    format_channel(channel_name.as_deref(), &id)
                }
                EntityReference::Team { id, label } => {
                    format_team(label.as_deref().filter(|label| !label.is_empty()), &id)
                }
                EntityReference::Usergroup { id, label } => {
                    format_usergroup(label.as_deref().filter(|label| !label.is_empty()), &id)
                }
                EntityReference::Broadcast { range, .. } => format_broadcast(&range),
                EntityReference::Link { url, label } => format_link(&url, label.as_deref()),
                EntityReference::Unknown { raw } => format!("<{raw}>"),
            };

            output.push_str(&replacement);
        }

        output.push_str(&text[last..]);

        Ok(output)
    }

    async fn get_message(&self, channel: &str, ts: &Timestamp) -> anyhow::Result<SlackMessage> {
        let mut headers = HeaderMap::new();
        let _form = Form::new()
            .text("channel", channel.to_string())
            .text("latest", ts.0.clone())
            .text("limit", "1");

        headers.insert(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse()?,
        );
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.bot_token).parse()?,
        );

        let url = reqwest::Url::parse_with_params(
            "https://slack.com/api/conversations.history",
            &[
                ("channel", channel),
                ("inclusive", "true"),
                ("latest", ts.0.as_str()),
                ("limit", "1"),
            ],
        )?;

        let response = self
            .http
            .request(Method::GET, url)
            .headers(headers)
            .send()
            .await?;

        let json: Value = serde_json::from_str(response.text().await?.as_str())?;

        if json["ok"] == false {
            return Err(anyhow!("get_message() {:?}", json));
        }

        if let Some(messages) = json["messages"].as_array() {
            let message: SlackMessage = serde_json::from_value(messages[0].clone())?;

            return Ok(message);
        }

        return Err(anyhow!(
            "No message found with timestamp {} in channel {}",
            ts.0,
            channel
        ));
    }

    async fn get_user_info(&self, user: &str) -> anyhow::Result<User> {
        let mut headers = HeaderMap::new();

        headers.insert(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse()?,
        );
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.bot_token).parse()?,
        );

        let url =
            reqwest::Url::parse_with_params("https://slack.com/api/users.info", &[("user", user)])?;

        let response = self
            .http
            .request(Method::GET, url)
            .headers(headers)
            .send()
            .await?;

        let json: Value = serde_json::from_str(response.text().await?.as_str())?;

        if json["ok"] == false {
            return Err(anyhow!("get_message() {:?}", json));
        }

        let user: User = serde_json::from_value(json["user"].clone())?;

        return Ok(user);
    }

    fn parse_special_chars(text: &str) -> String {
        let mut text_chars = text.chars();
        let mut text = String::new();
        while let Some(c) = text_chars.next() {
            match c {
                '<' => {
                    let mut is_url = true;
                    let mut substring = String::new();
                    while let Some(c) = text_chars.next() {
                        match c {
                            '>' => {
                                if is_url {
                                    substring.push('>')
                                }
                                break;
                            }
                            '|' => {
                                is_url = false;
                                substring.clear();
                            }
                            c => substring.push(c),
                        }
                    }
                    if is_url {
                        text.push('<')
                    }
                    text.push_str(&substring);
                }
                '&' => match text_chars.next() {
                    Some('a') => {
                        while let Some(c) = text_chars.next() {
                            if c == ';' {
                                break;
                            }
                        }
                        text.push('&');
                    }
                    Some('l') => {
                        while let Some(c) = text_chars.next() {
                            if c == ';' {
                                break;
                            }
                        }
                        text.push('<');
                    }
                    Some('g') => {
                        while let Some(c) = text_chars.next() {
                            if c == ';' {
                                break;
                            }
                        }
                        text.push('>');
                    }
                    Some(_) => unreachable!(),
                    None => break,
                },
                c => text.push(c),
            }
        }
        text
    }
}

#[serenity::async_trait]
impl RichTextResolver for Slack {
    async fn resolve_user_display_name(&mut self, user_id: &str) -> anyhow::Result<String> {
        self.get_user_display_name(Some(user_id.to_string())).await
    }

    fn resolve_channel_name(&self, channel_id: &str) -> Option<String> {
        self.id_map.get(channel_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadpool_sqlite::{Config, Runtime};
    use std::time::Duration;
    use tokio::time::timeout;

    fn append_fragments(fragments: &[&str]) -> String {
        let mut out = String::new();
        for fragment in fragments {
            Slack::append_rich_text_fragment(&mut out, fragment);
        }
        out
    }

    async fn make_test_slack() -> (Slack, broadcast::Receiver<Message>) {
        let pool = Config::new(":memory:")
            .create_pool(Runtime::Tokio1)
            .expect("pool");

        pool.get()
            .await
            .expect("conn")
            .interact(|conn| {
                conn.execute_batch(
                    "CREATE TABLE messages (
                        id        INTEGER PRIMARY KEY,
                        slackid   TEXT,
                        discordid INTEGER,
                        modtime   TEXT
                    );
                    CREATE TABLE slack_render_log (
                        id                INTEGER PRIMARY KEY,
                        rendered_at       TEXT,
                        event_id          TEXT,
                        ts                TEXT,
                        source_mode       TEXT,
                        rendered_text     TEXT,
                        attachment_count  INTEGER,
                        subtype           TEXT,
                        irc_flag          INTEGER,
                        is_edit           INTEGER,
                        channel_id        TEXT,
                        channel_name      TEXT
                    );",
                )
            })
            .await
            .expect("interact")
            .expect("create messages table");

        let (sender, receiver) = broadcast::channel(8);
        let mut bus_map = HashMap::new();
        bus_map.insert("main".to_string(), sender);

        let mut channel_mapping: HashMap<Arc<String>, Arc<String>> = HashMap::new();
        channel_mapping.insert(Arc::new("#test".to_string()), Arc::new("main".to_string()));

        let mut slack = Slack::new(
            1,
            &bus_map,
            Arc::new(Mutex::new(1)),
            pool,
            "xapp-token".to_string(),
            "xoxb-token".to_string(),
            true,
            &channel_mapping,
        )
        .await
        .expect("slack");

        slack.id_map.insert("C123".to_string(), "#test".to_string());

        (slack, receiver)
    }

    #[test]
    fn append_rich_text_fragment_adds_newline_only_at_boundaries() {
        let joined = append_fragments(&["> q1", "line2"]);
        assert_eq!(joined, "> q1\nline2");

        let already_separated = append_fragments(&["> q1\n", "line2"]);
        assert_eq!(already_separated, "> q1\nline2");

        let leading_newline_fragment = append_fragments(&["> q1", "\nline2"]);
        assert_eq!(leading_newline_fragment, "> q1\nline2");

        let inline_only = append_fragments(&["hello", " world"]);
        assert_eq!(inline_only, "hello\n world");
    }

    #[tokio::test]
    async fn handle_event_rich_text_quote_then_section_has_boundary_newline() {
        let (mut slack, mut receiver) = make_test_slack().await;

        let event = Event::Message(SlackMessage {
            subtype: Some("bot_message".to_string()),
            hidden: Some(false),
            message: None,
            bot_id: None,
            client_msg_id: None,
            text: None,
            files: None,
            upload: None,
            user: None,
            display_as_bot: None,
            ts: Some("1700000000.100001".to_string()),
            deleted_ts: None,
            team: None,
            attachments: None,
            blocks: Some(vec![Block::RichText {
                block_id: None,
                elements: vec![
                    Element::RichTextQuote {
                        elements: vec![Element::Text {
                            text: "q1".to_string(),
                            style: None,
                        }],
                    },
                    Element::RichTextSection {
                        elements: vec![Element::Text {
                            text: "line2".to_string(),
                            style: None,
                        }],
                    },
                ],
            }]),
            channel: Some("C123".to_string()),
            previous_message: None,
            event_ts: None,
            thread_ts: None,
            channel_type: None,
            edited: None,
        });

        slack
            .handle_event(event, false, None)
            .await
            .expect("handle event");

        let outbound = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("recv timeout")
            .expect("recv message");

        match outbound {
            Message::Bot { message, .. } => {
                assert_eq!(message.as_deref(), Some("> q1\nline2"));
            }
            other => panic!("unexpected outbound message: {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_event_rich_text_list_sequence_preserves_boundaries() {
        let (mut slack, mut receiver) = make_test_slack().await;

        let event = Event::Message(SlackMessage {
            subtype: Some("bot_message".to_string()),
            hidden: Some(false),
            message: None,
            bot_id: None,
            client_msg_id: None,
            text: None,
            files: None,
            upload: None,
            user: None,
            display_as_bot: None,
            ts: Some("1700000000.100002".to_string()),
            deleted_ts: None,
            team: None,
            attachments: None,
            blocks: Some(vec![Block::RichText {
                block_id: None,
                elements: vec![
                    Element::RichTextList {
                        elements: vec![Element::RichTextSection {
                            elements: vec![Element::Text {
                                text: "item 1".to_string(),
                                style: None,
                            }],
                        }],
                        style: "bullet".to_string(),
                        indent: 0,
                        border: None,
                    },
                    Element::RichTextList {
                        elements: vec![Element::RichTextSection {
                            elements: vec![Element::Text {
                                text: "nested 1".to_string(),
                                style: None,
                            }],
                        }],
                        style: "ordered".to_string(),
                        indent: 1,
                        border: None,
                    },
                    Element::RichTextList {
                        elements: vec![Element::RichTextSection {
                            elements: vec![Element::Text {
                                text: "item 2".to_string(),
                                style: None,
                            }],
                        }],
                        style: "bullet".to_string(),
                        indent: 0,
                        border: None,
                    },
                ],
            }]),
            channel: Some("C123".to_string()),
            previous_message: None,
            event_ts: None,
            thread_ts: None,
            channel_type: None,
            edited: None,
        });

        slack
            .handle_event(event, false, None)
            .await
            .expect("handle event");

        let outbound = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("recv timeout")
            .expect("recv message");

        match outbound {
            Message::Bot { message, .. } => {
                assert_eq!(
                    message.as_deref(),
                    Some("- item 1\n  1. nested 1\n- item 2")
                );
            }
            other => panic!("unexpected outbound message: {other:?}"),
        }
    }
}
