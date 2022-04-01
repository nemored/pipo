use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    sync::Arc, io::{SeekFrom, Cursor},
};

use anyhow::{Context, anyhow};
use bytes::BytesMut;
use protobuf::Message as ProtobufMessage;
use tokio::{sync::broadcast, net::TcpStream, fs::File, io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, copy}, time::{Duration, self}};
use tokio_rustls::{TlsConnector, rustls::{ClientConfig, RootCertStore, OwnedTrustAnchor, ServerName, Certificate}, client::TlsStream};
use webpki_roots;

use crate::Message;

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
    channels: HashMap<Arc<String>,broadcast::Sender<Message>>,
    buffer: Vec<u8>
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
                     voice_channel_mapping: &HashMap<Arc<String>,Arc<String>>)
                     -> anyhow::Result<Self>
    {
        let comment = comment.map(|s| s.to_string());
        let channels = channel_mapping.iter()
            .chain(voice_channel_mapping.iter())
            .filter_map(|(channelname, busname)| {
                if let Some(sender) = bus_map.get(busname.as_ref()) {
                    Some((channelname.clone(), sender.clone()))
                }
                else {
                    eprintln!("No bus named '{}' in configuration file.",
                              busname);

                    None
                }
            }).collect();
        let buffer = vec![0; MAX_PAYLOAD];

        Ok(Self {
            transport_id,
            server,
            password,
            nickname,
            client_cert,
            server_cert,
            comment,
            channels,
            buffer
        })
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        let mut delay = 0;
        let mut read_buf = BytesMut::new();
        read_buf.resize(MAX_PAYLOAD, 0);
        let mut message = BytesMut::new();
        loop {
            eprintln!("Sleeping for {} seconds", delay * 30);
            time::sleep(Duration::from_secs(delay * 30)).await;
            delay += 1;
            self.buffer = Vec::new();
            self.buffer.resize(4096, 0);
            let mut stream = match self.connect().await {
                Ok(stream) => stream,
                Err(e) => {
                    eprintln!("Failed to connect to Mumble server. Retrying...");

                    continue;
                }
            };

            let mut timer = time::interval(Duration::from_secs(10));
            loop {
                tokio::select! {
                    ret = stream.read(&mut read_buf) => {
                        match ret {
                            Ok(ret) => {
                                eprintln!("Read {} bytes", ret);
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
                                    eprintln!("read_buf now {} bytes long", read_buf.len());
                                    let len = read_be_u32(&read_buf[2..6]) as usize + 6;
                                    if len > read_buf.len() {
                                        eprintln!("message incomplete");
                                        read_buf.resize(len - read_buf.len(), 0);
                                        message = read_buf.split_to(len);
                                        eprintln!("read_buf now {} bytes long", read_buf.len());
                                        
                                        break;
                                    }
                                    eprintln!("Splitting buffer");
                                    message = read_buf.split_to(len);
                                    eprintln!("read_buf now {} bytes long", read_buf.len());
                                    match Self::handle_protobuf_message(&message).await {
                                        Ok(_) => (),
                                        Err(e) => {
                                            eprintln!("Error: {}", e);
                                            eprintln!("Message type: {}",
                                                      read_be_u16(&read_buf[..2]));
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
                        self.send_ping(&mut stream).await?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn connect(&mut self) -> anyhow::Result<TlsStream<TcpStream>> {
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
        
        Ok(stream)
    }

    async fn handle_protobuf_message(buffer: &[u8]) -> anyhow::Result<()> {
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
            },
            6 => {
                let message = mumble::ChannelRemove::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            7 => {
                let message = mumble::ChannelState::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            8 => {
                let message = mumble::UserRemove::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            9 => {
                let message = mumble::UserState::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            10 => {
                let message = mumble::BanList::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
            },
            11 => {
                let message = mumble::TextMessage::parse_from_bytes(&buffer[OFFSET..OFFSET+length])?;
                print_protobuf_message(&message as &dyn ProtobufMessage);
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

    async fn send_ping(&mut self, stream: &mut TlsStream<TcpStream>)
                       -> anyhow::Result<()> {
        let message = mumble::Ping::new();
        let packet = build_packet(Payload::Ping as u16, &message)?;
        let mut bytes_sent = 0;

        while bytes_sent < packet.len() {
            bytes_sent += stream.write(&packet).await?;
        }

        eprintln!("Sent {} bytes", bytes_sent);

        Ok(())
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
    println!("{}: {:?}", message.descriptor().name(), message);
}

fn build_packet(typ: u16, message: &dyn ProtobufMessage)
                -> anyhow::Result<Vec<u8>> {
    let mut packet = Vec::new();

    packet.extend_from_slice(&typ.to_be_bytes());
    packet.extend_from_slice(&message.compute_size().to_be_bytes());
    message.write_to_vec(&mut packet)?;

    Ok(packet)
}
