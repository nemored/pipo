use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    sync::{Arc, Mutex},
};

use anyhow::anyhow;
use deadpool_sqlite::Pool;
use reqwest::{Client as HttpClient, Method};
use rusqlite::{params, types::FromSql, ToSql};
use serde_json::Value;
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{self, Duration},
};

use crate::{Bus, Database, Message, MessageId, Router, Transport, TransportId};

const TRANSPORT_NAME: &'static str = "Rachni";

#[derive(Clone, Debug)]
struct RachniId {
    value: Option<usize>,
}

impl Display for RachniId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "None")
    }
}

impl FromSql for RachniId {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        use rusqlite::types::{FromSqlError, ValueRef::*};
        match value {
            Null => Ok(Self { value: None }),
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

impl ToSql for RachniId {
	fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
		self.value.to_sql()
	}
}

impl MessageId for RachniId {
    const COLUMN: &'static str = "rachniid";
}

pub struct Rachni {
    transport_id: usize,
    server: String,
    api_key: String,
    interval: u64,
    bus_map: HashMap<Arc<Bus>, Router>,
    database: Option<Database>,
}

impl Transport for Rachni {
    fn start(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            match self.run().await {
                Ok(_) => eprintln!("Rachni::run() exited Ok"),
                Err(e) => eprintln!("Rachni::run() exited with Error: {e:#}"),
            }
        })
    }
}

impl Rachni {
    pub async fn new(
        transport_id: TransportId,
        router: mpsc::Sender<(Message, TransportId)>,
        server: &str,
        api_key: &str,
        interval: u64,
        buses: &Vec<Arc<Bus>>,
        database: Option<Database>
    ) -> anyhow::Result<Rachni> {
        let server = String::from(server);
        let api_key = String::from(api_key);
        let router = Router(transport_id, router);
        let bus_map = buses
            .iter()
            .map(|bus| (bus.clone(), router.clone()))
            .collect();

        Ok(Rachni {
            transport_id,
            server,
            api_key,
            interval,
            bus_map,
            database,
        })
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        let mut streams = HashSet::new();
        let http = HttpClient::new();
        let url = format!("http://{}/api/{}/stream/ping", self.server, self.api_key);
        let response = http.request(Method::GET, &url).send().await?;

        if let Some(map) =
            serde_json::from_str::<Value>(response.text().await?.as_str())?.as_object()
        {
            for (stream, _) in map {
                streams.insert(stream.as_str().to_string());
            }
        }

        time::sleep(Duration::from_secs(self.interval)).await;

        loop {
            let mut new_stream_map = HashSet::new();
            let response = http.request(Method::GET, &url).send().await?;
            let json: Value = serde_json::from_str(response.text().await?.as_str())?;

            if let Some(map) = json.as_object() {
                for (stream, _) in map {
                    let stream = stream.as_str();

                    new_stream_map.insert(stream.to_string());

                    if let None = streams.get(stream) {
                        let message = format!(
                            "has started streaming: \
					       rtmp://{}/live/{} ",
                            self.server, stream
                        );

                        if let Err(e) = self.send_message(&stream, &message).await {
                            eprintln!("Rachni couldn't send message: {}", e);
                        }
                    }
                }

                for stream in streams.difference(&new_stream_map) {
                    let message = "has stopped streaming";

                    if let Err(e) = self.send_message(&stream, message).await {
                        eprintln!("Rachni couldn't send message: {}", e);
                    }
                }
            } else {
                for stream in streams.iter() {
                    let message = "has stopped streaming";

                    if let Err(e) = self.send_message(&stream, message).await {
                        eprintln!("Rachni couldn't send message: {}", e);
                    }
                }
            }

            streams = new_stream_map;

            time::sleep(Duration::from_secs(self.interval)).await;
        }
    }

    async fn send_message(&self, username: &str, message: &str) -> anyhow::Result<()> {
        let pipo_id = self.try_insert_into_messages_table().await?;
        let avatar_url = Some(format!(
            "http://{}/profiles/default/profile_default.png",
            self.server
        ));

        for (bus, sender) in self.bus_map.iter() {
            let message = Message::Action {
                sender: self.transport_id,
                pipo_id,
                transport: TRANSPORT_NAME.to_string(),
                bus: bus.as_ref().to_owned(),
                username: username.to_string(),
                avatar_url: avatar_url.to_owned(),
                thread: None,
                message: Some(message.to_string()),
                attachments: None,
                is_edit: false,
                irc_flag: false,
            };
            sender.send(message.clone()).await?;
        }

        Ok(())
    }

    async fn try_insert_into_messages_table(&self) -> anyhow::Result<Option<i64>> {
        match &self.database {
			Some(db) => Some(db.insert_into_messages_table::<RachniId>(None).await),
			None => None
		}.transpose()
    }
}
