use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use ikvmt::{
    console::{OpenOptions, Session, now_ms},
    interface::{Service, lines},
};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, BufReader, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Parser)]
#[command(
    version,
    about = "Native browser-free Supermicro iKVM console and virtual media"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}
#[derive(Subcommand)]
enum Cmd {
    /// Persistent JSON Lines service. One request per line; diagnostics on stderr.
    Serve {
        #[arg(long)]
        socket: Option<PathBuf>,
        #[arg(long, default_value = "artifacts")]
        output_dir: PathBuf,
    },
    /// Send a JSON request to an already running local service.
    Call {
        method: String,
        #[arg(long, env = "IKVM_SOCKET")]
        socket: PathBuf,
        #[arg(long, conflicts_with = "request_file")]
        params: Option<String>,
        #[arg(long)]
        request_file: Option<PathBuf>,
    },
    /// Attach local images, inspect sessions, or detach. Requires a running service.
    Media {
        #[command(subcommand)]
        command: MediaCmd,
    },
    /// Connect read-only, export one image, then release the session.
    Shot {
        target: String,
        #[arg(long, env = "IKVM_USER", default_value = "ADMIN")]
        username: String,
        #[arg(long)]
        insecure: bool,
        #[arg(long)]
        ocr: bool,
        #[arg(long, value_enum, default_value = "ocrs")]
        ocr_engine: ikvmt::ocr::Mode,
        #[arg(long, default_value = "artifacts")]
        output_dir: PathBuf,
    },
    /// Decode a captured AST2100 payload offline.
    Decode {
        input: PathBuf,
        #[arg(long)]
        width: u32,
        #[arg(long)]
        height: u32,
        #[arg(long)]
        output: PathBuf,
    },
    /// Recognize a saved image offline. Native OCR and models are bundled.
    Ocr {
        input: PathBuf,
        #[arg(long, value_enum, default_value = "ocrs")]
        engine: ikvmt::ocr::Mode,
    },
}

#[derive(Args)]
struct MediaSocket {
    /// Socket of an already running ikvmt service.
    #[arg(long, env = "IKVM_SOCKET")]
    socket: PathBuf,
}

#[derive(Subcommand)]
enum MediaCmd {
    /// Attach an existing image. Read-only by default; locks are automatic.
    Mount {
        target: String,
        image: PathBuf,
        #[arg(long, value_enum)]
        kind: ikvmt::media::Kind,
        /// Allow host writes; requires exclusive access. Only valid for disk images.
        #[arg(long)]
        writable: bool,
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=2))]
        slot: u8,
        #[arg(long, env = "IKVM_USER", default_value = "ADMIN")]
        username: String,
        /// Name of the password variable in the SERVICE environment.
        #[arg(long, default_value = "IKVM_PASS")]
        password_env: String,
        #[arg(long, default_value = "supermicro-x10-fw4.00")]
        profile: String,
        #[arg(long)]
        insecure: bool,
        #[command(flatten)]
        connection: MediaSocket,
    },
    /// Show connection, image lock and I/O counters as JSON.
    Status {
        media_id: String,
        #[command(flatten)]
        connection: MediaSocket,
    },
    /// List this service's media sessions as JSON, including released sessions.
    List {
        #[command(flatten)]
        connection: MediaSocket,
    },
    /// Detach and release the lock. Sync/unmount the HOST filesystem first.
    Unmount {
        media_id: String,
        #[command(flatten)]
        connection: MediaSocket,
    },
}

impl MediaCmd {
    fn request(self) -> Result<(PathBuf, &'static str, Value)> {
        let (connection, method, params) = match self {
            Self::Mount {
                target,
                image,
                kind,
                writable,
                slot,
                username,
                password_env,
                profile,
                insecure,
                connection,
            } => {
                if kind == ikvmt::media::Kind::Cdrom && writable {
                    bail!(
                        "INVALID_ARGUMENT: CD-ROM is read-only; use --kind disk for writable images"
                    );
                }
                // CLI paths belong to the caller; send an absolute path to the service.
                let image = std::fs::canonicalize(image).context("cannot resolve image path")?;
                (
                    connection,
                    "media.mount",
                    json!({"target":target,"image":image,"kind":kind,"writable":writable,"slot":slot,"username":username,"password_env":password_env,"profile":profile,"insecure":insecure}),
                )
            }
            Self::Status {
                media_id,
                connection,
            } => (connection, "media.status", json!({"media_id":media_id})),
            Self::List { connection } => (connection, "media.list", json!({})),
            Self::Unmount {
                media_id,
                connection,
            } => (connection, "media.unmount", json!({"media_id":media_id})),
        };
        Ok((connection.socket, method, params))
    }
}

