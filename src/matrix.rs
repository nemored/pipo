use std::{
    collections::HashMap,
    fmt::Write,
    fs::File,
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::anyhow;
use deadpool_sqlite::Pool;
use rand::{rngs::ThreadRng, thread_rng, RngCore};
use ruma::api::appservice::{Namespace, Namespaces, Registration, RegistrationInit};
use tokio::sync::broadcast;

use crate::{ConfigTransport, Message};

pub(crate) struct Matrix {
    transport_id: usize,
    registration: Registration,
    channels: HashMap<String, broadcast::Sender<Message>>,
    pipo_id: Arc<Mutex<i64>>,
    pool: Pool,
}

impl Matrix {
    pub async fn new<P>(
        transport_id: usize,
        bus_map: &HashMap<String, broadcast::Sender<Message>>,
        pipo_id: Arc<Mutex<i64>>,
        pool: Pool,
        url: &str,
        registration_path: P,
        protocols: &Vec<ConfigTransport>,
        channel_mapping: &HashMap<String, String>,
    ) -> anyhow::Result<Self>
    where
        P: AsRef<Path>,
    {
        let channels: HashMap<_, _> = channel_mapping
            .iter()
            .filter_map(|(channel_name, bus_name)| {
                if let Some(sender) = bus_map.get(bus_name) {
                    Some((channel_name.to_owned(), sender.clone()))
                } else {
                    eprintln!("No bus named '{}' in configuration file.", bus_name);
                    None
                }
            })
            .collect();
        let protocols: Vec<_> = protocols.iter().map(|x| x.name().to_owned()).collect();
        let mut namespaces = Namespaces::new();
        namespaces.users = protocols
            .iter()
            .map(|x| Namespace::new(true, format!("@_{}_.*:tejat\\.net", x.to_lowercase())))
            .collect();
        namespaces.aliases = protocols
            .iter()
            .map(|x| Namespace::new(true, format!("#_{}_.#:tejat\\.net", x.to_lowercase())))
            .collect();
        namespaces.rooms = channels
            .keys()
            .map(|x| Namespace::new(true, x.to_owned()))
            .collect();
        let registration = match File::open(registration_path) {
            Ok(registration_file) => {
                let mut registration: Registration = serde_yaml::from_reader(registration_file)?;
                if &registration.id == "pipo" {
                    registration.url = Some(url.to_string());
                    registration.sender_localpart = "_pipo".to_string();
                    registration.namespaces = namespaces;
                    registration.rate_limited = Some(true);
                    registration.protocols = Some(protocols);
                    Ok(registration)
                } else {
                    Err(anyhow!(
                        "incorrect application service ID: \"{}\"",
                        registration.id
                    ))
                }
            }
            Err(e) => match e.kind() {
                std::io::ErrorKind::NotFound => {
                    let mut token_generator = TokenGenerator::new();
                    let as_token = token_generator.generate_token();
                    let hs_token = token_generator.generate_token();
                    Ok(RegistrationInit {
                        id: "pipo".to_string(),
                        url: Some(url.to_string()),
                        as_token,
                        hs_token,
                        sender_localpart: "_pipo".to_string(),
                        namespaces,
                        rate_limited: Some(true),
                        protocols: Some(protocols),
                    }
                    .into())
                }
                _ => Err(e.into()),
            },
        }?;

        Ok(Self {
            transport_id,
            registration,
            channels,
            pipo_id,
            pool,
        })
    }
    pub async fn connect(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

struct TokenGenerator {
    random_bytes: [u8; 32],
    thread_rng: ThreadRng,
}

impl TokenGenerator {
    pub fn new() -> Self {
        Self {
            random_bytes: [0; 32],
            thread_rng: thread_rng(),
        }
    }
    pub fn generate_token(&mut self) -> String {
        self.thread_rng.fill_bytes(&mut self.random_bytes);
        let mut token = String::new();
        for byte in self.random_bytes {
            write!(token, "{byte:02x}").expect("failed to write to String");
        }
        token
    }
}
