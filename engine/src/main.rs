use anyhow::{ensure, Context, Result};
use clap::{Parser, Subcommand};
use extend_computer_agent::{
    discovery,
    identity::Identity,
    pairing::PairingWindow,
    session::{self, Client, Decision},
    trust::TrustStore,
};
use std::{
    io::{self, Write},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    path::PathBuf,
    time::Duration,
};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(about = "extend.computer device pairing and input control.")]
struct Args {
    #[arg(long, default_value = ".extend-computer-state", global = true)]
    state: PathBuf,
    #[arg(long, default_value = "default", global = true)]
    identity: String,
    /// In-memory identity for tests. Cannot reconnect after process exit.
    #[arg(long, global = true)]
    ephemeral: bool,
    /// Leave AWDL unchanged (true), or allow session-scoped Wi-Fi pauses (false, default).
    #[arg(long, global = true, action = clap::ArgAction::Set, num_args = 0..=1,
          default_missing_value = "true", require_equals = true, conflicts_with = "no_awdl")]
    awdl: Option<bool>,
    /// Allow temporary AWDL pauses during eligible cursor sessions (the default).
    #[arg(long, global = true, conflicts_with = "awdl")]
    no_awdl: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long, default_value = "127.0.0.1:48177")]
        bind: String,
        #[arg(long)]
        pair: bool,
        #[arg(long)]
        advertise: bool,
        /// Native helper executable; enables input for paired devices.
        #[arg(long)]
        cursor_helper: Option<PathBuf>,
    },
    Pair {
        address: String,
        #[arg(long, default_value_t = 5)]
        probes: u32,
    },
    Connect {
        address: String,
        #[arg(long)]
        peer: String,
        #[arg(long, default_value_t = 5)]
        probes: u32,
    },
    Cursor {
        address: String,
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        helper: PathBuf,
        /// Position of the peer's main display relative to this Mac.
        #[arg(long, default_value = "left", value_parser = ["left", "right"])]
        edge: String,
        /// Remote top relative to local top, in logical display points; positive is down.
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        offset_y: f64,
        /// Request full mouse and keyboard control for a paired device.
        #[arg(long)]
        input: bool,
    },
    /// Full input until stopped. Uses the same engine and trust store as the GUI will.
    Control {
        address: String,
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        helper: PathBuf,
        #[arg(long, default_value = "left", value_parser = ["left", "right"])]
        edge: String,
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        offset_y: f64,
        /// Retry network failures; requires an already-pinned peer and enabled receiving.
        #[arg(long, requires = "peer")]
        reconnect: bool,
        /// Optional per-connection run limit; omitted means until stopped/disconnected.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=86400))]
        seconds: Option<u64>,
    },
    Discover {
        #[arg(long, default_value_t = 8)]
        seconds: u64,
    },
    Peers,
    Revoke {
        fingerprint: String,
    },
    Identity,
}

fn confirm_pairing(peer: &str, known: bool) -> Decision {
    if known {
        return Decision::Once;
    }
    eprintln!("Pair with device {peer}?");
    eprint!("Type pair to confirm (default deny): ");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer).is_ok() && answer.trim() == "pair" {
        Decision::Remember
    } else {
        Decision::Deny
    }
}

fn connect(address: &str) -> Result<TcpStream> {
    let addresses = address.to_socket_addrs().context("resolve endpoint")?;
    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, Duration::from_secs(5)) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error
        .unwrap_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "no endpoint addresses",
            )
        })
        .into())
}

