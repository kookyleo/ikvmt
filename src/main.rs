use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
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
#[command(version, about = "Native browser-free Supermicro iKVM console")]
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
        #[arg(long)]
        socket: PathBuf,
        #[arg(long, conflicts_with = "request_file")]
        params: Option<String>,
        #[arg(long)]
        request_file: Option<PathBuf>,
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
            #[cfg(unix)]
            {
                use std::os::unix::net::UnixStream;
                let mut stream = UnixStream::connect(socket)?;
                serde_json::to_writer(
                    &mut stream,
                    &json!({"id":now_ms(),"method":method,"params":p}),
                )?;
                writeln!(stream)?;
                stream.flush()?;
                let mut response = String::new();
                BufReader::new(stream).read_line(&mut response)?;
                if response.is_empty() {
                    bail!("service disconnected before a response; do not blindly retry input")
                }
                print!("{response}");
                let v: Value = serde_json::from_str(&response)?;
                if v.get("error").is_some() {
                    std::process::exit(1);
                }
            }
            #[cfg(not(unix))]
            bail!("Unix sockets unavailable");
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
