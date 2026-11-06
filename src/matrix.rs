//! Matrix transport.
//!
//! PIPO acts as a single Matrix Application Service (appservice) that bridges
//! every other configured service. Because one bridge can register a user
//! namespace regex that spans all of those services (e.g. `@pipo_.*`), remote
//! senders are puppeted as distinct "ghost" Matrix users
//! (`@pipo_irc_alice`, `@pipo_slack_bob`, ...) rather than relayed through a
//! single bot account.
//!
//! Inbound events are pushed to us by the homeserver over HTTP
//! (`PUT /_matrix/app/v1/transactions/{txnId}`); we host that listener with
//! `axum`. Outbound sends use the Client-Server API via `reqwest`, masquerading
//! as the relevant ghost with the appservice `?user_id=` query parameter and
//! the `as_token` as the access token. The event *content* is built and parsed
//! with `ruma`'s community-maintained types.
//!
//! v1 scope (both directions): plain text, edits (`m.replace`), deletes
//! (redactions), and reactions (`m.reaction`). Attachments and threads are
//! deferred; the code marks where they would hook in.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use anyhow::{anyhow, Context};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::put,
    Json, Router,
};
use deadpool_sqlite::Pool;
use reqwest::{Client as HttpClient, Url};
use ruma::{
    events::{
        reaction::ReactionEventContent,
        relation::Annotation,
        room::message::{ReplacementMetadata, RoomMessageEventContent},
    },
    EventId,
};
use rusqlite::params;
use serde_json::{json, Value};
use tokio::{net::TcpListener, sync::broadcast};
use tokio_stream::{wrappers::BroadcastStream, StreamMap};

use crate::Message;

const TRANSPORT_NAME: &'static str = "Matrix";

pub(crate) struct Matrix {
    inner: Arc<MatrixInner>,
}

struct MatrixInner {
    transport_id: usize,
    http: HttpClient,
    /// Base client-server API URL with no trailing slash.
    homeserver_url: String,
    server_name: String,
    as_token: String,
    hs_token: String,
    /// The appservice's own user id, e.g. "@pipo:example.org".
    sender_user_id: String,
    sender_localpart: String,
    ghost_prefix: String,
    listen_addr: String,
    /// Matrix room id -> the bus this transport publishes inbound events to.
    room_to_bus: HashMap<String, broadcast::Sender<Message>>,
    pool: Pool,
    pipo_id: Arc<Mutex<i64>>,
    /// Ghost localparts we've already registered.
    registered_ghosts: Mutex<HashSet<String>>,
    /// (user_id, room_id) pairs we've already joined.
    joined: Mutex<HashSet<(String, String)>>,
    /// Ghost user ids whose displayname we've already set this run.
    displaynames_set: Mutex<HashSet<String>>,
    /// Cache of inbound sender user id -> displayname (for nicer usernames).
    displayname_cache: Mutex<HashMap<String, String>>,
    /// Inbound transaction ids already processed (idempotency).
    seen_txns: Mutex<HashSet<String>>,
    /// (pipo_id, ghost_user_id, emoji_key) -> reaction event id, so an outbound
    /// reaction removal can redact the reaction event we previously sent.
    reaction_events: Mutex<HashMap<(i64, String, String), String>>,
    /// Monotonic counter for outbound transaction ids.
    txn_counter: AtomicU64,
}

