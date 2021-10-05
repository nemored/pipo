use std::{
    collections::{
	HashMap,
	HashSet
    },
    sync::{
	Arc,
	Mutex
    }
};

use deadpool_sqlite::Pool;
use reqwest::{
    Client as HttpClient,
    Method
};
use rusqlite::params;
use serde_json::Value;
use tokio::{
    sync::broadcast,
    time::{
	self,
	Duration
    }
};

use crate::Message;

const TRANSPORT_NAME: &'static str = "Rachni";

pub(crate) struct Rachni {
    transport_id: usize,
    server: String,
    api_key: String,
    interval: u64,
    bus_map: HashMap<String, broadcast::Sender<Message>>,
    pool: Pool,
    pipo_id: Arc<Mutex<i64>>
}

impl Rachni {
    pub async fn new(transport_id: usize,
		     bus_map: &HashMap<String, broadcast::Sender<Message>>,
		     server: &str, api_key: &str, interval: u64,
		     buses: &Vec<String>, pool: Pool, pipo_id: Arc<Mutex<i64>>)
	-> anyhow::Result<Rachni> {
	let server = String::from(server);
	let api_key = String::from(api_key);
	let bus_map = buses.iter().filter_map(|bus| {
	    bus_map.get(bus).map(|sender| (bus.clone(), sender.clone()))
	}).collect();

	Ok(Rachni { transport_id, server, api_key, interval, bus_map, pool,
		    pipo_id })
    }

    pub async fn run(&self) -> anyhow::Result<()> {
	let mut streams = HashSet::new();
	let http = HttpClient::new();
	let url = format!("http://{}/api/{}/stream/ping", self.server,
			  self.api_key);

	loop {
	    let mut new_stream_map = HashSet::new();
	    let response = http.request(Method::GET, &url).send().await?;
	    let json: Value = serde_json::from_str(response.text().await?
						   .as_str())?;

	    if let Some(map) = json.as_object() {
		for (stream, details) in map {
		    let stream = stream.as_str();

		    new_stream_map.insert(stream.to_string());
		    
		    if let None = streams.get(stream) {
			let url = match details["URL"].as_str() {
			    Some(s) => s,
			    None => {
				eprintln!("Stream URL is not a string.");
				continue
			    }
			};
			let message = format!("has started streaming: {} ",
					      url);
			    
			if let Err(e) = self.send_message(&stream, &message)
			    .await {
				eprintln!("Rachni couldn't send message: {}",
					  e);
			    }
		    }
		}
		
		for stream in streams.difference(&new_stream_map) {
		    let message = "has stopped streaming";
			    
		    if let Err(e) = self.send_message(&stream, message)
			.await {
			    eprintln!("Rachni couldn't send message: {}", e);
			}
		}
	    }
	    else {
		for stream in streams.iter() {
		    let message = "has stopped streaming";

		    if let Err(e) = self.send_message(&stream, message)
			.await {
			    eprintln!("Rachni couldn't send message: {}", e);
			}
		}
	    }

	    streams = new_stream_map;
	    
	    time::sleep(Duration::from_secs(self.interval)).await;
	}
    }

    async fn send_message(&self, username: &str, message: &str)
	-> anyhow::Result<()> {
	let pipo_id = self.insert_into_messages_table().await?;
	let message = Message::Action {
	    sender: self.transport_id,
	    pipo_id,
	    transport: TRANSPORT_NAME.to_string(),
	    username: username.to_string(),
	    avatar_url: None,
	    thread: None,
	    message: Some(message.to_string()),
	    attachments: None,
	    is_edit: false,
	    irc_flag: false,
	};
	
	for (_, sender) in self.bus_map.iter() {
	    sender.send(message.clone())?;
	}

	Ok(())
    }

    async fn insert_into_messages_table(&self) -> anyhow::Result<i64> {
	let conn = self.pool.get().await.unwrap();
	let pipo_id = *self.pipo_id.lock().unwrap();

	conn.interact(move |conn| -> anyhow::Result<usize> {
		Ok(conn.execute("INSERT OR REPLACE INTO messages (id) 
                                 VALUES (?1)", params![pipo_id])?)
	}).await?;

	let mut pipo_id = self.pipo_id.lock().unwrap();
	*pipo_id += 1;
	if *pipo_id > 40000 { *pipo_id = 0 }

	Ok(*pipo_id)
    }
}