fn call_service(socket: PathBuf, method: &str, params: Value) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let mut stream = UnixStream::connect(socket)?;
        serde_json::to_writer(
            &mut stream,
            &json!({"id":now_ms(),"method":method,"params":params}),
        )?;
        writeln!(stream)?;
        stream.flush()?;
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response)?;
        if response.is_empty() {
            bail!("service disconnected before a response; inspect state before retrying");
        }
        print!("{response}");
        let v: Value = serde_json::from_str(&response)?;
        if v.get("error").is_some()
            || (method == "media.unmount" && v["result"]["state"] != "detached")
        {
            std::process::exit(1);
        }
        Ok(())
    }
    #[cfg(not(unix))]
    bail!("Unix sockets unavailable")
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{:#}", e);
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    match Cli::parse().command {
        Cmd::Serve { socket, output_dir } => {
            let service = Arc::new(Mutex::new(Service::new(output_dir)?));
            if let Some(path) = socket {
                #[cfg(unix)]
                {
                    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
                    let listener=UnixListener::bind(&path).context("cannot bind socket; choose a private directory and remove only known stale sockets")?;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
                    eprintln!("ikvmt service listening at {}", path.display());
                    for stream in listener.incoming() {
                        let stream = stream?;
                        let service = service.clone();
                        std::thread::spawn(move || {
                            if let Ok(read) = stream.try_clone()
                                && let Err(e) = lines(BufReader::new(read), stream, service)
                            {
                                eprintln!("client disconnected: {e}");
                            }
                        });
                    }
                }
                #[cfg(not(unix))]
                bail!("Unix sockets unavailable; use stdin service");
            } else {
                lines(io::stdin().lock(), io::stdout().lock(), service)?;
            }
        }
        Cmd::Call {
            method,
            socket,
            params,
            request_file,
        } => {
            let p: Value = if let Some(path) = request_file {
                serde_json::from_slice(&std::fs::read(path)?)?
            } else {
                serde_json::from_str(params.as_deref().unwrap_or("{}"))?
            };
            call_service(socket, &method, p)?;
        }
        Cmd::Media { command } => {
            let (socket, method, params) = command.request()?;
            call_service(socket, method, params)?;
        }
        Cmd::Shot {
            target,
            username,
            insecure,
            ocr,
            ocr_engine,
            output_dir,
        } => {
            let ocr_options = json!({"ocr":if ocr{ocr_engine.name()}else{"off"}});
            std::fs::create_dir_all(&output_dir)?;
            let o = OpenOptions {
                target,
                username,
                password_env: "IKVM_PASS".into(),
                insecure,
                profile: "supermicro-x10-fw4.00".into(),
            };
            let mut s = Session::open(
                format!("shot{}", now_ms()),
                o,
                std::fs::canonicalize(output_dir)?,
            )?;
            let result = s.observe(&ocr_options)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            s.close();
            if result["image"].is_null() {
                bail!("CAPTURE_FAILED: no decoded image; see video.error")
            }
        }
        Cmd::Decode {
            input,
            width,
            height,
            output,
        } => {
            let data = std::fs::read(input)?;
            let mut d = ikvmt::vendor::supermicro::x10_fw_4_00::codec::Decoder::new();
            d.decode(width, height, &data)?.save(&output)?;
            println!("{}", json!({"image":output}));
        }
        Cmd::Ocr { input, engine } => {
            let result = ikvmt::ocr::recognize(&input, engine);
            println!("{}", serde_json::to_string_pretty(&result)?);
            if result["status"] != "ok" && result["status"] != "disabled" {
                bail!("OCR_FAILED: see JSON result");
            }
        }
    }
    Ok(())
}
