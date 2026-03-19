use std::{
    collections::HashMap,
    env, fmt,
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Context};
use deadpool_sqlite::Pool;
use deadpool_sqlite::{Config, Runtime};
use regex::bytes::Regex;
use rusqlite::Error::QueryReturnedNoRows;
use serde::Deserialize;
use serde_json;
use tokio::{fs::File, io::AsyncReadExt, sync::broadcast};

mod discord;
mod irc;
mod mumble;
pub(crate) mod protos;
mod rachni;
pub mod slack;

use crate::discord::Discord;
use crate::irc::{
    IrcPresenceMode, ThreadContextRepeat, ThreadFallbackStyle, ThreadPresentationMode, IRC,
};
use crate::mumble::Mumble;
use crate::rachni::Rachni;
use crate::slack::Slack;

pub use crate::slack::objects;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteActor {
    transport: String,
    remote_id: String,
    display_name: String,
    avatar_url: Option<String>,
    presence_identity_key: Option<String>,
}

impl RemoteActor {
    fn new(
        transport: impl Into<String>,
        remote_id: impl Into<String>,
        display_name: impl Into<String>,
        avatar_url: Option<String>,
    ) -> Self {
        Self {
            transport: transport.into(),
            remote_id: remote_id.into(),
            display_name: display_name.into(),
            avatar_url,
            presence_identity_key: None,
        }
    }

    fn transport(&self) -> &str {
        &self.transport
    }

    fn remote_id(&self) -> &str {
        &self.remote_id
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn avatar_url(&self) -> Option<&str> {
        self.avatar_url.as_deref()
    }
}

pub(crate) async fn upsert_remote_actor(pool: &Pool, actor: &RemoteActor) -> anyhow::Result<i64> {
    let transport = actor.transport().to_string();
    let remote_user_id = actor.remote_id().to_string();
    let display_name = actor.display_name().to_string();
    let avatar_url = actor.avatar_url().map(ToOwned::to_owned);
    let conn = pool.get().await?;
    conn.interact(move |conn| -> anyhow::Result<i64> {
        conn.execute(
            "INSERT INTO remote_actors (transport, remote_user_id, latest_display_name, latest_avatar_url)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(transport, remote_user_id) DO UPDATE SET
               latest_display_name = excluded.latest_display_name,
               latest_avatar_url = excluded.latest_avatar_url,
               updated_at = CURRENT_TIMESTAMP",
            rusqlite::params![transport.clone(), remote_user_id.clone(), display_name, avatar_url],
        )?;
        Ok(conn.query_row(
            "SELECT id FROM remote_actors WHERE transport = ?1 AND remote_user_id = ?2",
            rusqlite::params![transport, remote_user_id],
            |row| row.get(0),
        )?)
    })
    .await
    .unwrap_or_else(|_| Err(anyhow!("Interact Error")))
}

pub(crate) async fn upsert_irc_presence(
    pool: &Pool,
    actor: &RemoteActor,
    preferred_nick: Option<&str>,
    last_successful_nick: Option<&str>,
    collision_suffix: i64,
    mark_active: bool,
) -> anyhow::Result<()> {
    let actor_id = upsert_remote_actor(pool, actor).await?;
    let preferred_nick = preferred_nick.map(ToOwned::to_owned);
    let last_successful_nick = last_successful_nick.map(ToOwned::to_owned);
    let conn = pool.get().await?;
    conn.interact(move |conn| -> anyhow::Result<()> {
        conn.execute(
            "INSERT INTO irc_presences (remote_actor_id, preferred_nick, last_successful_nick, collision_suffix, last_seen_at, last_active_at)
             VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP, CASE WHEN ?5 THEN CURRENT_TIMESTAMP ELSE NULL END)
             ON CONFLICT(remote_actor_id) DO UPDATE SET
               preferred_nick = COALESCE(excluded.preferred_nick, irc_presences.preferred_nick),
               last_successful_nick = COALESCE(excluded.last_successful_nick, irc_presences.last_successful_nick),
               collision_suffix = excluded.collision_suffix,
               last_seen_at = CURRENT_TIMESTAMP,
               last_active_at = CASE WHEN ?5 THEN CURRENT_TIMESTAMP ELSE irc_presences.last_active_at END,
               updated_at = CURRENT_TIMESTAMP",
            rusqlite::params![actor_id, preferred_nick, last_successful_nick, collision_suffix, mark_active],
        )?;
        Ok(())
    })
    .await
    .unwrap_or_else(|_| Err(anyhow!("Interact Error")))
}