impl Matrix {
    pub async fn new(
        transport_id: usize,
        bus_map: &HashMap<String, broadcast::Sender<Message>>,
        pipo_id: Arc<Mutex<i64>>,
        pool: Pool,
        homeserver: String,
        use_tls: bool,
        server_name: String,
        as_token: String,
        hs_token: String,
        sender_localpart: String,
        ghost_prefix: String,
        listen_addr: String,
        channel_mapping: &HashMap<Arc<String>, Arc<String>>,
    ) -> anyhow::Result<Matrix> {
        let room_to_bus = channel_mapping
            .iter()
            .filter_map(|(room_id, busname)| {
                if let Some(sender) = bus_map.get(busname.as_ref()) {
                    Some((room_id.as_ref().clone(), sender.clone()))
                } else {
                    eprintln!("No bus named '{}' in configuration file.", busname);
                    None
                }
            })
            .collect();

        let scheme = if use_tls { "https" } else { "http" };
        let homeserver_url = format!("{}://{}", scheme, homeserver.trim_end_matches('/'));
        let sender_user_id = format!("@{}:{}", sender_localpart, server_name);

        let inner = MatrixInner {
            transport_id,
            http: HttpClient::new(),
            homeserver_url,
            server_name,
            as_token,
            hs_token,
            sender_user_id,
            sender_localpart,
            ghost_prefix,
            listen_addr,
            room_to_bus,
            pool,
            pipo_id,
            registered_ghosts: Mutex::new(HashSet::new()),
            joined: Mutex::new(HashSet::new()),
            displaynames_set: Mutex::new(HashSet::new()),
            displayname_cache: Mutex::new(HashMap::new()),
            seen_txns: Mutex::new(HashSet::new()),
            reaction_events: Mutex::new(HashMap::new()),
            txn_counter: AtomicU64::new(0),
        };

        Ok(Matrix {
            inner: Arc::new(inner),
        })
    }

