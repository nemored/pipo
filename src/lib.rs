use std::{
    collections::HashMap,
    fmt::{self, Debug, Display},
    sync::{Arc, Mutex}, ops::Deref,
};

use anyhow::anyhow;
use deadpool_sqlite::{Config, Pool, Runtime};
use discord::DiscordId;
use regex::bytes::Regex;
use rusqlite::{types::FromSql, ToSql, params};
use serde::{Deserialize, Serialize};
use serde_json;
use serenity::model::prelude::ChannelId;

use tokio::{
    fs::File,
    io::AsyncReadExt,
    sync::mpsc::{self, error::SendError},
    task::JoinHandle,
};
use tokio_stream::{StreamExt, StreamMap};

mod discord;
mod irc;
mod mumble;
pub(crate) mod protos;
mod rachni;
pub mod slack;

pub use crate::discord::Discord;
pub use crate::irc::IRC;
pub use crate::mumble::Mumble;
pub use crate::rachni::Rachni;
pub use crate::slack::Slack;

pub use crate::slack::objects;
use crate::slack::objects::Timestamp;

pub(crate) trait MessageId: Clone + Debug + Display + FromSql + Send + Sync + ToSql {
    const COLUMN: &'static str;
}

#[derive(Clone)]
pub struct Database {
    pool: Pool,
    pipo_id: Arc<Mutex<i64>>,
}

