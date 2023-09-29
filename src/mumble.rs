use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    sync::{Arc, Mutex}, io::SeekFrom,
};

use anyhow::{Context, anyhow};
use bytes::BytesMut;
use deadpool_sqlite::Pool;
use html_escape;
use protobuf::Message as ProtobufMessage;
use rusqlite::params;
use tokio::{sync::broadcast, net::TcpStream, fs::File, io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt}, time::{Duration, self}};
use tokio_rustls::{TlsConnector, rustls::{ClientConfig, RootCertStore, OwnedTrustAnchor, ServerName, Certificate}, client::TlsStream};
use tokio_stream::{wrappers::BroadcastStream, StreamMap};
use webpki_roots;

use crate::{Message, Attachment};

mod cert_verifier;
mod protocol;

use cert_verifier::MumbleCertVerifier;
use crate::protos::Mumble as mumble;

const TRANSPORT_NAME: &'static str = "Mumble";

const MAX_PAYLOAD: usize = 8 * 1024 * 1024 + 6;
const MUMBLE_VERSION: u32 = 1 << 16 | 4 << 8 | 0;



enum Payload {
    Version,
    UDPTunnel,
    Authenticate,
    Ping,
    Reject,
    ServerSync,
    ChannelRemove,
    ChannelState,
    UserRemove,
    UserState,
    BanList,
    TextMessage,
    PermissionDenied,
    ACL,
    QueryUsers,
    CryptSetup,
    ContextActionModify,
    ContextAction,
    UserList,
    VoiceTarget,
    PermissionQuery,
    CodecVersion,
    UserStats,
    RequestBlob,
    ServerConfig,
    SuggestConfig,
}

pub(crate) struct Mumble {
    transport_id: usize,
    server: Arc<String>,
    password: Arc<Option<String>>,
    nickname: Arc<String>,
    client_cert: Arc<Option<String>>,
    server_cert: Arc<Option<String>>,
    comment: Option<String>,
    stream: Option<TlsStream<TcpStream>>,
    channels: HashMap<Arc<String>,(Option<Arc<mumble::ChannelState>>,broadcast::Sender<Message>)>,
    channel_ids: HashMap<u32,Arc<mumble::ChannelState>>,
    users: HashMap<u32,mumble::UserState>,
    pipo_id: Arc<Mutex<i64>>,
    pool: Pool,
    actor_id: Option<u32>,
}