    pub async fn connect(&mut self) -> anyhow::Result<()> {
        let inner = self.inner.clone();

        // Ensure the appservice's own user exists (best effort).
        if let Err(e) = inner.register_user(&inner.sender_localpart).await {
            eprintln!("Matrix: couldn't register sender user: {:#}", e);
        }

        // Inbound: the homeserver PUTs transactions to us.
        let app = Router::new()
            .route(
                "/_matrix/app/v1/transactions/{txn_id}",
                put(handle_transaction),
            )
            // Legacy unstable path still used by some homeservers.
            .route("/transactions/{txn_id}", put(handle_transaction))
            .with_state(inner.clone());
        let listener = TcpListener::bind(inner.listen_addr.as_str())
            .await
            .with_context(|| format!("Matrix: couldn't bind {}", inner.listen_addr))?;
        eprintln!("Matrix: appservice listening on {}", inner.listen_addr);
        let mut server = tokio::spawn(async move { axum::serve(listener, app).await });

        // Outbound: multiplex every mapped bus, keyed by destination room id.
        let mut input_buses: StreamMap<String, BroadcastStream<Message>> = StreamMap::new();
        for (room_id, sender) in inner.room_to_bus.iter() {
            input_buses.insert(room_id.clone(), BroadcastStream::new(sender.subscribe()));
        }

        loop {
            tokio::select! {
                Some((room_id, message))
                    = tokio_stream::StreamExt::next(&mut input_buses) => {
                    match message {
                        Ok(message) => inner.handle_outbound(&room_id, message).await,
                        Err(e) => eprintln!("Matrix: bus stream error: {}", e),
                    }
                }
                res = &mut server => {
                    return match res {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(e)) => Err(anyhow!("Matrix inbound server error: {}", e)),
                        Err(e) => Err(anyhow!("Matrix inbound server task failed: {}", e)),
                    };
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Outbound: bus Message -> Matrix Client-Server API
// ---------------------------------------------------------------------------

impl MatrixInner {
    async fn handle_outbound(&self, room_id: &str, message: Message) {
        match message {
            Message::Text {
                sender,
                pipo_id,
                transport,
                username,
                thread: _,
                message,
                attachments: _,
                is_edit,
                irc_flag: _,
                ..
            } => {
                if sender != self.transport_id {
                    self.send_user_message(
                        room_id, pipo_id, &transport, &username, message, is_edit, false,
                    )
                    .await;
                }
            }
            Message::Action {
                sender,
                pipo_id,
                transport,
                username,
                thread: _,
                message,
                attachments: _,
                is_edit,
                irc_flag: _,
                ..
            } => {
                if sender != self.transport_id {
                    self.send_user_message(
                        room_id, pipo_id, &transport, &username, message, is_edit, true,
                    )
                    .await;
                }
            }
            Message::Bot {
                sender,
                pipo_id,
                transport: _,
                message,
                attachments: _,
                is_edit,
            } => {
                if sender != self.transport_id && !is_edit {
                    self.send_bot_message(room_id, pipo_id, message).await;
                }
            }
            Message::Delete {
                sender,
                pipo_id,
                transport: _,
            } => {
                if sender != self.transport_id {
                    self.handle_outbound_delete(room_id, pipo_id).await;
                }
            }
            Message::Reaction {
                sender,
                pipo_id,
                transport: _,
                emoji,
                remove,
                username,
                thread: _,
                ..
            } => {
                if sender != self.transport_id {
                    self.handle_outbound_reaction(room_id, pipo_id, &emoji, remove, username)
                        .await;
                }
            }
            // Not represented in Matrix rooms for v1.
            Message::Names { .. } | Message::Pin { .. } => {}
        }
    }

    /// Send (or edit) a text/emote message puppeted as the sender's ghost.
    async fn send_user_message(
        &self,
        room_id: &str,
        pipo_id: i64,
        transport: &str,
        username: &str,
        message: Option<String>,
        is_edit: bool,
        emote: bool,
    ) {
        let message = match message {
            Some(m) if !m.is_empty() => m,
            _ => return,
        };

        let ghost = self.ghost_user_id(transport, username);
        if let Err(e) = self.ensure_ghost(&ghost, username).await {
            eprintln!("Matrix: couldn't prepare ghost {}: {:#}", ghost, e);
            return;
        }
        if let Err(e) = self.ensure_joined(&ghost, room_id).await {
            eprintln!("Matrix: ghost {} couldn't join {}: {:#}", ghost, room_id, e);
            return;
        }

        let content = if is_edit {
            // Resolve the original Matrix event to replace.
            match self.select_matrixid_from_messages(pipo_id).await {
                Some(orig) => match EventId::parse(&orig) {
                    Ok(orig_id) => {
                        let new = if emote {
                            RoomMessageEventContent::emote_plain(message)
                        } else {
                            RoomMessageEventContent::text_plain(message)
                        };
                        new.make_replacement(ReplacementMetadata::new(orig_id, None))
                    }
                    Err(_) => return,
                },
                // Nothing to edit on the Matrix side; drop it.
                None => return,
            }
        } else if emote {
            RoomMessageEventContent::emote_plain(message)
        } else {
            RoomMessageEventContent::text_plain(message)
        };
        // Attachments/threads would be attached to `content` here in a later
        // revision (media upload + `m.image`/`m.file`, `m.thread` relations).

        let content = match serde_json::to_value(&content) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Matrix: couldn't serialize message content: {:#}", e);
                return;
            }
        };

        match self
            .send_event(Some(&ghost), room_id, "m.room.message", content)
            .await
        {
            Ok(event_id) => {
                if !is_edit {
                    // Remember pipo_id <-> Matrix event id so later edits,
                    // deletes and reactions can find this message.
                    if let Err(e) = self.update_messages_matrixid(pipo_id, Some(event_id)).await {
                        eprintln!("Matrix: couldn't store matrixid: {:#}", e);
                    }
                }
            }
            Err(e) => eprintln!("Matrix: send to {} failed: {:#}", room_id, e),
        }
    }

    /// Send a bot/system notice as the appservice's own user.
    async fn send_bot_message(&self, room_id: &str, pipo_id: i64, message: Option<String>) {
        let message = match message {
            Some(m) if !m.is_empty() => m,
            _ => return,
        };
        if let Err(e) = self.ensure_joined(&self.sender_user_id, room_id).await {
            eprintln!("Matrix: sender user couldn't join {}: {:#}", room_id, e);
            return;
        }
        let content = match serde_json::to_value(RoomMessageEventContent::notice_plain(message)) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Matrix: couldn't serialize notice content: {:#}", e);
                return;
            }
        };
        // `None` user_id => acts as the appservice sender user.
        match self.send_event(None, room_id, "m.room.message", content).await {
            Ok(event_id) => {
                if let Err(e) = self.update_messages_matrixid(pipo_id, Some(event_id)).await {
                    eprintln!("Matrix: couldn't store matrixid: {:#}", e);
                }
            }
            Err(e) => eprintln!("Matrix: bot send to {} failed: {:#}", room_id, e),
        }
    }

