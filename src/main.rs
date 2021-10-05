use std::{
    collections::HashMap,
    env,
    fmt,
    sync::{Arc, Mutex},
};
use anyhow::{
    anyhow,
    Context,
};
use deadpool_sqlite::Config;
use regex::bytes::Regex;
use rusqlite::Error::QueryReturnedNoRows;
use serde::Deserialize;
use serde_json;
use tokio::{
    io::AsyncReadExt,
    fs::File,
    sync::broadcast,
};

mod irc;
use crate::irc::*;
mod slack;
use crate::slack::*;
mod discord;
use crate::discord::*;
mod rachni;
use crate::rachni::*;

#[derive(Deserialize, Debug)]
struct ConfigBus {
    id: String
}

#[derive(Deserialize, Debug)]
#[serde(tag="transport")]
enum ConfigTransport {
    IRC {
	nickname: String,
	server: String,
	use_tls: bool,
	channel_mapping: HashMap<String,String>,
    },
    Discord {
	token: String,
	guild_id: u64,
	channel_mapping: HashMap<String,String>,
    },
    Slack {
	token: String,
	bot_token: String,
	channel_mapping: HashMap<String,String>,
    },
    Minecraft {
	username: String,
	buses: Vec<String>,
    },
    Mumble {
	server: String,
	password: Option<String>,
	nickname: String,
	client_cert: Option<String>,
	server_cert: Option<String>,
	comment: Option<String>,
	channel_mapping: HashMap<String,String>,
	voice_channel_mapping: HashMap<String,String>,
    },
    Rachni {
	server: String,
	api_key: String,
	interval: u64,
	buses: Vec<String>
    },
}

#[derive(Deserialize, Debug)]
struct ParsedConfig {
    buses: Vec<ConfigBus>,
    transports: Vec<ConfigTransport>,
}

#[derive(Clone, Debug)]
enum Message {
    Action {
	sender: usize,
	pipo_id: i64,
	transport: String,
	username: String,
	avatar_url: Option<String>,
	thread: Option<(Option<String>, Option<u64>)>,
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
	transport: String,
	username: String,
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
	transport: String,
	emoji: String,
	remove: bool,
	username: Option<String>,
	avatar_url: Option<String>,
	thread: Option<(Option<String>, Option<u64>)>
    },
    Text {
	sender: usize,
	pipo_id: i64,
	transport: String,
	username: String,
	avatar_url: Option<String>,
	thread: Option<(Option<String>, Option<u64>)>,
	message: Option<String>,
	attachments: Option<Vec<Attachment>>,
	is_edit: bool,
	irc_flag: bool,
    }
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
		transport: _,
		username: _,
		avatar_url: _,
		thread: _,
		message,
		attachments: _,
		is_edit: _,
		irc_flag: _,
	    } => match message {
		Some(message) => write!(f, "{}", message),
		None => write!(f, "Empty message")
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
		None => write!(f, "Empty message")
	    },
	    Message::Delete {
		sender: _,
		pipo_id: _,
		transport: _,
	    } => write!(f, "Delete message"),
	    Message::Names {
		sender: _,
		transport: _,
		username: _,
		message,
	    } => match message {
		Some(message) => write!(f, "{}", message),
		None => write!(f, "Empty message")
	    },
	    Message::Pin {
		sender: _,
		pipo_id: _,
		remove: _,
	    } => write!(f, "Pin message"),
	    Message::Reaction {
		sender: _,
		pipo_id: _,
		transport: _,
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
		username: _,
		avatar_url: _,
		thread: _,
		message,
		attachments: _,
		is_edit: _,
		irc_flag: _,
	    } => match message {
		Some(message) => write!(f, "{}", message),
		None => write!(f, "Empty Message")
	    }
	}
    }
}

