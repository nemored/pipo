const TRANSPORT_NAME: &'static str = "Mumble";

pub(crate) struct Mumble {
    transport_id: usize,
    channels: HashMap<String,broadcast::Sender<Message>>,
}

impl Mumble {
    pub async fn new(transport_id: usize,
                     bus_map: &HashMap<String,broadcast::Sender<Message>>,
                     channel_mapping: &HashMap<String,String>)
                     -> anyhow::Result<Self> {
        let channels = channel_mapping.iter()
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
            channels,
        })
    }
}