pub(crate) async fn load_irc_presence(
    pool: &Pool,
    actor: &RemoteActor,
) -> anyhow::Result<Option<(Option<String>, Option<String>, i64)>> {
    let transport = actor.transport().to_string();
    let remote_user_id = actor.remote_id().to_string();
    let conn = pool.get().await?;
    conn.interact(
        move |conn| -> anyhow::Result<Option<(Option<String>, Option<String>, i64)>> {
            let mut stmt = conn.prepare(
                "SELECT p.preferred_nick, p.last_successful_nick, p.collision_suffix
             FROM irc_presences p
             JOIN remote_actors a ON a.id = p.remote_actor_id
             WHERE a.transport = ?1 AND a.remote_user_id = ?2",
            )?;
            let mut rows =
                stmt.query(rusqlite::params![transport.clone(), remote_user_id.clone()])?;
            if let Some(row) = rows.next()? {
                Ok(Some((row.get(0)?, row.get(1)?, row.get(2)?)))
            } else {
                Ok(None)
            }
        },
    )
    .await
    .unwrap_or_else(|_| Err(anyhow!("Interact Error")))
}

#[derive(Clone, Debug)]
enum Message {
    Action {
        sender: usize,
        pipo_id: i64,
        actor: RemoteActor,
        thread: Option<ThreadRef>,
        message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        is_edit: bool,
        irc_flag: bool,
    },
    Bot {
        sender: usize,
        pipo_id: i64,
        transport: String,
        message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        is_edit: bool,
    },
    Delete {
        sender: usize,
        pipo_id: i64,
        transport: String,
    },
    Names {
        sender: usize,
        actor: RemoteActor,
        message: Option<String>,
    },
    Pin {
        sender: usize,
        pipo_id: i64,
        remove: bool,
    },
    Reaction {
        sender: usize,
        pipo_id: i64,
        actor: RemoteActor,
        emoji: String,
        remove: bool,
        thread: Option<ThreadRef>,
    },
    Text {
        sender: usize,
        pipo_id: i64,
        actor: RemoteActor,
        thread: Option<ThreadRef>,
        message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        is_edit: bool,
        irc_flag: bool,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ThreadRef {
    origin_transport: String,
    thread_root_id: Option<String>,
    reply_target_id: Option<u64>,
    root_author: Option<String>,
    root_excerpt: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct Attachment {
    id: u64,
    pipo_id: Option<i64>,
    service_name: Option<String>,
    service_url: Option<String>,
    author_name: Option<String>,
    author_subname: Option<String>,
    author_link: Option<String>,
    author_icon: Option<String>,
    filename: Option<String>,
    from_url: Option<String>,
    original_url: Option<String>,
    footer: Option<String>,
    footer_icon: Option<String>,
    content_type: Option<String>,
    size: Option<u64>,
    text: Option<String>,
    image_url: Option<String>,
    image_bytes: Option<u64>,
    image_height: Option<u64>,
    image_width: Option<u64>,
    fallback: Option<String>,
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Message::Action {
                sender: _,
                pipo_id: _,
                actor: _,
                thread: _,
                message,
                attachments: _,
                is_edit: _,
                irc_flag: _,
            } => match message {
                Some(message) => write!(f, "{}", message),
                None => write!(f, "Empty message"),
            },
            Message::Bot {
                sender: _,
                pipo_id: _,
                transport: _,
                message,
                attachments: _,
                is_edit: _,
            } => match message {
                Some(message) => write!(f, "{}", message),
                None => write!(f, "Empty message"),
            },
            Message::Delete {
                sender: _,
                pipo_id: _,
                transport: _,
            } => write!(f, "Delete message"),
            Message::Names {
                sender: _,
                actor: _,
                message,
            } => match message {
                Some(message) => write!(f, "{}", message),
                None => write!(f, "Empty message"),
            },
            Message::Pin {
                sender: _,
                pipo_id: _,
                remove: _,
            } => write!(f, "Pin message"),
            Message::Reaction {
                sender: _,
                pipo_id: _,
                actor: _,
                emoji,
                remove: _,
                thread: _,
            } => write!(f, ":{}:", emoji),
            Message::Text {
                sender: _,
                pipo_id: _,
                actor: _,
                thread: _,
                message,
                attachments: _,
                is_edit: _,
                irc_flag: _,
            } => match message {
                Some(message) => write!(f, "{}", message),
                None => write!(f, "Empty Message"),
            },
        }
    }
}

#[derive(Deserialize, Debug)]
struct ConfigBus {
    id: String,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "transport")]