impl Database {
    pub fn new(pool: Pool, pipo_id: Arc<Mutex<i64>>) -> Self {
        Database { pool, pipo_id }
    }
    async fn insert_into_messages_table<I>(&self, id: Option<I>) -> anyhow::Result<i64>
    where
        I: MessageId + 'static
    {
        let conn = self.pool.get().await.unwrap();
        let column = id.as_ref().map(|_| I::COLUMN);
        let pipo_id = *self.pipo_id.lock().unwrap();
        if let Some(ref id) = id {
            eprintln!("Inserting message_id {id} into table at id {pipo_id}");
        }
        let id = id.clone();

        // TODO: ugly error handling needs fixing
        conn.interact(move |conn| -> anyhow::Result<usize> {
            Ok(conn.execute(
                "INSERT OR REPLACE INTO messages (id, ?1) 
            VALUES (?2, ?3)",
                params![column, pipo_id, id],
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
    async fn select_id_from_messages<M>(&self, id: M) -> anyhow::Result<Option<i64>>
    where
        M: MessageId + 'static
    {
        let conn = self.pool.get().await.unwrap();

        Ok(Some(
            conn.interact(move |conn| -> anyhow::Result<i64> {
                Ok(conn.query_row(
                    "SELECT id FROM messages WHERE ?1 = ?2",
                    params![M::COLUMN, id],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap_or_else(|_| Err(anyhow!("Interact Error")))?,
        ))
    }
    async fn update_messages_table<M>(
        &self,
        pipo_id: i64,
        message_id: M,
    ) -> anyhow::Result<()>
    where
        M: MessageId + 'static
    {
        let conn = self.pool.get().await.unwrap();

        eprintln!("Adding {message_id} ID: {pipo_id}");

        // TODO: ugly error handling needs fixing
        conn.interact(move |conn| -> anyhow::Result<usize> {
            Ok(conn.execute(
                "UPDATE messages SET ?2 = ?3
            WHERE id = ?1",
                params![pipo_id, M::COLUMN, message_id],
            )?)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("Interact Error")))?;

        Ok(())
    }
    async fn select_messageid_from_messages<M>(&self, pipo_id: i64) -> anyhow::Result<Option<M>>
    where
        M: MessageId + 'static
    {
        let conn = self.pool.get().await.unwrap();

        // TODO: ugly error handling needs fixing
        let ret = match conn
            .interact(move |conn| -> anyhow::Result<Option<M>> {
                Ok(conn.query_row(
                    "SELECT ?2 FROM messages WHERE id = ?1",
                    params![pipo_id, M::COLUMN],
                    |row| row.get(0),
                )?)
            })
            .await
        {
            Ok(res) => res,
            Err(_) => Err(anyhow!("Interact Error")),
        }?;

        eprintln!("Found ts {:?} at id {}", ret, pipo_id);

        Ok(ret)
    }
    async fn get_messageid_from_messageid<I, O>(&self, message_id: I) -> anyhow::Result<Option<O>>
    where
        I: MessageId + 'static,
        O: MessageId + 'static
    {
        let conn = self.pool.get().await.unwrap();
        let old_message_id = message_id.clone();

        // TODO: ugly error handling needs fixing
        let ret = match conn
            .interact(move |conn| -> anyhow::Result<Option<O>> {
                Ok(conn.query_row(
                    "SELECT ?3 FROM messages WHERE ?1 = ?2",
                    params![I::COLUMN, message_id, O::COLUMN],
                    |row| row.get(0),
                )?)
            })
            .await
        {
            Ok(res) => res,
            Err(_) => Err(anyhow!("Interact Error")),
        }?;

        eprintln!("Found ts {ret:?} at id {old_message_id}");

        Ok(ret)
    }
}

pub type TransportId = usize;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Subscriber {
    transport: TransportId,
}

impl Subscriber {
    pub fn new(transport: TransportId) -> Self {
        Self { transport }
    }
}

type BusId = usize;

#[derive(Clone, Deserialize, Eq, Debug, Hash, PartialEq, Serialize)]
pub struct Bus {
    pub id: BusId,
    pub name: String,
    pub subscribers: Vec<Subscriber>,
}

impl Bus {
    pub fn new(id: usize, name: &str) -> Self {
        Self {
            id,
            name: name.to_string(),
            subscribers: vec![],
        }
    }
    pub fn subscribe(
        &mut self,
        subscribers: &mut Subscribers,
        subscriber: &Subscriber,
    ) -> anyhow::Result<mpsc::Receiver<Message>> {
        if self.subscribers.iter().find(|x| *x == subscriber).is_none() {
            let (tx, rx) = mpsc::channel(100);
            self.subscribers.push(subscriber.clone());
            let res = subscribers.insert(subscriber.clone(), tx);
            assert!(res.is_none());
            Ok(rx)
        } else {
            Err(anyhow!("already subscribed"))
        }
    }
}

type Subscribers = HashMap<Subscriber, mpsc::Sender<Message>>;

#[derive(Clone, Eq, Hash, PartialEq)]
struct MessageSource {
    bus: BusId,
    transport: TransportId,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Message {
    Action {
        sender: usize,
        pipo_id: Option<i64>,
        transport: String,
        bus: Bus,
        username: String,
        avatar_url: Option<String>,
        thread: Option<(Option<Timestamp>, Option<ChannelId>)>,
        message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        is_edit: bool,
        irc_flag: bool,
    },
    Bot {
        sender: usize,
        pipo_id: Option<i64>,
        transport: String,
        bus: Bus,
        message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        is_edit: bool,
    },
    Delete {
        sender: usize,
        pipo_id: Option<i64>,
        transport: String,
        bus: Bus,
    },
    Names {
        sender: usize,
        transport: String,
        bus: Bus,
        username: String,
        message: Option<String>,
    },
    Pin {
        sender: usize,
        pipo_id: Option<i64>,
        bus: Bus,
        remove: bool,
    },
    Reaction {
        sender: usize,
        pipo_id: Option<i64>,
        transport: String,
        bus: Bus,
        emoji: String,
        remove: bool,
        username: Option<String>,
        avatar_url: Option<String>,
        thread: Option<(Option<Timestamp>, Option<ChannelId>)>,
    },
    Text {
        sender: usize,
        pipo_id: Option<i64>,
        transport: String,
        bus: Bus,
        username: String,
        avatar_url: Option<String>,
        thread: Option<(Option<Timestamp>, Option<ChannelId>)>,
        message: Option<String>,
        attachments: Option<Vec<Attachment>>,
        is_edit: bool,
        irc_flag: bool,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Attachment {
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
                transport: _,
                bus: _,
                username: _,
                avatar_url: _,
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
                bus: _,
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
                bus: _,
            } => write!(f, "Delete message"),
            Message::Names {
                sender: _,
                transport: _,
                bus: _,
                username: _,
                message,
            } => match message {
                Some(message) => write!(f, "{}", message),
                None => write!(f, "Empty message"),
            },
            Message::Pin {
                sender: _,
                pipo_id: _,
                bus: _,
                remove: _,
            } => write!(f, "Pin message"),
            Message::Reaction {
                sender: _,
                pipo_id: _,
                transport: _,
                bus: _,
                emoji,
                remove: _,
                username: _,
                avatar_url: _,
                thread: _,
            } => write!(f, ":{}:", emoji),
            Message::Text {
                sender: _,
                pipo_id: _,
                transport: _,
                bus: _,
                username: _,
                avatar_url: _,
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
pub struct ConfigBus {
    pub id: String,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "transport")]
pub enum ConfigTransport {
    IRC {
        nickname: Arc<String>,
        server: Arc<String>,
        use_tls: bool,
        img_root: Arc<String>,
        channel_mapping: HashMap<Arc<String>, Arc<String>>,
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
pub struct ParsedConfig {
    pub buses: Vec<ConfigBus>,
    pub transports: Vec<ConfigTransport>,
}

#[derive(Clone)]
pub(crate) struct Router(TransportId, mpsc::Sender<(Message, TransportId)>);

impl Router {
    pub async fn send(&self, message: Message) -> Result<(), SendError<(Message, TransportId)>> {
        self.1.send((message, self.0)).await
    }
}

pub trait Transport {
    fn start(self) -> JoinHandle<()>;
}