use std::collections::HashMap;

use tokio::{sync::broadcast, net::ToSocketAddrs};

use crate::Message;

mod protocol;

const TRANSPORT_NAME: &'static str = "Mumble";

pub(crate) struct Mumble<'a, A: ToSocketAddrs> {
    transport_id: usize,
    server: A,
    password: Option<&'a str>,
    nickname: &'a str,
    client_cert: Option<&'a str>,
    server_cert: Option<&'a str>,
    comment: Option<String>,
    channels: HashMap<String,broadcast::Sender<Message>>,
}

impl<'a, A: ToSocketAddrs> Mumble<'a, A> {
    pub async fn new<S>(transport_id: usize,
                        server: A,
                        password: Option<&'a S>,
                        nickname: &'a S,
                        client_cert: Option<&'a S>,
                        server_cert: Option<&'a S>,
                        comment: Option<&S>,
                        bus_map: &HashMap<String,broadcast::Sender<Message>>,
                        channel_mapping: &HashMap<String,String>,
                        voice_channel_mapping: &HashMap<String,String>)
                        -> anyhow::Result<Mumble<'a, A>>
    where
        S: AsRef<str>
    {
        let password = password.map(|s| s.as_ref());
        let nickname = nickname.as_ref();
        let client_cert = client_cert.map(|s| s.as_ref());
        let server_cert = server_cert.map(|s| s.as_ref());
        let comment = comment.map(|s| s.as_ref().to_string());
        let channels = channel_mapping.iter()
            .chain(voice_channel_mapping.iter())
            .filter_map(|(channelname, busname)| {
                if let Some(sender) = bus_map.get(busname) {
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
