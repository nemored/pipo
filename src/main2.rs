use std::{sync::Arc, collections::HashMap, env};

use anyhow::{anyhow, Context};
use deadpool_sqlite::{Config, Runtime};
use pipo::{self, ParsedConfig, Bus, ConfigTransport, IRC, Discord, Message, Transport, TransportId, Slack, Mumble, Rachni, Database};
use regex::Regex;
use tokio::{sync::{mpsc, Mutex}, task::JoinHandle, fs::File};
use tokio_stream::{StreamMap, StreamExt};

pub(crate) enum Request {
    Supervise(JoinHandle<()>),
    UpdateConfig(Vec<u8>),
}

pub(crate) enum Reply {
    Error(String),
}

async fn config(
    parent: mpsc::UnboundedSender<Request>,
    rx: mpsc::UnboundedReceiver<(Request, mpsc::UnboundedSender<Reply>)>,
    database: Option<Database>,
    router_tx: mpsc::Sender<(Message, TransportId)>,
) -> anyhow::Result<()> {
    let mut bus_map = HashMap::new();
    let mut bus_id_map = HashMap::new();
    let mut transport_map = HashMap::new();
    while let Some((Request::UpdateConfig(read_buf), tx)) = rx.recv().await {
        let config_json: ParsedConfig = match serde_json::from_slice(&read_buf[..]) {
            Ok(x) => x,
            Err(_) => {
                let reply = Reply::Error("Couldn't parse the JSON in the config file".to_string());
                tx.send(reply);
                continue;
            }
        };
        let mut i = bus_id_map.len();
        for bus in config_json.buses.into_iter() {
            bus_id_map.insert(bus.id.clone(), i);
            let bus = Arc::new(Bus {
                id: i,
                name: bus.id,
                subscribers: vec![],
            });
            bus_map.insert(bus.id, bus);
            i += 1;
        }
        let mut transport_id = transport_map.len();
        for transport in config_json.transports {
            let (inbox_tx, inbox_rx) = mpsc::channel(100);
            let instance: impl Transport = match transport {
                ConfigTransport::IRC {
                    nickname,
                    server,
                    use_tls,
                    img_root,
                    channel_mapping,
                } => {
                    let channel_mapping = channel_mapping
                        .iter()
                        .filter_map(|(c, b)| {
                            bus_id_map
                                .get(&**b)
                                .and_then(|x| bus_map.get(x))
                                .map(|x| (c.clone(), x.clone()))
                        })
                        .collect();
                    IRC::new(
                        router_tx.clone(),
                        inbox_rx,
                        database.to_owned(),
                        nickname.to_string(),
                        server.to_string(),
                        use_tls,
                        &img_root,
                        &channel_mapping,
                        transport_id,
                    )
                    .await?
                }
                ConfigTransport::Discord {
                    token,
                    guild_id,
                    channel_mapping,
                } => {
                    let channel_mapping = channel_mapping
                        .iter()
                        .filter_map(|(c, b)| {
                            bus_id_map
                                .get(&**b)
                                .and_then(|x| bus_map.get(x))
                                .map(|x| (c.clone(), x.clone()))
                        })
                        .collect();
                    Discord::new(
                        transport_id,
                        router_tx.clone(),
                        inbox_rx,
                        database.to_owned(),
                        token.to_string(),
                        guild_id,
                        &channel_mapping,
                    )
                    .await?
                }
                ConfigTransport::Slack {
                    token,
                    bot_token,
                    channel_mapping,
                } => {
                    let channel_mapping = channel_mapping
                        .iter()
                        .filter_map(|(c, b)| {
                            bus_id_map
                                .get(&**b)
                                .and_then(|x| bus_map.get(x))
                                .map(|x| (c.clone(), x.clone()))
                        })
                        .collect();
                    Slack::new(
                        transport_id,
                        router_tx.clone(),
                        inbox_rx,
                        database.to_owned(),
                        token.to_string(),
                        bot_token.to_string(),
                        &channel_mapping,
                    )
                    .await?
                }
                ConfigTransport::Minecraft {
                    username: _,
                    buses: _,
                } => {
                    todo!("Minecraft");
                }
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
                    let channel_mapping = channel_mapping
                        .iter()
                        .filter_map(|(c, b)| {
                            bus_id_map
                                .get(&**b)
                                .and_then(|x| bus_map.get(x))
                                .map(|x| (c.clone(), x.clone()))
                        })
                        .collect();
                    Mumble::new(
                        transport_id,
                        server.clone(),
                        password.clone(),
                        nickname.clone(),
                        client_cert.clone(),
                        server_cert.clone(),
                        comment.as_deref(),
                        router_tx.clone(),
                        inbox_rx,
                        &channel_mapping,
                        &voice_channel_mapping,
                        database.to_owned(),
                    )
                    .await?
                }
                ConfigTransport::Rachni {
                    server,
                    api_key,
                    interval,
                    buses,
                } => {
                    let buses = buses
                        .iter()
                        .filter_map(|b| bus_id_map.get(&**b).and_then(|x| bus_map.get(x)).cloned())
                        .collect();
                    Rachni::new(
                        transport_id,
                        router_tx.clone(),
                        &server,
                        &api_key,
                        interval,
                        &buses,
                        database.to_owned(),
                    )
                    .await?
                }
            };
            let handle = instance.start();
            tokio::spawn(async move { while parent.send(Request::Supervise(handle)).is_err() {} });
            transport_id += 1;
        }
    }

	Ok(())
}