async fn inner_main() -> anyhow::Result<()> {
    // Parse command line arguments
    //
    // Usage: ./pipo path-oogkm.json [path-to-db.db3]
    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
	println!("Usage: {} path-to-config.json [path-to-db.sqlite3]",
		 args.get(0).unwrap_or(&"pipo".to_owned()));
	return Ok(()) // no, don't do this
    }

    let mut config = File::open(&args[1]).await
	.context("Couldn't open config file")?;
    let db_pool = Config::new(&args[2]).create_pool();

    let pipo_id: Arc<Mutex<i64>>
	= Arc::new(Mutex::new(db_pool.get().await?.interact(move |conn| -> anyhow::Result<i64> {
	    match conn.query_row("SELECT name 
                                  FROM sqlite_master 
                                  WHERE type='table' 
                                  AND name='messages'",
				 [], |row| row.get::<usize,String>(0)) {
		Ok(_) => eprintln!("Table found"),
		Err(QueryReturnedNoRows) => {
		    conn.execute_batch("CREATE TABLE messages (
                                           id        INTEGER PRIMARY KEY,
                                           slackid   TEXT,
                                           discordid INTEGER,
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
                                        end;")?;
		},
		Err(e) => return Err(anyhow!(e))
	    }

	    Ok(match conn.query_row("SELECT id FROM messages 
                                     ORDER BY modtime DESC",
				    [], |row| row.get(0)) {
		Ok(id) => id,
		Err(_) => 0
	    })
	}).await? + 1));

    // Parse JSON
    let mut read_buf = Vec::new();
    config.read_to_end(&mut read_buf).await
	.context("Couldn't read config file")?;
    let comment_removal_regex = Regex::new("//[^\n\r]*").unwrap();
   
    // do the rest
    let read_buf = comment_removal_regex.replace_all(&read_buf, &b""[..]);
    
    let config_json: ParsedConfig = serde_json::from_slice(&read_buf[..])
	.context("Couldn't parse the JSON in the config file")?;

    // Once the configuration JSON has been deserialized into a
    // ParsedConfig, iterate through buses, creating a broadcast channel
    // for each.
    let mut bus_map: HashMap<String, broadcast::Sender<Message>>
	= HashMap::new();
    for bus in config_json.buses.into_iter() {
	bus_map.insert(bus.id, broadcast::channel(100).0);
    }

    // Create Sender and Receiver for database mpsc channel
    // let (db_tx, mut db_rx): (mpsc::Sender<(String, String)>,
    // 			     mpsc::Receiver<(String, String)>)
    // 	= mpsc::channel(100);
    
    // Now do transports and create a ???
    // for each.

    let mut all_transport_tasks = vec![];
    // let handle = tokio::spawn(async move {
    // 	while let Some((command, message)) = db_rx.recv().await {
    // 	    match command.as_str() {
    // 		"execute" => if let Err(e) = db.execute(&message, []) {
    // 		    eprintln!("Error executing db command: {}", e);
    // 		},
    // 		default => eprintln!("{} not implemented.", default)
    // 	    }
    // 	}
    // });

    // all_transport_tasks.push(handle);
    
    for transport_id in 0..config_json.transports.len() {
	match &config_json.transports[transport_id] {
	    ConfigTransport::IRC {
		nickname,
		server,
		use_tls,
		channel_mapping
	    } => {
		// tokio::spawn maybe?
		let mut instance = IRC::new(&bus_map,
					    pipo_id.clone(),
					    db_pool.clone(),
					    nickname.to_string(),
					    server.to_string(),
					    *use_tls,
					    channel_mapping,
					    transport_id).await?;
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
			    eprintln!("IRC::connect() exited with Error: {:#}",
				      e);
			}
		    }
		});
		all_transport_tasks.push(handle);
	    }
	    ConfigTransport::Discord {
		token,
		guild_id,
		channel_mapping
	    } => {
		let mut instance = Discord::new(transport_id,
						&bus_map,
						pipo_id.clone(),
						db_pool.clone(),
						token.to_string(),
						*guild_id,
						channel_mapping).await?;
		let handle = tokio::spawn(async move {
 		    match instance.connect().await {
			Ok(_) => eprintln!("Discord::connect() exited Ok"),
			Err(e) => {
			    eprintln!("Discord::connect() exited with \
				       Error: {:#}", e);
			}
		    }
		});
		all_transport_tasks.push(handle);},
	    ConfigTransport::Slack {
		token,
		bot_token,
		channel_mapping
	    } => {
		let mut instance = Slack::new(transport_id,
					      &bus_map,
					      pipo_id.clone(),
					      db_pool.clone(),
					      token.to_string(),
					      bot_token.to_string(),
					      channel_mapping).await?;
		let handle = tokio::spawn(async move {
		    match instance.connect().await {
			Ok(_) => eprintln!("Slack::connect() exited Ok"),
			Err(e) => {
			    eprintln!("Slack::connect() exited with Error: \
				       {:#}", e);
			}
		    }
		});
		all_transport_tasks.push(handle);
	    },
	    ConfigTransport::Minecraft {
		username: _,
		buses: _
	    } => todo!("Minecraft"),
	    ConfigTransport::Mumble {
		server: _,
		password: _,
		nickname: _,
		client_cert: _,
		server_cert: _,
		comment: _, 
		channel_mapping: _,
		voice_channel_mapping: _
	    } => todo!("Mumble"),
	    ConfigTransport::Rachni {
		server,
		api_key,
		interval,
		buses
	    } => {
		let instance = Rachni::new(transport_id,
					   &bus_map,
					   &server,
					   &api_key,
					   *interval,
					   buses,
					   db_pool.clone(),
					   pipo_id.clone()).await?;
		let handle = tokio::spawn(async move {
		    match instance.run().await {
			Ok(_) => eprintln!("Rachni::run() exited Ok"),
			Err(e) => {
			    eprintln!("Rachni::run() exited with Error: \
				       {:#}", e);
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
	    Err(e) => eprintln!("Task error: {:#}", e)
	}
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    match inner_main().await {
	Ok(_) => (),
	Err(e) => {
	    eprintln!("{:#}", e);
	    ()
	}
    }
}