fn probes(mut client: Client, count: u32) -> Result<()> {
    ensure!((1..=1000).contains(&count), "probes must be 1..=1000");
    println!("authenticated peer={}", client.peer());
    for _ in 0..count {
        println!("probe rtt_ms={:.3}", client.probe()?.as_secs_f64() * 1000.0);
        std::thread::sleep(Duration::from_millis(100));
    }
    client.close()
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Discover { seconds } => {
            ensure!(
                (1..=60).contains(&seconds),
                "discovery duration must be 1..=60 seconds"
            );
            return discovery::discover(Duration::from_secs(seconds));
        }
        Command::Peers => {
            let store = TrustStore::open(&args.state)?;
            for id in store.load()?.peers.keys() {
                println!("{id}");
            }
            return Ok(());
        }
        Command::Revoke { ref fingerprint } => {
            return TrustStore::open(&args.state)?.revoke(fingerprint)
        }
        _ => {}
    }
    let identity = if args.ephemeral {
        eprintln!("Test mode: ephemeral identity; lost on exit.");
        Identity::generate()
    } else {
        Identity::load_persistent(&args.identity)
            .context("Keychain identity unavailable; no insecure fallback")?
    };
    let store = TrustStore::open(&args.state)?;
    match args.command {
        Command::Serve {
            bind,
            pair,
            advertise,
            cursor_helper,
        } => {
            let listener = TcpListener::bind(&bind)?;
            ensure!(
                listener.local_addr()?.is_ipv4(),
                "prototype listener requires IPv4"
            );
            ensure!(
                !advertise || listener.local_addr()?.ip().is_unspecified(),
                "discovery advertisement requires --bind 0.0.0.0:PORT in this prototype"
            );
            let _advertisement = if advertise {
                Some(discovery::Advertisement::start(
                    listener.local_addr()?.port(),
                )?)
            } else {
                None
            };
            let mut window = pair.then(|| PairingWindow::new(Duration::from_secs(120)));
            println!(
                "listening={} identity={}",
                listener.local_addr()?,
                identity.fingerprint()
            );
            if let Some(window) = &window {
                eprintln!(
                    "Pair code: {} (120 seconds; one attempt; keep private)",
                    window.code().unwrap()
                );
            }
            for stream in listener.incoming() {
                let stream = stream?;
                let mut cursor = extend_computer_agent::low_jitter::ManagedCursor::new(
                    extend_computer_agent::cursor::NativeSink::new(cursor_helper.clone()),
                    stream.peer_addr()?.ip(),
                    args.no_awdl || !args.awdl.unwrap_or(false),
                );
                match session::serve_connection_with_cursor(
                    stream,
                    &identity,
                    &store,
                    &mut window,
                    confirm_pairing,
                    &mut cursor,
                ) {
                    Ok(()) => eprintln!("session closed"),
                    Err(error) => eprintln!("session rejected/closed: {error}"),
                }
            }
            Ok(())
        }
        Command::Pair {
            address,
            probes: count,
        } => {
            let code = Zeroizing::new(rpassword::prompt_password("Pair code: ")?);
            let client = Client::connect(
                connect(&address)?,
                &identity,
                &store,
                Some(code.trim()),
                None,
                confirm_pairing,
            )?;
            probes(client, count)
        }
        Command::Connect {
            address,
            peer,
            probes: count,
        } => {
            let client = Client::connect(
                connect(&address)?,
                &identity,
                &store,
                None,
                Some(&peer),
                confirm_pairing,
            )?;
            probes(client, count)
        }
        Command::Cursor {
            address,
            peer,
            helper,
            edge,
            offset_y,
            input,
        } => {
            let code = if peer.is_none() {
                Some(Zeroizing::new(rpassword::prompt_password("Pair code: ")?))
            } else {
                None
            };
            let client = Client::connect(
                connect(&address)?,
                &identity,
                &store,
                code.as_ref().map(|s| s.trim()),
                peer.as_deref(),
                confirm_pairing,
            )?;
            let send = if input {
                extend_computer_agent::control::send
            } else {
                extend_computer_agent::cursor::send_with_policy
            };
            send(
                client,
                &helper,
                args.no_awdl || !args.awdl.unwrap_or(false),
                &edge,
                offset_y,
            )
        }
        Command::Control {
            address,
            peer,
            helper,
            edge,
            offset_y,
            reconnect,
            seconds,
        } => {
            ensure!(
                !reconnect || !args.ephemeral,
                "automatic reconnect requires a persistent identity"
            );
            let code = if peer.is_none() {
                Some(Zeroizing::new(rpassword::prompt_password("Pair code: ")?))
            } else {
                None
            };
            extend_computer_agent::reconnect::run(reconnect, || {
                let client = Client::connect(
                    connect(&address)?,
                    &identity,
                    &store,
                    code.as_ref().map(|s| s.trim()),
                    peer.as_deref(),
                    |id, known| {
                        if peer.is_some() && known {
                            Decision::Once
                        } else {
                            confirm_pairing(id, known)
                        }
                    },
                )?;
                extend_computer_agent::control::send_session(
                    client,
                    &helper,
                    args.no_awdl || !args.awdl.unwrap_or(false),
                    &edge,
                    offset_y,
                    seconds.map(Duration::from_secs),
                )
            })
        }
        Command::Identity => {
            println!("{}", identity.fingerprint());
            Ok(())
        }
        _ => unreachable!(),
    }
}