async fn router(
    transport_map: HashMap<TransportId, mpsc::Sender<Message>>,
    mut rx: mpsc::Receiver<(Message, TransportId)>,
) {
    while let Some((message, transport_id)) = rx.recv().await {
        for (id, tx) in transport_map.iter() {
            if *id == transport_id {
                continue;
            }
            let message = message.clone();
            let tx = tx.clone();
            tokio::spawn(async move { tx.send(message).await });
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	let args: Vec<String> = env::args().collect();
    let config_path = args.get(1).cloned().or(env::var("PIPO_CONFIG_PATH").ok());
    let db_path = args.get(2).cloned().or(env::var("PIPO_DB_PATH").ok());
    let host = env::var("PIPO_HOST").ok();
    let port = env::var("PIPO_PORT").ok();
    // Parse command line arguments
    //
    // Usage: ./pipo path-oogkm.json [path-to-db.db3]

    if db_path.is_none() || (config_path.is_none() && (host.is_none() || port.is_none())) {
        println!(
            "Usage: {} path-to-config.json [path-to-db.sqlite3]",
            args.get(0).unwrap_or(&"pipo".to_owned())
        );
        return Ok(()); // no, don't do this
    }

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
                                           mumbleid  INTEGER,
                                           modtime   DEFAULT 
                                             (strftime('%Y-%m-%d %H:%M:%S:%s',
                                                       'now', 
                                                       'localtime'))
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

    let (supervisor_tx, supervisor_rx) = mpsc::unbounded_channel();
    let (child_tx, child_rx) = mpsc::unbounded_channel();
    let supervisor = tokio::spawn(async move {
        let mut handles = StreamMap::new();
        tokio::select! {
            Some(request) = supervisor_rx.recv() => match request {
                Request::Supervise(handle) => {
                    let k = handles.len();
                    handles.insert(k, handle);
                },
                _ => ()
            },
            result = handles.next() => {
                todo!();
            }
        }
    });

    let (router_tx, router_rx) = mpsc::channel(10);
    let mut transport_map = HashMap::new();
    let router_handle = tokio::spawn(router(transport_map, router_rx));
    supervisor_tx.send(Request::Supervise(router_handle));

    let config_handle = tokio::spawn(config(
        supervisor_tx.clone(),
        child_rx,
        pipo_id,
        db_pool,
        router_tx.clone(),
    ));
    supervisor_tx.send(Request::Supervise(config_handle));

    match host {
        Some(host) => {
            if port.is_none() {
                return Err(anyhow!("Port must be specified if running in listen mode!"));
            }
            if config_path.is_some() {
                println!("operating in listen; config path will be ignored");
            }
            let port = port.unwrap();
        }
        None => {
            let mut config = File::open(config_path.unwrap())
                .await
                .context("Couldn't open config file")?;

            // Parse JSON
            let mut read_buf = Vec::new();
            config
                .read_to_end(&mut read_buf)
                .await
                .context("Couldn't read config file")?;
            let comment_removal_regex = Regex::new("//[^\n\r]*").unwrap();

            // do the rest
            let read_buf = comment_removal_regex
                .replace_all(&read_buf, &b""[..])
                .to_vec();

            child_tx.send(Request::UpdateConfig(read_buf));
        }
    }

    supervisor.await;

    Ok(())
}
