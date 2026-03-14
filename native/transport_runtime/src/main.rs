use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::process;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use transport_runtime::protocol::{
    encode_ndjson_frame, parse_ndjson_line, FrameMeta, InboundFrame, OutboundFrame,
    SUPPORTED_PROTOCOL_VERSION,
};
use transport_runtime::ExitCode;

#[derive(Debug)]
struct Cli {
    transport: String,
    config: String,
    instance_id: String,
    protocol_version: String,
}

#[derive(Debug)]
struct TransportEvent {
    bus: String,
    payload: serde_json::Value,
}

fn main() {
    let code = std::panic::catch_unwind(run)
        .unwrap_or(ExitCode::InternalPanic)
        .as_i32();
    process::exit(code);
}

fn run() -> ExitCode {
    let cli = match parse_cli(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::InvalidArgsOrConfig;
        }
    };

    if cli.protocol_version != SUPPORTED_PROTOCOL_VERSION {
        eprintln!(
            "Protocol version mismatch: requested={}, supported={}",
            cli.protocol_version, SUPPORTED_PROTOCOL_VERSION
        );
        return ExitCode::ProtocolVersionIncompatible;
    }

    let config_raw = match fs::read_to_string(&cli.config) {
        Ok(raw) => raw,
        Err(err) => {
            eprintln!("Failed to read config: {err}");
            return ExitCode::InvalidArgsOrConfig;
        }
    };

    if serde_json::from_str::<serde_json::Value>(&config_raw).is_err() {
        eprintln!("Config must be valid JSON");
        return ExitCode::InvalidArgsOrConfig;
    }

    if let Some(exit) = simulate_transport_startup_failures(&config_raw) {
        return exit;
    }

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    let ready = OutboundFrame::Ready { meta: meta(&cli) };
    if emit(&mut stdout, &ready).is_err() {
        return ExitCode::TransportFatal;
    }

    let (stdin_tx, stdin_rx) = mpsc::channel::<String>();
    let (transport_tx, transport_rx) = mpsc::channel::<TransportEvent>();

    thread::spawn(move || {
        let mut reader = BufReader::new(stdin.lock());
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let _ = stdin_tx.send(line.trim_end().to_owned());
                }
                Err(_) => break,
            }
        }
    });

    loop {
        while let Ok(event) = transport_rx.try_recv() {
            let frame = OutboundFrame::Message {
                meta: meta(&cli),
                bus: event.bus,
                pipo_id: None,
                payload: event.payload,
            };
            if emit(&mut stdout, &frame).is_err() {
                return ExitCode::TransportFatal;
            }
        }

        match stdin_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
                if line.is_empty() {
                    continue;
                }
                match parse_ndjson_line(&line) {
                    Ok(InboundFrame::Shutdown) => return ExitCode::OrderlyShutdown,
                    Ok(InboundFrame::HealthCheck) => {
                        let frame = OutboundFrame::Health {
                            meta: meta(&cli),
                            status: "ok",
                        };
                        if emit(&mut stdout, &frame).is_err() {
                            return ExitCode::TransportFatal;
                        }
                    }
                    Ok(InboundFrame::InjectMessage {
                        bus,
                        pipo_id,
                        origin_transport,
                        payload,
                    }) => {
                        let _ = (pipo_id, origin_transport);
                        if transport_tx.send(TransportEvent { bus, payload }).is_err() {
                            return ExitCode::TransportFatal;
                        }
                    }
                    Ok(InboundFrame::ReloadConfig) => {
                        let frame = OutboundFrame::Warn {
                            meta: meta(&cli),
                            warn_type: "unimplemented",
                            message: "reload_config is reserved and not implemented".to_owned(),
                        };
                        if emit(&mut stdout, &frame).is_err() {
                            return ExitCode::TransportFatal;
                        }
                    }
                    Err(err) => {
                        let frame = OutboundFrame::Warn {
                            meta: meta(&cli),
                            warn_type: "invalid_frame",
                            message: format!("Failed to parse inbound frame: {err}"),
                        };
                        if emit(&mut stdout, &frame).is_err() {
                            return ExitCode::TransportFatal;
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return ExitCode::OrderlyShutdown,
        }
    }
}

fn emit(stdout: &mut io::Stdout, frame: &OutboundFrame) -> io::Result<()> {
    let encoded = encode_ndjson_frame(frame)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    stdout.write_all(encoded.as_bytes())?;
    stdout.flush()
}

fn meta(cli: &Cli) -> FrameMeta {
    FrameMeta::new(&cli.protocol_version, &cli.transport, &cli.instance_id)
}

fn parse_cli(args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut transport = None;
    let mut config = None;
    let mut instance_id = None;
    let mut protocol_version = Some(SUPPORTED_PROTOCOL_VERSION.to_owned());

    let mut iter = args.peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--transport" => transport = iter.next(),
            "--config" => config = iter.next(),
            "--instance-id" => instance_id = iter.next(),
            "--protocol-version" => protocol_version = iter.next(),
            _ => return Err(format!("Unknown argument: {arg}")),
        }
    }

    let transport = transport.ok_or("Missing required --transport".to_owned())?;
    let config = config.ok_or("Missing required --config".to_owned())?;
    let instance_id = instance_id.unwrap_or_else(|| format!("{}_0", transport));
    let protocol_version = protocol_version.ok_or("Missing protocol version value".to_owned())?;

    Ok(Cli {
        transport,
        config,
        instance_id,
        protocol_version,
    })
}

fn simulate_transport_startup_failures(config_raw: &str) -> Option<ExitCode> {
    let value: serde_json::Value = serde_json::from_str(config_raw).ok()?;
    let startup = value.get("startup_failure")?.as_str()?;
    match startup {
        "auth" => Some(ExitCode::TransportAuthFailure),
        "network" => Some(ExitCode::TransportNetworkUnreachable),
        "fatal" => Some(ExitCode::TransportFatal),
        _ => Some(ExitCode::TransportFatal),
    }
}