enum ConfigTransport {
    IRC {
        nickname: Arc<String>,
        server: Arc<String>,
        use_tls: bool,
        img_root: Arc<String>,
        channel_mapping: HashMap<Arc<String>, Arc<String>>,
        #[serde(default)]
        thread_presentation_mode: ThreadPresentationMode,
        #[serde(default)]
        thread_fallback_style: ThreadFallbackStyle,
        #[serde(default)]
        thread_context_repeat: ThreadContextRepeat,
        #[serde(default = "default_thread_excerpt_len")]
        thread_excerpt_len: usize,
        #[serde(default = "default_show_thread_root_marker")]
        show_thread_root_marker: bool,
        #[serde(default)]
        presence_mode: IrcPresenceMode,
    },
    Discord {
        token: Arc<String>,
        guild_id: u64,
        channel_mapping: HashMap<Arc<String>, Arc<String>>,
    },
    Slack {
        token: Arc<String>,
        bot_token: Arc<String>,
        channel_mapping: HashMap<Arc<String>, Arc<String>>,
    },
    Minecraft {
        username: Arc<String>,
        buses: Vec<Arc<String>>,
    },
    Mumble {
        server: Arc<String>,
        password: Arc<Option<String>>,
        nickname: Arc<String>,
        client_cert: Arc<Option<String>>,
        server_cert: Arc<Option<String>>,
        comment: Option<String>,
        channel_mapping: HashMap<Arc<String>, Arc<String>>,
        voice_channel_mapping: HashMap<Arc<String>, Arc<String>>,
    },
    Rachni {
        server: Arc<String>,
        api_key: Arc<String>,
        interval: u64,
        buses: Arc<Vec<String>>,
    },
}

#[derive(Deserialize, Debug)]
struct ParsedConfig {
    buses: Vec<ConfigBus>,
    transports: Vec<ConfigTransport>,
}

fn default_thread_excerpt_len() -> usize {
    120
}

fn default_show_thread_root_marker() -> bool {
    true
}

