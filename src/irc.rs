use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use deadpool_sqlite::Pool;
use irc::{
    client::prelude::{
	Client,
	Command,
	Config,
	Prefix,
    },
    proto::caps::Capability,
};
use lazy_static::lazy_static;
use regex::Regex;
use rusqlite::params;
use tokio::sync::mpsc;

use anyhow::anyhow;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use crate::{
    Attachment,
    Message, Bus,
};


const TRANSPORT_NAME: &'static str = "IRC";

pub(crate) struct IRC {
    transport_id: usize,
    config: Config,
    img_root: String,
    channel_map: HashMap<String,(Arc<Bus>,mpsc::Sender<Message>)>,
    bus_map: HashMap<Arc<Bus>,String>,
    pool: Pool,
    inbox: ReceiverStream<Message>,
    pipo_id: Arc<Mutex<i64>>
}

impl IRC {
    pub async fn new(bus_map: &HashMap<Arc<Bus>,(mpsc::Sender<Message>, mpsc::Receiver<Message>)>,
                     inbox: mpsc::Receiver<Message>,
		     pipo_id: Arc<Mutex<i64>>,
		     pool: Pool,
		     nickname: String,
		     server: String,
		     use_tls: bool,
		     img_root: &str,
		     channel_mapping: &HashMap<Arc<String>,Arc<Bus>>,
		     transport_id: usize)
	-> anyhow::Result<IRC> {
        let inbox = ReceiverStream::new(inbox);
	let channel_map: HashMap<String,(Arc<Bus>,mpsc::Sender<Message>)> = channel_mapping.iter()
	    .filter_map(|(channelname, bus)| {
		if let Some((sender, _)) = bus_map.get(bus.as_ref()) {
		    Some((channelname.as_ref().clone(), (bus.clone(), sender.clone())))
		}
		else {
		    eprintln!("No bus named '{}' in configuration file.",
			      bus.name);
		    None
		}
	    }
	    ).collect();
        let bus_map = channel_map
            .iter()
            .map(|(a, (x, _))| (x.clone(), a.clone()))
            .collect();
	// Can this be done without an if/else?
	let config = if let Some((server_addr, server_port))
	    = server.rsplit_once(':') {
		Config {
		    nickname: Some(nickname.clone()).to_owned(),
		    server: Some(server_addr.to_string()).to_owned(),
		    port: Some(server_port.parse()?).to_owned(),
		    use_tls: Some(use_tls).to_owned(),
		    ..Config::default()
		}
	    }
	else {
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
	    channel_map,
            bus_map,
	    transport_id,
	    pool,
            inbox,
	    pipo_id
	})
    }

    pub async fn connect(&mut self) -> anyhow::Result<()> {
	loop {
	let (client, mut irc_stream)
	    = self.connect_irc().await?;

	loop {
	    // stupid sexy infinite loop
	    tokio::select! {
		Some(message)
		    = self.inbox.next() => {
			match message {
			    Message::Action {
				sender,
				pipo_id: _,
				transport,
                                bus,
				username,
				avatar_url: _,
				thread: _,
				message,
				attachments,
				is_edit,
				irc_flag,
			    } => if let Some(channel) = self.bus_map.get(&bus) {
				if sender != self.transport_id {
				    self.handle_action_message(&client,
							       channel,
							       transport,
							       username,
							       message,
							       attachments,
							       is_edit,
							       irc_flag);
				}
			    }
                            else {
                                eprintln!("No channel for bus {}", bus.name);
                            },
			    Message::Bot {
				sender,
				pipo_id: _,
				transport,
                                bus,
				message,
				attachments,
				is_edit,
			    } => if let Some(channel) = self.bus_map.get(&bus) {
				if sender != self.transport_id {
				    self.handle_bot_message(&client,
							    channel,
							    transport,
							    message,
							    attachments,
							    is_edit);
				}
			    }
                            else {
                                eprintln!("No channel for bus {}", bus.name);
                            },
			    Message::Delete {
				sender: _,
				pipo_id: _,
				transport: _,
                                bus: _,
			    } => {
				continue
			    },
			    Message::Names {
				sender,
				transport: _,
                                bus,
				username,
				message,
			    } => if let Some(channel) = self.bus_map.get(&bus) {
				if sender != self.transport_id {
				    self.handle_names_message(&client,
							      channel,
							      username,
							      message);
				}
			    }
                            else {
                                eprintln!("No channel for bus {}", bus.name);
                            },
			    Message::Pin {
				sender: _,
				pipo_id: _,
                                bus: _,
				remove: _,
			    } => {
				continue
			    },
			    Message::Reaction {
				sender: _,
				pipo_id: _,
				transport: _,
                                bus: _,
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
				pipo_id: _,
				transport,
                                bus,
				username,
				avatar_url: _,
				thread: _,
				message,
				attachments,
				is_edit,
				irc_flag,
			    } => if let Some(channel) = self.bus_map.get(&bus) {
				if sender != self.transport_id {
				    self.handle_text_message(&client,
							     channel,
							     transport,
							     username,
							     message,
							     attachments,
							     is_edit,
							     irc_flag);
				}
			    }
                            else {
                                eprintln!("No channel for bus {}", bus.name);
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
			    Some(Prefix::Nickname(nickname, _, _)) => nickname,
			    Some(Prefix::ServerName(servername)) => servername,
			    None => "".to_string(),
			};
			if let Command::PRIVMSG(channel, message)
			    = message.command {
				if let Err(e) = self.handle_priv_msg(nickname,
								     channel,
								     message)
				    .await {
					eprintln!("Error handling PRIVMSG: {}",
						  e);
				    }
			    }
			else if let Command::NOTICE(channel, message)
			    = message.command {
				if let Err(e) = self.handle_notice(nickname,
								   channel,
								   message)
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

    fn handle_action_message(&self, client: &Client, channel: &str,
			     transport: String, username: String,
			     message: Option<String>,
			     attachments: Option<Vec<Attachment>>,
			     is_edit: bool, irc_flag: bool) {
	let mut message = message;

	if irc_flag && is_edit { message = None }
	if let Some(message) = message {
	    let mut is_edit = is_edit;

	    for msg in message.split("\n") {
		if msg == "" { continue }

		let message = if is_edit {
		    is_edit = false;
		    
		    format!("\x01ACTION \x02* \x02{}!\x02{}\x02 {}*\x01",
			    &transport[..1].to_uppercase(), username, msg)
		}
		else {
		    format!("\x01ACTION \x02* \x02{}!\x02{}\x02 {}\x01",
			    &transport[..1].to_uppercase(), username, msg)
		};
		
		if let Err(e) = client.send_privmsg(channel.clone(),
					  message.clone()) {
		    eprintln!("Failed to send message '{}' channel {}: {:#}",
			      message, channel, e);
		};
	    }
	}

	if let Some(attachments) = attachments {
	    IRC::handle_attachments(client, channel, attachments);
	}
    }

    fn handle_bot_message(&self, client: &Client, channel: &str,
			   _transport: String, _message: Option<String>,
			   attachments: Option<Vec<Attachment>>,
			   is_edit: bool) {
	if !is_edit { return }
	
	if let Some(attachment) = attachments {
	    IRC::handle_attachments(client, channel, attachment);
	}
    }

    async fn handle_names_message(&self, client: &Client, channel: &str,
			    username: String, message: Option<String>) {
	if message == Some("/names".to_string()) {
	    if let Some(users) = client.list_users(channel) {
		let users: Vec<String> = users.into_iter().map(|user| {
		    user.get_nickname().to_string()
		}).collect();
                if let Some((bus, sender)) = self.channel_map.get(channel) {
		    let message = Message::Names {
		        sender: self.transport_id,
		        transport: TRANSPORT_NAME.to_string(),
                        bus: bus.as_ref().to_owned(),
		        username: username.to_string(),
		        message: Some(serde_json::json!(users).to_string()),
		    };

		    eprintln!("Sending message: {:#}", message);
		    if let Err(e) = sender.send(message).await {
			eprintln!("Couldn't send message: {:#}", e);
		    }
                }
	    }
	}
    }

    fn handle_text_message(&self, client: &Client, channel: &str,
			   transport: String, username: String,
			   message: Option<String>,
			   attachments: Option<Vec<Attachment>>,
			   is_edit: bool, irc_flag: bool) {
	let mut message = message;

	if irc_flag && is_edit { message = None }
	if let Some(message) = message {
	    let mut is_edit = is_edit;
	
	    for msg in message.split("\n") {
		if msg == "" { continue }
		
		let message = if is_edit {
		    is_edit = false;
		    
		    format!("\x01ACTION <{}!\x02{}\x02> \x02EDIT:\x02 {}\x01",
			    &transport[..1].to_uppercase(), username, msg)
		}
		else {
		    format!("\x01ACTION <{}!\x02{}\x02> {}\x01",
			    &transport[..1].to_uppercase(), username, msg)
		};
		
		if let Err(e) = client.send_privmsg(channel.clone(),
						    message.clone()) {
		    eprintln!("Failed to send message '{}' channel {}: {:#}",
			      message, channel, e);
		}
	    }
	}
	
	if let Some(attachment) = attachments {
	    IRC::handle_attachments(client, channel, attachment);
	}
    }

    fn handle_attachments(client: &Client, channel: &str,
			  attachments: Vec<Attachment>) {
	for attachment in attachments {
	    let has_text = attachment.text.is_some();
	    let has_fallback = attachment.fallback.is_some();
	    let service_name = match attachment.service_name {
		Some(s) => s,
		None => String::from("Unknown")
	    };
	    let author_name = match attachment.author_name {
		Some(s) => s,
		None => String::new()
	    };
	    let text = match attachment.text {
		Some(s) => s,
		None => match attachment.fallback {
		    Some(s) => s,
		    None => continue
		}
	    };
	    
	    if !has_text && !has_fallback { continue }

	    let mut line_counter = 0;
	    
	    for msg in text.split("\n") {
		line_counter += 1;

		if msg == "" { continue }
		
		let message = if author_name.is_empty() {
		    format!("\x01ACTION [\x02{}\x02] {}\x01", service_name,
			    msg)
		}
		else {
		    format!("\x01ACTION [{}!\x02{}\x02] {}\x01",
			    &service_name[..1].to_uppercase(), author_name,
			    msg)
		};

		if let Err(e) = client.send_privmsg(channel.clone(),
						    message.clone()) {
		    eprintln!("Failed to send message '{}' channel {}: {:#}",
			      message, channel, e);
		}

		if line_counter > 6 { break }
	    }
	}
    }

    async fn connect_irc(&mut self)
	-> anyhow::Result<(Client,
			   irc::client::ClientStream)> {
	    let mut client = Client::from_config(self.config.clone()).await?;

	    client.send_cap_req(&[Capability::MultiPrefix])?;
	    client.identify()?;

	    let irc_stream = client.stream()?;
	    for channel_name in self.channel_map.keys() {
		if let Err(e) = client.send_join(channel_name) {
		    eprintln!("Failed to join channel {}: {:#}", channel_name,
			      e);
		}
	    }

	    Ok((client, irc_stream))
	}

    async fn get_avatar_url(&self, nickname: &str) -> String {
	let client = reqwest::Client::new();
	let url = format!("{}/{}.png", self.img_root, nickname);

	let response = match client.head(url).send().await {
	    Ok(response) => response,
	    Err(_) => return format!("{}/irc.png", self.img_root)
	};

	if let Some(etag) = response.headers().get(reqwest::header::ETAG) {
	    if let Ok(etag) = etag.to_str() {
		return format!("{}/{}.png?{}", self.img_root, nickname, etag)
	    }
	}

	return format!("{}/{}.png", self.img_root, nickname)
    }

    async fn handle_priv_msg(&self,
			     nickname: String,
			     channel: String,
			     message: String) -> anyhow::Result<()> {
	if let Some((bus, sender)) = self.channel_map.get(&channel) {
	    lazy_static! {
		static ref RE: Regex
		    =  Regex::new("^\x01ACTION (.*)\x01\r?$").unwrap();
	    }
	    let pipo_id = self.insert_into_messages_table().await?;

	    let avatar_url = self.get_avatar_url(&nickname).await;

	    eprintln!("IRC PIPO ID: {}", pipo_id);
	    
	    if let Some(message) = RE.captures(&message) {
		let message = message.get(1).unwrap().as_str();
		let message = Message::Action {
		    sender: self.transport_id,
		    pipo_id,
		    transport: TRANSPORT_NAME.to_string(),
                    bus: bus.as_ref().to_owned(),
		    username: nickname.clone(),
		    avatar_url: Some(avatar_url),
		    thread: None,
		    message: Some(message.to_string()),
		    attachments: None,
		    is_edit: false,
		    irc_flag: false,
		};
		return match sender.send(message).await {
		    Ok(_) => Ok(()),
		    Err(e) => Err(anyhow!("Couldn't send message: {:#}", e))
		}
	    }
	    else {
		let message = Message::Text {
		    sender: self.transport_id,
		    pipo_id,
		    transport: TRANSPORT_NAME.to_string(),
                    bus: bus.as_ref().to_owned(),
		    username: nickname.clone(),
		    avatar_url: Some(avatar_url),
		    thread: None,
		    message: Some(message.to_string()),
		    attachments: None,
		    is_edit: false,
		    irc_flag: false,
		};
		return match sender.send(message).await {
		    Ok(_) => Ok(()),
		    Err(e) => Err(anyhow!("Couldn't send message: {:#}", e))
		}
	    }
	}
	else {
	    return Err(anyhow!("Could not get sender for channel {}", channel))
	}
    }

    async fn insert_into_messages_table(&self) -> anyhow::Result<i64> {
	let conn = self.pool.get().await.unwrap();
	let pipo_id = *self.pipo_id.lock().unwrap();

	// TODO: ugly error handling needs fixing
	match conn.interact(move |conn| -> anyhow::Result<usize> {
		Ok(conn.execute("INSERT OR REPLACE INTO messages (id) 
                                 VALUES (?1)", params![pipo_id])?)
	}).await {
		Ok(res) => res,
		Err(_) => Err(anyhow!("Interact Error"))	
	}?;

	let ret = pipo_id;
	let mut pipo_id = self.pipo_id.lock().unwrap();
	*pipo_id += 1;
	if *pipo_id > 40000 { *pipo_id = 0 }

	Ok(ret)
    }

    async fn handle_notice(&self,
			   nickname: String,
			   channel: String,
			   message: String) -> anyhow::Result<()> {
	if let Some((bus, sender)) = self.channel_map.get(&channel) {
	    lazy_static! {
		static ref RE: Regex
		    =  Regex::new("^\x01ACTION (.*)\x01\r?$").unwrap();
	    }
	    let pipo_id = self.insert_into_messages_table().await?;	    
	    
	    let avatar_url = self.get_avatar_url(&nickname).await;

	    if let Some(message) = RE.captures(&message) {
		let message = format!("```{}```",
				      message.get(1).unwrap().as_str());
		let message = Message::Action {
		    sender: self.transport_id,
		    pipo_id,
		    transport: TRANSPORT_NAME.to_string(),
                    bus: bus.as_ref().to_owned(),
		    username: nickname.clone(),
		    avatar_url: Some(avatar_url),
		    thread: None,
		    message: Some(message.to_string()),
		    attachments: None,
		    is_edit: false,
		    irc_flag: false,
		};
		return match sender.send(message).await {
		    Ok(_) => Ok(()),
		    Err(e) => Err(anyhow!("Couldn't send message: {:#}", e))
		}
	    }
	    else {
		let message = Message::Text {
		    sender: self.transport_id,
		    pipo_id,
		    transport: TRANSPORT_NAME.to_string(),
                    bus: bus.as_ref().to_owned(),
		    username: nickname.clone(),
		    avatar_url: Some(avatar_url),
		    thread: None,
		    message: Some(format!("```{}```",
					  message.to_string())),
		    attachments: None,
		    is_edit: false,
		    irc_flag: false,
		};
		return match sender.send(message).await {
		    Ok(_) => Ok(()),
		    Err(e) => Err(anyhow!("Couldn't send message: {:#}", e))
		}
	    }
	}
	else {
	    return Err(anyhow!("Could not get sender for channel {}", channel))
	}
    }
}