impl Mumble {
    pub async fn new(transport_id: usize,
                     server: Arc<String>,
                     password: Arc<Option<String>>,
                     nickname: Arc<String>,
                     client_cert: Arc<Option<String>>,
                     server_cert: Arc<Option<String>>,
                     comment: Option<&str>,
                     bus_map: &HashMap<String,broadcast::Sender<Message>>,
                     channel_mapping: &HashMap<Arc<String>,Arc<String>>,
                     _voice_channel_mapping: &HashMap<Arc<String>,Arc<String>>,
                     pipo_id: Arc<Mutex<i64>>,
                     pool: Pool)
                     -> anyhow::Result<Self>
    {
        let comment = comment.map(|s| s.to_string());
        let stream = None;
        let channels = channel_mapping.iter()
            .filter_map(|(channelname, busname)| {
                if let Some(sender) = bus_map.get(busname.as_ref()) {
                    Some((channelname.clone(), (None, sender.clone())))
                }
                else {
                    eprintln!("No bus named '{}' in configuration file.",
                              busname);

                    None
                }
            }).collect();
        let channel_ids = HashMap::new();
        let users = HashMap::new();
        let actor_id = None;

        Ok(Self {
            transport_id,
            server,
            password,
            nickname,
            client_cert,
            server_cert,
            comment,
            stream,
            channels,
            channel_ids,
            users,
            pipo_id,
            pool,
            actor_id,
        })
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        let mut delay = 0;
        let mut read_buf = BytesMut::new();
        read_buf.resize(MAX_PAYLOAD, 0);
        let mut message = BytesMut::new();
        let mut input_buses = StreamMap::new();
        for (channel_name, (_, channel)) in self.channels.iter() {
            input_buses.insert(channel_name.clone(), BroadcastStream::new(channel.subscribe()));
        }
        #[allow(unreachable_code)]
        loop {
            time::sleep(Duration::from_secs(delay * 30)).await;
            delay += 1;
            if let Err(_) = self.connect().await {
                eprintln!("Failed to connect to Mumble server. Retrying...");

                continue;
            }

            let mut timer = time::interval(Duration::from_secs(10));
            loop {
                tokio::select! {
                    ret = self.stream.as_mut().unwrap().read(&mut read_buf) => {
                        match ret {
                            Ok(ret) => {
                                if ret == 0 {
                                    eprintln!("Socket closed.");
                                    eprintln!("Reconnecting...");

                                    break;
                                }
                                read_buf.truncate(ret);
                                while read_buf.len() > 0 {
                                    if message.len() > 0 {
                                        message.unsplit(read_buf);
                                        read_buf = message;
                                    }
                                    let len = read_be_u32(&read_buf[2..6]) as usize + 6;
                                    if len > read_buf.len() {
                                        read_buf.resize(len - read_buf.len(), 0);
                                        message = read_buf.split_to(len);
                                        
                                        break;
                                    }
                                    message = read_buf.split_to(len);
                                    match self.handle_protobuf_message(&message).await {
                                        Ok(_) => (),
                                        Err(e) => {
                                            eprintln!("Error: {}", e);
                                            eprintln!("Message type: {}",
                                                      read_be_u16(&message[..2]));
                                            eprintln!("Reconnecting...");

                                            break;
                                        }
                                    }
                                    message.truncate(0);
                                }
                                read_buf.resize(MAX_PAYLOAD, 0);
                            },
                            Err(e) => {
                                eprintln!("Error reading from socket: {}", e);
                                eprintln!("Reconnecting...");

                                break;
                            }
                        }
                    }
                    _ = timer.tick() => {
                        self.send_ping().await?;
                    }
                    Some((channel, message)) = tokio_stream::StreamExt::next(&mut input_buses) => {
                        let message = message.unwrap();
                        match self.handle_pipo_message(&channel, message).await {
                            Ok(_) => (),
                            Err(e) => {
                                eprintln!("Error handling PIPO Message: {}", e);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn connect(&mut self) -> anyhow::Result<()> {
        eprintln!("Connecting...");
        let mut root_store = RootCertStore::empty();
        root_store.add_server_trust_anchors(webpki_roots::TLS_SERVER_ROOTS
                                            .0
                                            .iter()
                                            .map(|ta| {
                                                OwnedTrustAnchor::from_subject_spki_name_constraints(ta.subject,
                                                                                                     ta.spki,
                                                                                                     ta.name_constraints)
                                            }));
        let config;
        if let Some(path) = self.server_cert.as_ref() {
            let mut file = File::open(path).await?;
            file.seek(SeekFrom::End(0)).await?;
            let len = file.stream_position().await? as usize;
            file.seek(SeekFrom::Start(0)).await?;
            let mut cert = Certificate(Vec::new());
            cert.0.resize(len, 0);
            let mut bytes_read = 0;
            while bytes_read < len {
                bytes_read += file.read(cert.0.as_mut()).await?;
            }
            root_store.add(&cert)?;
            config = ClientConfig::builder()
                .with_safe_defaults()
                .with_custom_certificate_verifier(Arc::new(MumbleCertVerifier::new(cert)))
                .with_no_client_auth();

        }
        else {
            config = ClientConfig::builder()
                .with_safe_defaults()
                .with_root_certificates(root_store)
                .with_no_client_auth();
        }
        let config = TlsConnector::from(Arc::new(config));
        let (hostname, port) = self.server
            .split_once(":")
            .or(Some((&self.server, "64738")))
            .unwrap();
        let dns_name = ServerName::try_from(hostname)?;
        let socket = TcpStream::connect(self.server.as_ref()).await
            .context("Failed to connect to the TCP socket")?;
        let mut stream = config.connect(dns_name, socket).await?;
        let mut message = mumble::Version::new();
        message.set_version(MUMBLE_VERSION);
        message.set_release(String::from("PIPO"));
        message.set_os(String::from("PIPO"));
        message.set_os_version(String::from("PIPO"));
        let mut version = vec![0, 0];
        version.extend_from_slice(&message.compute_size().to_be_bytes());
        message.write_to_vec(&mut version)?;
        let mut message = mumble::Authenticate::new();
        message.set_username(self.nickname.as_ref().clone());
        if let Some(password) = self.password.as_ref() {
            message.set_password(password.clone());
        }
        let mut authenticate = vec![0, 2];
        authenticate.extend_from_slice(&message.compute_size().to_be_bytes());
        message.write_to_vec(&mut authenticate)?;
        let mut bytes_sent = 0;
        while bytes_sent < version.len() {
            bytes_sent += stream.write(&version).await?;
        }
        bytes_sent = 0;
        while bytes_sent < authenticate.len() {
            bytes_sent += stream.write(&authenticate).await?;
        }
        self.stream = Some(stream);

        Ok(())
    }

    async fn handle_protobuf_message(&mut self, buffer: &[u8]) -> anyhow::Result<()> {
        const OFFSET: usize = 6;

        let length = read_be_u32(&buffer[2..6]) as usize;
        
        match read_be_u16(&buffer[..2]) {
            0 => {
                let message = mumble::Version::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            1 => {
                // UDPTunnel: Ignored
                ()
            },
            2 => {
                let message = mumble::Authenticate::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            3 => {
                let message = mumble::Ping::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            4 => {
                let message = mumble::Reject::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
                return Err(anyhow!("Server rejected the connection."));
            },
            5 => {
                let message = mumble::ServerSync::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
                self.handle_server_sync_message(message).await;
            },
            6 => {
                let message = mumble::ChannelRemove::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
                self.handle_channel_remove_message(message).await;
            },
            7 => {
                let message = mumble::ChannelState::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
                self.handle_channel_state_message(message).await;
            },
            8 => {
                let message = mumble::UserRemove::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
                self.handle_user_remove_message(message).await;
            },
            9 => {
                let message = mumble::UserState::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
                self.handle_user_state_message(message).await;
            },
            10 => {
                let message = mumble::BanList::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            11 => {
                let message = mumble::TextMessage::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
                self.handle_text_message_message(message).await;
            },
            12 => {
                let message = mumble::PermissionDenied::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            13 => {
                let message = mumble::ACL::parse_from_bytes(&buffer[OFFSET..length+OFFSET])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            14 => {
                let message = mumble::QueryUsers::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            15 => {
                let message = mumble::CryptSetup::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            16 => {
                let message = mumble::ContextActionModify::parse_from_bytes(&buffer[6..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            17 => {
                let message = mumble::ContextAction::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            18 => {
                let message = mumble::UserList::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            19 => {
                let message = mumble::VoiceTarget::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            20 => {
                let message = mumble::PermissionQuery::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            21 => {
                let message = mumble::CodecVersion::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            22 => {
                let message = mumble::UserStats::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            23 => {
                let message = mumble::RequestBlob::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            24 => {
                let message = mumble::ServerConfig::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            25 => {
                let message = mumble::SuggestConfig::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            _ => unimplemented!()
        }

        Ok(())
    }

    async fn handle_server_sync_message(&mut self, message: mumble::ServerSync) {
        let actor_id = match self.actor_id {
            Some(id) => id,
            None => return
        };
        let mut user = mumble::UserState::new();
        user.set_actor(actor_id);
        user.set_self_mute(true);
        user.set_self_deaf(true);
        user.set_listening_channel_add(self.channel_ids.keys().map(|id| id.clone()).collect::<Vec<u32>>());

        let packet = match build_packet(Payload::UserState as u16, &user) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("failed to build packet: {}", e);

                return;
            }
        };
        let mut bytes_sent = 0;

        while bytes_sent < packet.len() {
            match self.stream.as_mut().unwrap().write(&packet).await {
                Ok(b) => bytes_sent += b,
                Err(e) => {
                    eprintln!("failed to send PIPO's UserState to server: {}", e);

                    return;
                }
            }
        }
    }

    async fn handle_channel_state_message(&mut self, mut message: mumble::ChannelState) {
        let name = message.get_name().to_string();
        let channel_id = message.get_channel_id();

        if let Some(mut channel) = self.channels.get_mut(&name) {
            if let Some(channel) = self.channel_ids.get(&channel_id) {
                (|| message.merge_from_bytes(&channel.write_to_bytes()?))().unwrap();
            }

            let rc = Arc::new(message);
            
            self.channel_ids.remove(&channel_id);
            channel.0 = None;
        }
    }

    async fn handle_channel_remove_message(&mut self, message: mumble::ChannelRemove) {
        if let Some(channel) = self.channel_ids.remove(&message.get_channel_id()) {
            if let Some(mut channel) = self.channels.get_mut(&channel.get_name().to_string()) {
                channel.0 = None;
            }
        }
    }    

    async fn handle_user_state_message(&mut self, mut message: mumble::UserState) {
        // Use the session ID as the actor index because ???
        let session = message.get_session();

        if let Some(user) = self.users.get(&session) {
            (|| message.merge_from_bytes(&user.write_to_bytes()?))().unwrap();
        }

        self.users.insert(session, message);
        if let Some(user) = self.users.get(&session) {
            if user.get_name() == self.nickname.as_ref() {
                self.actor_id = Some(session);
            }
        }
    }

    async fn handle_user_remove_message(&mut self, message: mumble::UserRemove) {
        // Use the session ID as the actor index because ???
        let session = message.get_session();

        self.users.remove(&session);
    }    

    async fn handle_text_message_message(&mut self, message: mumble::TextMessage)
        -> anyhow::Result<()> {
        for channel_id in message.get_channel_id().iter() {
            if let Some(channel) = self.channel_ids.get(&channel_id) {
                if let Some(bus) = self.channels.get(&channel.get_name().to_string()) {
                    let pipo_id = self.insert_into_messages_table().await?;
                    let username = self.users.get(&message.get_actor())
                        .ok_or(anyhow!("No user found for actor ID {}", message.get_actor()))?
                        .get_name()
                        .to_string();

                    let message = Message::Text {
                        sender: self.transport_id,
                        pipo_id,
                        transport: TRANSPORT_NAME.to_string(),
                        username,
                        avatar_url: None,
                        thread: None,
                        message: Some(html_escape::decode_html_entities(message.get_message()).to_string()),
                        attachments: None,
                        is_edit: false,
                        irc_flag: false,
                    };
                    
                    bus.1
                        .send(message)
                        .context(format!("Couldn't send message"))?;
                }
            }
        }

        Ok(())
    }

    async fn send_ping(&mut self) -> anyhow::Result<()> {
        let message = mumble::Ping::new();
        let packet = build_packet(Payload::Ping as u16, &message)?;
        let mut bytes_sent = 0;

        while bytes_sent < packet.len() {
            bytes_sent += self.stream.as_mut().unwrap().write(&packet).await?;
        }

        Ok(())
    }

    async fn handle_pipo_message(&mut self,
                                 channel: &str,
                                 message: Message)
                                 -> anyhow::Result<()> {
        match message {
	    Message::Action {
		sender,
		transport,
		username,
		message,
		attachments,
		is_edit,
                ..
            } => {
		if sender != self.transport_id {
                    self.handle_pipo_action_message(&channel,
                                                    &transport,
                                                    &username,
                                                    message.as_deref(),
                                                    attachments,
                                                    is_edit).await
                        .context("Failed to send TextMessage to Mumble")?;
		}

                Ok(())
	    },
	    Message::Bot { .. } => {
                // TODO
                Ok(())
	    },
	    Message::Delete { .. } => {
                // Not handled
                Ok(())
	    },
	    Message::Names {
		sender,
		transport: _,
		username,
		message,
	    } => {
                // TODO
                Ok(())
	    },
	    Message::Pin { .. } => {
                // Not handled
                Ok(())
	    },
	    Message::Reaction { .. } => {
                // Not handled
                Ok(())
	    },
	    Message::Text {
		sender,
		transport,
		username,
		message,
		attachments,
		is_edit,
                ..
	    } => {
		if sender != self.transport_id {
                    self.handle_pipo_text_message(&channel,
                                                  &transport,
                                                  &username,
                                                  message.as_deref(),
                                                  attachments,
                                                  is_edit).await
                    .context("Failed to send TextMessage to Mumble")?;
		}

                Ok(())
	    },
	}

    }

    async fn handle_pipo_action_message(&mut self,
                                        channel: &str,
                                        transport: &str,
                                        username: &str,
                                        message: Option<&str>,
                                        _attachments: Option<Vec<Attachment>>,
                                        is_edit: bool) -> anyhow::Result<()> {
        let message_text = format!("{}<i>*{}!<font color=\"#3ae\"><b>{}</b></font> {}</i>",
                                   if is_edit { "<b>EDIT:</b> "} else { "" },
                                   html_escape::encode_text(&transport[..1].to_uppercase()),
                                   html_escape::encode_text(username),
                                   html_escape::encode_text(message.ok_or(anyhow!("Action Message contains no message"))?));
        let actor_id = self.actor_id.ok_or(anyhow!("PIPO does not have an actor ID"))?;
        if let Some(channel_id) = (|| self.channels.get(&channel.to_string())?.0.as_ref())() {
            let mut message = mumble::TextMessage::new();
            message.set_actor(actor_id);
            message.set_channel_id(Vec::from([channel_id.get_channel_id()]));
            message.set_message(message_text);
            let packet = build_packet(Payload::TextMessage as u16, &message)?;
            let mut bytes_sent = 0;

            while bytes_sent < packet.len() {
                bytes_sent += self.stream.as_mut().unwrap().write(&packet).await?;
            }

            return Ok(())
        }

        Err(anyhow!("Couldn't find channel ID for channel {}", channel))
    }

    async fn handle_pipo_text_message(&mut self,
                                      channel: &str,
                                      transport: &str,
                                      username: &str,
                                      message: Option<&str>,
                                      _attachments: Option<Vec<Attachment>>,
                                      is_edit: bool) -> anyhow::Result<()> {
        let message_text = format!("{}!<font color=\"#3ae\"><b>{}</b></font>: {}{}",
                                   html_escape::encode_text(&transport[..1].to_uppercase()),
                                   html_escape::encode_text(username),
                                   if is_edit { "<b>EDIT:</b> "} else { "" },
                                   html_escape::encode_text(message.ok_or(anyhow!("TextMessage contains no message"))?));
        let actor_id = self.actor_id.ok_or(anyhow!("PIPO does not have an actor ID"))?;
        if let Some(channel_id) = (|| self.channels.get(&channel.to_string())?.0.as_ref())() {
            let mut message = mumble::TextMessage::new();
            message.set_actor(actor_id);
            message.set_channel_id(Vec::from([channel_id.get_channel_id()]));
            message.set_message(message_text);
            let packet = build_packet(Payload::TextMessage as u16, &message)?;
            let mut bytes_sent = 0;

            while bytes_sent < packet.len() {
                bytes_sent += self.stream.as_mut().unwrap().write(&packet).await?;
            }

            return Ok(())
        }

        Err(anyhow!("Couldn't find channel ID for channel {}", channel))
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
}

fn read_be_u16(input: &[u8]) -> u16 {
    let (int_bytes, _) = input.split_at(std::mem::size_of::<u16>());
    u16::from_be_bytes(int_bytes.try_into().unwrap())
}

fn read_be_u32(input: &[u8]) -> u32 {
    let (int_bytes, _) = input.split_at(std::mem::size_of::<u32>());
    u32::from_be_bytes(int_bytes.try_into().unwrap())
}

fn print_protobuf_message(message: &dyn ProtobufMessage) {
    if cfg!(debug_assertions) {
        println!("{}: {:?}", message.descriptor().name(), message);
    }
}

fn build_packet(typ: u16, message: &dyn ProtobufMessage)
                -> anyhow::Result<Vec<u8>> {
    let mut packet = Vec::new();

    packet.extend_from_slice(&typ.to_be_bytes());
    packet.extend_from_slice(&message.compute_size().to_be_bytes());
    message.write_to_vec(&mut packet)?;

    Ok(packet)
}