pub async fn inner_main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let config_path = args.get(1).cloned().or(env::var("CONFIG_PATH").ok());
    let db_path = args.get(2).cloned().or(env::var("DB_PATH").ok());
    // Parse command line arguments
    //
    // Usage: ./pipo path-oogkm.json [path-to-db.db3]

    if config_path.is_none() || db_path.is_none() {
        println!(
            "Usage: {} path-to-config.json [path-to-db.sqlite3]",
            args.get(0).unwrap_or(&"pipo".to_owned())
        );
        return Ok(()); // no, don't do this
    }

    let mut config = File::open(config_path.unwrap())
        .await
        .context("Couldn't open config file")?;
    let db_pool = Config::new(&db_path.unwrap()).create_pool(Runtime::Tokio1)?;

    // TODO: ugly error handling needs fixing
    let pipo_id: Arc<Mutex<i64>> = Arc::new(Mutex::new(
        db_pool
            .get()
            .await?
            .interact(move |conn| -> anyhow::Result<i64> {
                match conn.query_row(
                    "SELECT name 
                                  FROM sqlite_master 
                                  WHERE type='table' 
                                  AND name='messages'",
                    [],
                    |row| row.get::<usize, String>(0),
                ) {
                    Ok(_) => eprintln!("Table found"),
                    Err(QueryReturnedNoRows) => {
                        conn.execute_batch(
                            "CREATE TABLE messages (
                                           id        INTEGER PRIMARY KEY,
                                           slackid   TEXT,
                                           discordid INTEGER,
                                           ircid     TEXT,
                                           modtime   DEFAULT 
                                             (strftime('%Y-%m-%d %H:%M:%S:%s',
                                                       'now', 
                                                       'localtime'))
                                           );
                             CREATE TABLE identity_groups (
                                           id INTEGER PRIMARY KEY,
                                           created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                                           updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                                           );
                             CREATE TABLE remote_actors (
                                           id INTEGER PRIMARY KEY,
                                           transport TEXT NOT NULL,
                                           remote_user_id TEXT NOT NULL,
                                           latest_display_name TEXT NOT NULL,
                                           latest_avatar_url TEXT,
                                           identity_group_id INTEGER,
                                           created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                                           updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                                           UNIQUE(transport, remote_user_id),
                                           FOREIGN KEY(identity_group_id) REFERENCES identity_groups(id)
                                           );
                             CREATE TABLE linked_identities (
                                           id INTEGER PRIMARY KEY,
                                           identity_group_id INTEGER NOT NULL,
                                           remote_actor_id INTEGER NOT NULL,
                                           created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                                           UNIQUE(identity_group_id, remote_actor_id),
                                           FOREIGN KEY(identity_group_id) REFERENCES identity_groups(id),
                                           FOREIGN KEY(remote_actor_id) REFERENCES remote_actors(id)
                                           );
                             CREATE TABLE irc_presences (
                                           remote_actor_id INTEGER PRIMARY KEY,
                                           preferred_nick TEXT,
                                           last_successful_nick TEXT,
                                           collision_suffix INTEGER NOT NULL DEFAULT 0,
                                           last_seen_at TEXT,
                                           last_active_at TEXT,
                                           created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                                           updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                                           FOREIGN KEY(remote_actor_id) REFERENCES remote_actors(id)
                                           );
                             CREATE TRIGGER updatemodtime
                                        BEFORE update ON messages
                                        begin
                                        update messages set modtime 
                                          = strftime('%Y-%m-%d %H:%M:%S:%s',
                                                     'now', 
                                                     'localtime') 
                                            where id = old.id;
                                        end;",
                        )?;
                    }
                    Err(e) => return Err(anyhow!(e)),
                }

                let ircid_exists = conn
                    .prepare("PRAGMA table_info(messages)")?
                    .query_map([], |row| row.get::<usize, String>(1))?
                    .filter_map(Result::ok)
                    .any(|column| column == "ircid");
                if !ircid_exists {
                    conn.execute("ALTER TABLE messages ADD COLUMN ircid TEXT", [])?;
                }

                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS identity_groups (
                         id INTEGER PRIMARY KEY,
                         created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                         updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                     );
                     CREATE TABLE IF NOT EXISTS remote_actors (
                         id INTEGER PRIMARY KEY,
                         transport TEXT NOT NULL,
                         remote_user_id TEXT NOT NULL,
                         latest_display_name TEXT NOT NULL,
                         latest_avatar_url TEXT,
                         identity_group_id INTEGER,
                         created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                         updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                         UNIQUE(transport, remote_user_id),
                         FOREIGN KEY(identity_group_id) REFERENCES identity_groups(id)
                     );
                     CREATE TABLE IF NOT EXISTS linked_identities (
                         id INTEGER PRIMARY KEY,
                         identity_group_id INTEGER NOT NULL,
                         remote_actor_id INTEGER NOT NULL,
                         created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                         UNIQUE(identity_group_id, remote_actor_id),
                         FOREIGN KEY(identity_group_id) REFERENCES identity_groups(id),
                         FOREIGN KEY(remote_actor_id) REFERENCES remote_actors(id)
                     );
                     CREATE TABLE IF NOT EXISTS irc_presences (
                         remote_actor_id INTEGER PRIMARY KEY,
                         preferred_nick TEXT,
                         last_successful_nick TEXT,
                         collision_suffix INTEGER NOT NULL DEFAULT 0,
                         last_seen_at TEXT,
                         last_active_at TEXT,
                         created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                         updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                         FOREIGN KEY(remote_actor_id) REFERENCES remote_actors(id)
                     );",
                )?;

                Ok(
                    match conn.query_row(
                        "SELECT id FROM messages 
                                     ORDER BY modtime DESC",
                        [],
                        |row| row.get(0),
                    ) {
                        Ok(id) => id,
                        Err(_) => 0,
                    },
                )
            })
            .await
            .unwrap_or_else(|_| Err(anyhow!("Interact Error")))?
            + 1,
    ));

    // Parse JSON
    let mut read_buf = Vec::new();
    config
        .read_to_end(&mut read_buf)
        .await
        .context("Couldn't read config file")?;
    let comment_removal_regex = Regex::new("//[^\n\r]*").unwrap();

    // do the rest
    let read_buf = comment_removal_regex.replace_all(&read_buf, &b""[..]);

    let config_json: ParsedConfig = serde_json::from_slice(&read_buf[..])
        .context("Couldn't parse the JSON in the config file")?;

    // Once the configuration JSON has been deserialized into a
    // ParsedConfig, iterate through buses, creating a broadcast channel
    // for each.
    let mut bus_map: HashMap<String, broadcast::Sender<Message>> = HashMap::new();
    for bus in config_json.buses.into_iter() {
        bus_map.insert(bus.id, broadcast::channel(100).0);
    }

    // Create Sender and Receiver for database mpsc channel
    // let (db_tx, mut db_rx): (mpsc::Sender<(String, String)>,
    //                  mpsc::Receiver<(String, String)>)
    //     = mpsc::channel(100);

    // Now do transports and create a ???
    // for each.

    let mut all_transport_tasks = vec![];
    // let handle = tokio::spawn(async move {
    //     while let Some((command, message)) = db_rx.recv().await {
    //         match command.as_str() {
    //         "execute" => if let Err(e) = db.execute(&message, []) {
    //             eprintln!("Error executing db command: {}", e);
    //         },
    //         default => eprintln!("{} not implemented.", default)
    //         }
    //     }
    // });

    // all_transport_tasks.push(handle);

    for transport_id in 0..config_json.transports.len() {
        match &config_json.transports[transport_id] {
            ConfigTransport::IRC {
                nickname,
                server,
                use_tls,
                img_root,
                channel_mapping,
                thread_presentation_mode,
                thread_fallback_style,
                thread_context_repeat,
                thread_excerpt_len,
                show_thread_root_marker,
                presence_mode,
            } => {
                // tokio::spawn maybe?
                let mut instance = IRC::new(
                    &bus_map,
                    pipo_id.clone(),
                    db_pool.clone(),
                    nickname.to_string(),
                    server.to_string(),
                    *use_tls,
                    &img_root,
                    &channel_mapping,
                    *thread_presentation_mode,
                    *thread_fallback_style,
                    *thread_context_repeat,
                    *thread_excerpt_len,
                    *show_thread_root_marker,
                    *presence_mode,
                    transport_id,
                )
                .await?;
                // you should push enough state to connect the spawned
                // transport to all its buses... you don't need to push
                // the task itself, tokio will track that

                // let spawn take care of it... as in, the loop will continue
                // while spawn does its thing why don't the comments continue
                // automatically on the new line???
                let handle = tokio::spawn(async move {
                    match instance.connect().await {
                        Ok(_) => eprintln!("IRC::connect() exited Ok"),
                        Err(e) => {
                            eprintln!("IRC::connect() exited with Error: {:#}", e);
                        }
                    }
                });
                all_transport_tasks.push(handle);
            }
            ConfigTransport::Discord {
                token,
                guild_id,
                channel_mapping,
            } => {
                let mut instance = Discord::new(
                    transport_id,
                    &bus_map,
                    pipo_id.clone(),
                    db_pool.clone(),
                    token.to_string(),
                    *guild_id,
                    &channel_mapping,
                )
                .await?;
                let handle = tokio::spawn(async move {
                    match instance.connect().await {
                        Ok(_) => eprintln!("Discord::connect() exited Ok"),
                        Err(e) => {
                            eprintln!(
                                "Discord::connect() exited with \
                       Error: {:#}",
                                e
                            );
                        }
                    }
                });
                all_transport_tasks.push(handle);
            }
            ConfigTransport::Slack {
                token,
                bot_token,
                channel_mapping,
            } => {
                let mut instance = Slack::new(
                    transport_id,
                    &bus_map,
                    pipo_id.clone(),
                    db_pool.clone(),
                    token.to_string(),
                    bot_token.to_string(),
                    &channel_mapping,
                )
                .await?;
                let handle = tokio::spawn(async move {
                    match instance.connect().await {
                        Ok(_) => eprintln!("Slack::connect() exited Ok"),
                        Err(e) => {
                            eprintln!(
                                "Slack::connect() exited with Error: \
                       {:#}",
                                e
                            );
                        }
                    }
                });
                all_transport_tasks.push(handle);
            }
            ConfigTransport::Minecraft {
                username: _,
                buses: _,
            } => todo!("Minecraft"),
            ConfigTransport::Mumble {
                server,
                password,
                nickname,
                client_cert,
                server_cert,
                comment,
                channel_mapping,
                voice_channel_mapping,
            } => {
                let mut instance = Mumble::new(
                    transport_id,
                    server.clone(),
                    password.clone(),
                    nickname.clone(),
                    client_cert.clone(),
                    server_cert.clone(),
                    comment.as_deref(),
                    &bus_map,
                    &channel_mapping,
                    &voice_channel_mapping,
                    pipo_id.clone(),
                    db_pool.clone(),
                )
                .await?;
                let handle = tokio::spawn(async move {
                    match instance.run().await {
                        Ok(_) => eprintln!("Mumble::run() exited Ok"),
                        Err(e) => {
                            eprintln!(
                                "Mumble::run() exited with Error: \
                                       {:#}",
                                e
                            )
                        }
                    }
                });
                all_transport_tasks.push(handle);
            }
            ConfigTransport::Rachni {
                server,
                api_key,
                interval,
                buses,
            } => {
                let instance = Rachni::new(
                    transport_id,
                    &bus_map,
                    &server,
                    &api_key,
                    *interval,
                    &buses,
                    db_pool.clone(),
                    pipo_id.clone(),
                )
                .await?;
                let handle = tokio::spawn(async move {
                    match instance.run().await {
                        Ok(_) => eprintln!("Rachni::run() exited Ok"),
                        Err(e) => {
                            eprintln!(
                                "Rachni::run() exited with Error: \
                       {:#}",
                                e
                            );
                        }
                    }
                });
                all_transport_tasks.push(handle);
            }
        }
    }

    for task in all_transport_tasks {
        match task.await {
            Ok(_) => (),
            Err(e) => eprintln!("Task error: {:#}", e),
        }
    }

    Ok(())
}
