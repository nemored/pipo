use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
};

use tokio::sync::broadcast;

use crate::Message;

mod protocol;

const TRANSPORT_NAME: &'static str = "Mumble";

pub(crate) struct Mumble {
    transport_id: usize,
    server: SocketAddr,
    password: Arc<Option<String>>,
    nickname: Arc<String>,
    client_cert: Arc<Option<String>>,
    server_cert: Arc<Option<String>>,
    comment: Option<String>,
    channels: HashMap<Arc<String>,broadcast::Sender<Message>>,
}

impl Mumble {
    pub async fn new(transport_id: usize,
                     server: &str,
                     password: Arc<Option<String>>,
                     nickname: Arc<String>,
                     client_cert: Arc<Option<String>>,
                     server_cert: Arc<Option<String>>,
                     comment: Option<&str>,
                     bus_map: &HashMap<String,broadcast::Sender<Message>>,
                     channel_mapping: &HashMap<Arc<String>,Arc<String>>,
                     voice_channel_mapping: &HashMap<Arc<String>,Arc<String>>)
                     -> anyhow::Result<Mumble>
    {
        let server = tokio::net::lookup_host(server).await?
            .next()
            .unwrap();
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

        Ok(Self {
            transport_id,
            server,
            password,
            nickname,
            client_cert,
            server_cert,
            comment,
            channels,
        })
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        

        Ok(())
    }
}