    async fn handle_outbound_delete(&self, room_id: &str, pipo_id: i64) {
        let event_id = match self.select_matrixid_from_messages(pipo_id).await {
            Some(id) => id,
            None => return,
        };
        // Redact as the appservice sender user, which is expected to hold a
        // moderator power level in bridged rooms.
        if let Err(e) = self.redact(None, room_id, &event_id).await {
            eprintln!("Matrix: redact in {} failed: {:#}", room_id, e);
        }
    }

    async fn handle_outbound_reaction(
        &self,
        room_id: &str,
        pipo_id: i64,
        emoji: &str,
        remove: bool,
        username: Option<String>,
    ) {
        let key = self.emoji_to_key(emoji);
        // React as the reactor's ghost when a username is known, so that
        // distinct users' identical reactions don't collide. The origin
        // transport isn't carried on a reaction, so the ghost omits the
        // service segment (e.g. `@pipo_alice`). Without a username we react as
        // the appservice sender user.
        let actor = username
            .as_deref()
            .map(|u| self.ghost_user_id("", u));
        let actor_key = actor.clone().unwrap_or_default();

        if remove {
            let stored = {
                let map = self.reaction_events.lock().unwrap();
                map.get(&(pipo_id, actor_key, key)).cloned()
            };
            if let Some(reaction_event) = stored {
                if let Err(e) = self.redact(actor.as_deref(), room_id, &reaction_event).await {
                    eprintln!("Matrix: reaction redact failed: {:#}", e);
                }
            }
            return;
        }

        let target = match self.select_matrixid_from_messages(pipo_id).await {
            Some(id) => id,
            None => return,
        };
        let target_id = match EventId::parse(&target) {
            Ok(id) => id,
            Err(_) => return,
        };

        if let (Some(actor), Some(username)) = (actor.as_deref(), username.as_deref()) {
            if let Err(e) = self.ensure_ghost(actor, username).await {
                eprintln!("Matrix: couldn't prepare reacting ghost {}: {:#}", actor, e);
                return;
            }
            if let Err(e) = self.ensure_joined(actor, room_id).await {
                eprintln!("Matrix: reacting ghost couldn't join: {:#}", e);
                return;
            }
        }

        let content = match serde_json::to_value(ReactionEventContent::new(Annotation::new(
            target_id,
            key.clone(),
        ))) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Matrix: couldn't serialize reaction: {:#}", e);
                return;
            }
        };
        match self.send_event(actor.as_deref(), room_id, "m.reaction", content).await {
            Ok(reaction_event) => {
                let mut map = self.reaction_events.lock().unwrap();
                map.insert((pipo_id, actor_key, key), reaction_event);
            }
            Err(e) => eprintln!("Matrix: reaction send failed: {:#}", e),
        }
    }

    // -- ghost / membership management --------------------------------------

    async fn ensure_ghost(&self, user_id: &str, display: &str) -> anyhow::Result<()> {
        let localpart = localpart_of(user_id).unwrap_or(user_id).to_string();
        let already = {
            let set = self.registered_ghosts.lock().unwrap();
            set.contains(&localpart)
        };
        if !already {
            self.register_user(&localpart).await?;
            self.registered_ghosts.lock().unwrap().insert(localpart);
        }

        let display_done = {
            let set = self.displaynames_set.lock().unwrap();
            set.contains(user_id)
        };
        if !display_done {
            if let Err(e) = self.set_displayname(user_id, display).await {
                eprintln!("Matrix: couldn't set displayname for {}: {:#}", user_id, e);
            } else {
                self.displaynames_set
                    .lock()
                    .unwrap()
                    .insert(user_id.to_string());
            }
        }
        Ok(())
    }

    async fn ensure_joined(&self, user_id: &str, room_id: &str) -> anyhow::Result<()> {
        let key = (user_id.to_string(), room_id.to_string());
        {
            let set = self.joined.lock().unwrap();
            if set.contains(&key) {
                return Ok(());
            }
        }
        self.join_room(user_id, room_id).await?;
        self.joined.lock().unwrap().insert(key);
        Ok(())
    }

    async fn register_user(&self, localpart: &str) -> anyhow::Result<()> {
        let url = self.client_url(&["register"])?;
        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.as_token)
            .json(&json!({
                "type": "m.login.application_service",
                "username": localpart,
            }))
            .send()
            .await?;
        if resp.status().is_success() {
            return Ok(());
        }
        // A user that already exists is fine.
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if body.contains("M_USER_IN_USE") {
            return Ok(());
        }
        Err(anyhow!("register {} -> {}: {}", localpart, status, body))
    }

    async fn set_displayname(&self, user_id: &str, name: &str) -> anyhow::Result<()> {
        let mut url = self.client_url(&["profile", user_id, "displayname"])?;
        url.query_pairs_mut().append_pair("user_id", user_id);
        let resp = self
            .http
            .put(url)
            .bearer_auth(&self.as_token)
            .json(&json!({ "displayname": name }))
            .send()
            .await?;
        expect_success(resp, "set_displayname").await
    }

    async fn join_room(&self, user_id: &str, room_id: &str) -> anyhow::Result<()> {
        let mut url = self.client_url(&["join", room_id])?;
        if user_id != self.sender_user_id {
            url.query_pairs_mut().append_pair("user_id", user_id);
        }
        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.as_token)
            .json(&json!({}))
            .send()
            .await?;
        expect_success(resp, "join_room").await
    }

    async fn send_event(
        &self,
        user_id: Option<&str>,
        room_id: &str,
        event_type: &str,
        content: Value,
    ) -> anyhow::Result<String> {
        let txn = self.next_txn();
        let mut url = self.client_url(&["rooms", room_id, "send", event_type, &txn])?;
        if let Some(user_id) = user_id {
            url.query_pairs_mut().append_pair("user_id", user_id);
        }
        let resp = self
            .http
            .put(url)
            .bearer_auth(&self.as_token)
            .json(&content)
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("send {} -> {}: {}", event_type, status, body));
        }
        let value: Value = serde_json::from_str(&body).unwrap_or_default();
        value
            .get("event_id")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| anyhow!("send response missing event_id: {}", body))
    }

    async fn redact(
        &self,
        user_id: Option<&str>,
        room_id: &str,
        event_id: &str,
    ) -> anyhow::Result<String> {
        let txn = self.next_txn();
        let mut url = self.client_url(&["rooms", room_id, "redact", event_id, &txn])?;
        if let Some(user_id) = user_id {
            url.query_pairs_mut().append_pair("user_id", user_id);
        }
        let resp = self
            .http
            .put(url)
            .bearer_auth(&self.as_token)
            .json(&json!({}))
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("redact -> {}: {}", status, body));
        }
        let value: Value = serde_json::from_str(&body).unwrap_or_default();
        Ok(value
            .get("event_id")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_default())
    }

    // -- helpers ------------------------------------------------------------

    fn client_url(&self, segments: &[&str]) -> anyhow::Result<Url> {
        let mut url = Url::parse(&self.homeserver_url).context("invalid homeserver_url")?;
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| anyhow!("homeserver_url cannot be a base"))?;
            path.pop_if_empty();
            path.extend(["_matrix", "client", "v3"]);
            path.extend(segments);
        }
        Ok(url)
    }

    fn next_txn(&self) -> String {
        let n = self.txn_counter.fetch_add(1, Ordering::Relaxed);
        format!("pipo{}", n)
    }

    fn ghost_user_id(&self, transport: &str, username: &str) -> String {
        let svc = sanitize_localpart(transport);
        let user = sanitize_localpart(username);
        if svc.is_empty() {
            format!("@{}{}:{}", self.ghost_prefix, user, self.server_name)
        } else {
            format!("@{}{}_{}:{}", self.ghost_prefix, svc, user, self.server_name)
        }
    }

    /// True if `user_id` belongs to this appservice (a ghost or the sender
    /// user), i.e. an event we authored and should not re-bridge.
    fn is_our_user(&self, user_id: &str) -> bool {
        if user_id == self.sender_user_id {
            return true;
        }
        match localpart_of(user_id) {
            Some(local) => local.starts_with(self.ghost_prefix.as_str()),
            None => false,
        }
    }

    fn emoji_to_key(&self, emoji: &str) -> String {
        emojis::get_by_shortcode(emoji)
            .map(|e| e.as_str().to_string())
            .unwrap_or_else(|| emoji.to_string())
    }

    async fn display_name_for(&self, user_id: &str) -> String {
        if let Some(name) = self.displayname_cache.lock().unwrap().get(user_id).cloned() {
            return name;
        }
        let fallback = localpart_of(user_id).unwrap_or(user_id).to_string();
        let name = match self.client_url(&["profile", user_id, "displayname"]) {
            Ok(url) => match self.http.get(url).bearer_auth(&self.as_token).send().await {
                Ok(resp) if resp.status().is_success() => resp
                    .json::<Value>()
                    .await
                    .ok()
                    .and_then(|v| v.get("displayname").and_then(Value::as_str).map(String::from))
                    .filter(|s| !s.is_empty())
                    .unwrap_or(fallback),
                _ => fallback,
            },
            Err(_) => fallback,
        };
        self.displayname_cache
            .lock()
            .unwrap()
            .insert(user_id.to_string(), name.clone());
        name
    }

    // -- SQLite id mapping (mirrors the ircid helpers in src/irc.rs) --------

    async fn insert_into_messages_table(&self) -> anyhow::Result<i64> {
        let conn = self.pool.get().await.unwrap();
        let pipo_id = *self.pipo_id.lock().unwrap();

        match conn
            .interact(move |conn| -> anyhow::Result<usize> {
                Ok(conn.execute(
                    "INSERT OR REPLACE INTO messages (id) VALUES (?1)",
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

    async fn update_messages_matrixid(
        &self,
        pipo_id: i64,
        matrix_event_id: Option<String>,
    ) -> anyhow::Result<()> {
        let conn = self.pool.get().await.unwrap();

        conn.interact(move |conn| -> anyhow::Result<usize> {
            Ok(conn.execute(
                "UPDATE messages SET matrixid = ?2 WHERE id = ?1",
                params![pipo_id, matrix_event_id],
            )?)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("Interact Error")))?;

        Ok(())
    }

    async fn select_matrixid_from_messages(&self, pipo_id: i64) -> Option<String> {
        let conn = self.pool.get().await.unwrap();

        conn.interact(move |conn| -> anyhow::Result<Option<String>> {
            Ok(conn.query_row(
                "SELECT matrixid FROM messages WHERE id = ?1",
                params![pipo_id],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("Interact Error")))
        .ok()
        .flatten()
    }

    async fn select_pipo_id_by_matrixid(&self, matrix_event_id: String) -> Option<i64> {
        let conn = self.pool.get().await.unwrap();

        conn.interact(move |conn| -> anyhow::Result<Option<i64>> {
            Ok(conn.query_row(
                "SELECT id FROM messages WHERE matrixid = ?1",
                params![matrix_event_id],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("Interact Error")))
        .ok()
        .flatten()
    }
}

// ---------------------------------------------------------------------------
// Inbound: homeserver transaction -> bus Message
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct QueryParams {
    access_token: Option<String>,
}

async fn handle_transaction(
    State(inner): State<Arc<MatrixInner>>,
    Path(txn_id): Path<String>,
    headers: HeaderMap,
    Query(params): Query<QueryParams>,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    // Authenticate the homeserver via the hs_token (bearer header preferred,
    // legacy access_token query parameter accepted).
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    let presented = bearer.or(params.access_token);
    if presented.as_deref() != Some(inner.hs_token.as_str()) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "errcode": "M_FORBIDDEN" })),
        );
    }

    // Idempotency: the homeserver retries transactions until it gets a 200.
    {
        let mut seen = inner.seen_txns.lock().unwrap();
        if seen.contains(&txn_id) {
            return (StatusCode::OK, Json(json!({})));
        }
        // Keep the set from growing without bound.
        if seen.len() > 10_000 {
            seen.clear();
        }
        seen.insert(txn_id);
    }

    let transaction: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Matrix: couldn't parse transaction: {:#}", e);
            // Ack anyway so the homeserver stops retrying a malformed body.
            return (StatusCode::OK, Json(json!({})));
        }
    };

    if let Some(events) = transaction.get("events").and_then(Value::as_array) {
        for event in events {
            inner.handle_inbound_event(event).await;
        }
    }

    (StatusCode::OK, Json(json!({})))
}

impl MatrixInner {
    async fn handle_inbound_event(&self, event: &Value) {
        let event_type = event.get("type").and_then(Value::as_str).unwrap_or_default();
        let sender = event.get("sender").and_then(Value::as_str).unwrap_or_default();
        let room_id = event.get("room_id").and_then(Value::as_str).unwrap_or_default();
        let event_id = event.get("event_id").and_then(Value::as_str).unwrap_or_default();

        // Loop prevention: never re-bridge events we authored.
        if sender.is_empty() || self.is_our_user(sender) {
            return;
        }
        // Only bridge rooms we're configured for.
        if !self.room_to_bus.contains_key(room_id) {
            return;
        }

        match event_type {
            "m.room.message" => self.inbound_message(room_id, sender, event_id, event).await,
            "m.reaction" => self.inbound_reaction(room_id, sender, event).await,
            "m.room.redaction" => self.inbound_redaction(room_id, event).await,
            _ => {}
        }
    }

    async fn inbound_message(&self, room_id: &str, sender: &str, event_id: &str, event: &Value) {
        let content = match event.get("content") {
            Some(c) => c,
            None => return,
        };
        let msgtype = content.get("msgtype").and_then(Value::as_str).unwrap_or("m.text");

        // Is this an edit (m.replace)?
        let relates = content.get("m.relates_to");
        let is_replace = relates
            .and_then(|r| r.get("rel_type"))
            .and_then(Value::as_str)
            == Some("m.replace");

        let username = self.display_name_for(sender).await;

        if is_replace {
            let target = relates
                .and_then(|r| r.get("event_id"))
                .and_then(Value::as_str);
            let new_body = content
                .get("m.new_content")
                .and_then(|c| c.get("body"))
                .and_then(Value::as_str);
            let (Some(target), Some(new_body)) = (target, new_body) else {
                return;
            };
            let pipo_id = match self.select_pipo_id_by_matrixid(target.to_string()).await {
                Some(id) => id,
                // We don't know the original message; ignore the edit.
                None => return,
            };
            let message = self.build_text_message(pipo_id, msgtype, username, new_body, true);
            self.publish(room_id, message);
            return;
        }

        let body = match content.get("body").and_then(Value::as_str) {
            Some(b) if !b.is_empty() => b,
            _ => return,
        };

        let pipo_id = match self.insert_into_messages_table().await {
            Ok(id) => id,
            Err(e) => {
                eprintln!("Matrix: couldn't allocate pipo_id: {:#}", e);
                return;
            }
        };
        if let Err(e) = self
            .update_messages_matrixid(pipo_id, Some(event_id.to_string()))
            .await
        {
            eprintln!("Matrix: couldn't store inbound matrixid: {:#}", e);
        }

        let message = self.build_text_message(pipo_id, msgtype, username, body, false);
        self.publish(room_id, message);
    }

    fn build_text_message(
        &self,
        pipo_id: i64,
        msgtype: &str,
        username: String,
        body: &str,
        is_edit: bool,
    ) -> Message {
        let common = (
            self.transport_id,
            pipo_id,
            TRANSPORT_NAME.to_string(),
            username,
            Some(body.to_string()),
            is_edit,
        );
        let (sender, pipo_id, transport, username, message, is_edit) = common;
        if msgtype == "m.emote" {
            Message::Action {
                sender,
                pipo_id,
                transport,
                username,
                avatar_url: None,
                thread: None,
                message,
                attachments: None,
                is_edit,
                irc_flag: false,
            }
        } else {
            Message::Text {
                sender,
                pipo_id,
                transport,
                username,
                avatar_url: None,
                thread: None,
                message,
                attachments: None,
                is_edit,
                irc_flag: false,
            }
        }
    }

    async fn inbound_reaction(&self, room_id: &str, sender: &str, event: &Value) {
        let relates = match event.get("content").and_then(|c| c.get("m.relates_to")) {
            Some(r) => r,
            None => return,
        };
        let target = relates.get("event_id").and_then(Value::as_str);
        let key = relates.get("key").and_then(Value::as_str);
        let (Some(target), Some(key)) = (target, key) else {
            return;
        };
        let pipo_id = match self.select_pipo_id_by_matrixid(target.to_string()).await {
            Some(id) => id,
            None => return,
        };
        let username = self.display_name_for(sender).await;
        let emoji = key_to_emoji(key);
        self.publish(
            room_id,
            Message::Reaction {
                sender: self.transport_id,
                pipo_id,
                transport: TRANSPORT_NAME.to_string(),
                emoji,
                remove: false,
                username: Some(username),
                avatar_url: None,
                thread: None,
            },
        );
    }

    async fn inbound_redaction(&self, room_id: &str, event: &Value) {
        // `redacts` is top-level in older room versions and in `content` in
        // newer ones.
        let redacts = event
            .get("redacts")
            .and_then(Value::as_str)
            .or_else(|| event.get("content").and_then(|c| c.get("redacts")).and_then(Value::as_str));
        let redacts = match redacts {
            Some(r) => r,
            None => return,
        };
        let pipo_id = match self.select_pipo_id_by_matrixid(redacts.to_string()).await {
            Some(id) => id,
            None => return,
        };
        self.publish(
            room_id,
            Message::Delete {
                sender: self.transport_id,
                pipo_id,
                transport: TRANSPORT_NAME.to_string(),
            },
        );
    }

    fn publish(&self, room_id: &str, message: Message) {
        if let Some(sender) = self.room_to_bus.get(room_id) {
            if let Err(e) = sender.send(message) {
                eprintln!("Matrix: couldn't publish to bus for {}: {:#}", room_id, e);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

/// Extract the localpart of a Matrix user id (`@local:server` -> `local`).
fn localpart_of(user_id: &str) -> Option<&str> {
    user_id
        .strip_prefix('@')
        .and_then(|rest| rest.split(':').next())
}

/// Map an arbitrary display name/service name to a valid Matrix localpart
/// fragment: lowercase, keeping only `[a-z0-9._=/-]`, other characters become
/// `_`.
fn sanitize_localpart(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '.' | '_' | '=' | '-' | '/' => c,
            _ => '_',
        })
        .collect()
}

/// Convert a Matrix reaction key (usually a unicode emoji) to a shortcode name
/// when we recognise it, matching how other transports refer to emoji.
fn key_to_emoji(key: &str) -> String {
    emojis::get(key)
        .and_then(|e| e.shortcode())
        .map(str::to_string)
        .unwrap_or_else(|| key.to_string())
}

async fn expect_success(resp: reqwest::Response, what: &str) -> anyhow::Result<()> {
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let body = resp.text().await.unwrap_or_default();
    Err(anyhow!("{} -> {}: {}", what, status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localpart_extraction() {
        assert_eq!(localpart_of("@pipo_irc_alice:example.org"), Some("pipo_irc_alice"));
        assert_eq!(localpart_of("@pipo:example.org"), Some("pipo"));
        assert_eq!(localpart_of("not-a-user"), None);
    }

    #[test]
    fn localpart_sanitization() {
        assert_eq!(sanitize_localpart("Alice"), "alice");
        assert_eq!(sanitize_localpart("A B!C"), "a_b_c");
        assert_eq!(sanitize_localpart("user.name_1"), "user.name_1");
        assert_eq!(sanitize_localpart("IRC"), "irc");
    }

    #[test]
    fn emoji_shortcode_roundtrip() {
        // Known emoji maps unicode -> shortcode and back.
        assert_eq!(key_to_emoji("😀"), "grinning");
        // Unknown reaction keys pass through unchanged.
        assert_eq!(key_to_emoji("+1-custom"), "+1-custom");
    }
}
