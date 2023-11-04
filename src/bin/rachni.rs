use std::{collections::HashMap, sync::Arc};

use bytes::BytesMut;
use pipo::{Bus, Rachni, Message, Transport};
use serde::Deserialize;
use tokio::{
    io::AsyncReadExt,
    sync::mpsc::{self},
};

#[derive(Debug, Deserialize, serde::Serialize)]
struct Hello {
    server: String,
    api_key: String,
    interval: u64,
    buses: Vec<Arc<Bus>>,
}

#[derive(Debug)]
enum Error {
    Anyhow,
    SerdeJson(serde_json::Error),
    Io,
    TokioSend,
    TokioJoin,
    Whatever,
}

impl From<anyhow::Error> for Error {
    fn from(value: anyhow::Error) -> Self {
        match value {
            _ => Self::Anyhow,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::SerdeJson(value)
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        match value {
            _ => Self::Io,
        }
    }
}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for Error {
    fn from(value: tokio::sync::mpsc::error::SendError<T>) -> Self {
        match value {
            _ => Self::TokioSend,
        }
    }
}

impl From<tokio::task::JoinError> for Error {
    fn from(value: tokio::task::JoinError) -> Self {
        match value {
            _ => Self::TokioJoin,
        }
    }
}

type Result<T> = std::result::Result<T, Error>;

#[tokio::main]
async fn main() -> Result<()> {
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut read_buf = BytesMut::with_capacity(1024);
    let (router_tx, mut router_rx) = mpsc::channel(100);
    let hello: Hello = {
        let _ = stdin.read_buf(&mut read_buf).await?;
        serde_json::from_slice(&read_buf.split())?
    };
    let _ = Rachni::new(
        0,
        router_tx,
        &hello.server,
        &hello.api_key,
        hello.interval,
        &hello.buses,
        None,
    )
    .await?
    .start();
    loop {
        tokio::select! {
            message = router_rx.recv() => {
                let (message, _) = message.ok_or(Error::Whatever)?;
                // eprintln!("Got message:\n{message:?}");
                // let buf = serde_json::to_vec(&message)?;
                // stdout.write_all(&buf).await?;
                let json = serde_json::to_string(&message)?;
                println!("{json}");
            },
            else => {
                break;
            }
        }
    }

    Ok(())
}
