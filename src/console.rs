use crate::vendor::supermicro::x10_fw_4_00::{
    PROFILE,
    codec::Decoder,
    keyboard::{self, Action, KeyEvent},
    protocol::{self, Connection, Event},
    web::WebSession,
};
use anyhow::{Context, Result, bail, ensure};
use image::RgbaImage;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenOptions {
    pub target: String,
    #[serde(default = "default_user")]
    pub username: String,
    #[serde(default = "default_password_env")]
    pub password_env: String,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default = "default_profile")]
    pub profile: String,
}
fn default_user() -> String {
    "ADMIN".into()
}
fn default_password_env() -> String {
    "IKVM_PASS".into()
}
fn default_profile() -> String {
    PROFILE.into()
}

struct State {
    frame: Option<RgbaImage>,
    seq: u64,
    last_update: Option<u64>,
    error: Option<String>,
    connected: bool,
    keyboard: bool,
    no_signal: bool,
}
enum Control {
    Keys(Vec<KeyEvent>, Duration, mpsc::Sender<Value>),
    Close(mpsc::Sender<()>),
}
struct Worker {
    tx: mpsc::Sender<Control>,
    state: Arc<Mutex<State>>,
    join: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn connect(o: &OpenOptions) -> Result<Self> {
        ensure!(o.profile == PROFILE, "UNSUPPORTED_PROFILE: {}", o.profile);
        let password = std::env::var(&o.password_env)
            .context("CREDENTIAL_UNAVAILABLE: BMC password environment variable is not set")?;
        let web = WebSession::login(&o.target, &o.username, &password, o.insecure)?;
        let conn = Connection::connect(web)?;
        let keyboard = conn.keyboard;
        let state = Arc::new(Mutex::new(State {
            frame: None,
            seq: 0,
            last_update: None,
            error: None,
            connected: true,
            keyboard,
            no_signal: false,
        }));
        let (tx, rx) = mpsc::channel();
        let shared = state.clone();
        let join = thread::spawn(move || run(conn, rx, shared));
        Ok(Self {
            tx,
            state,
            join: Some(join),
        })
    }
    fn close(&mut self) {
        let (tx, rx) = mpsc::channel();
        let _ = self.tx.send(Control::Close(tx));
        let _ = rx.recv_timeout(Duration::from_secs(22));
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn run(mut c: Connection, rx: mpsc::Receiver<Control>, state: Arc<Mutex<State>>) {
    let mut decoder = Decoder::new();
    let mut pending: Option<(Vec<KeyEvent>, usize, Duration, mpsc::Sender<Value>)> = None;
    let mut pressed = HashSet::new();
    let mut next_key = Instant::now();
    let mut last_request = Instant::now();
    let result = (|| -> Result<()> {
        loop {
            match rx.try_recv() {
                Ok(Control::Close(reply)) => {
                    for &key in &pressed {
                        let _ = c.send(keyboard::packet(key, false).to_vec());
                    }
                    c.close();
                    let _ = reply.send(());
                    return Ok(());
                }
                Ok(Control::Keys(events, interval, reply)) => {
                    if pending.is_some() {
                        let _ = reply.send(json!({"status":"not_submitted","error":"INPUT_BUSY"}));
                    } else if !c.keyboard {
                        let _ = reply
                            .send(json!({"status":"not_submitted","error":"READ_ONLY_SESSION"}));
                    } else {
                        pending = Some((events, 0, interval, reply));
                        next_key = Instant::now();
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    c.close();
                    return Ok(());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if let Some((events, index, interval, reply)) = pending.as_mut() {
                if *index < events.len() && Instant::now() >= next_key {
                    let e = &events[*index];
                    if let Err(err) = c.send(keyboard::packet(e.code, e.down).to_vec()) {
                        let _=reply.send(json!({"status":"unknown","submitted_events":index,"in_flight_action_index":e.action,"error":format!("{err:#}")}));
                        pending = None;
                        return Err(err);
                    }
                    if e.down {
                        pressed.insert(e.code);
                    } else {
                        pressed.remove(&e.code);
                    }
                    *index += 1;
                    next_key = Instant::now() + *interval;
                }
                if *index == events.len() {
                    let _=reply.send(json!({"status":"submitted","submitted_events":index,"submitted_actions":events.last().map_or(0,|e|e.action+1)}));
                    pending = None;
                }
            }
            c.receive()?;
            while let Some(event) = protocol::parse(&mut c.buffer)? {
                match event {
                    Event::Frame {
                        width,
                        height,
                        encoding,
                        data,
                    } => {
                        let frame = match encoding {
                            87 => decoder.decode(width.into(), height.into(), &data)?.clone(),
                            88 => {
                                let img = image::load_from_memory_with_format(
                                    &data,
                                    image::ImageFormat::Jpeg,
                                )
                                .context("unsupported AST JPEG payload")?
                                .into_rgba8();
                                ensure!(
                                    img.dimensions() == (width.into(), height.into()),
                                    "JPEG dimensions mismatch"
                                );
                                img
                            }
                            _ => bail!("UNSUPPORTED_PROFILE: video encoding {encoding}"),
                        };
                        {
                            let mut s = state.lock().unwrap();
                            s.frame = Some(frame);
                            s.seq += 1;
                            s.last_update = Some(now_ms());
                            s.no_signal = false;
                        }
                        c.width = width;
                        c.height = height;
                        c.send(protocol::update_request(width, height, true))?;
                        last_request = Instant::now();
                    }
                    Event::NoSignal => {
                        let mut s = state.lock().unwrap();
                        s.no_signal = true;
                        s.frame = None;
                    }
                    Event::Notice { code } => {
                        eprintln!("BMC notice code={code}");
                    }
                    Event::Other => {}
                }
            }
            // Re-request pixels when the server has not answered; this never sends keys.
            if last_request.elapsed() > Duration::from_secs(2) {
                c.send(protocol::update_request(c.width, c.height, true))?;
                last_request = Instant::now();
            }
        }
    })();
    if let Some((events, index, _, reply)) = pending {
        let _=reply.send(json!({"status":if index==0{"not_submitted"}else{"partial"},"submitted_events":index,"in_flight_action_index":events.get(index).map(|e|e.action)}));
    }
    if result.is_err() {
        for &key in &pressed {
            let _ = c.send(keyboard::packet(key, false).to_vec());
        }
    }
    let mut s = state.lock().unwrap();
    s.connected = false;
    s.error = result.err().map(|e| format!("{e:#}"));
    drop(s);
    c.close();
}

struct Observation {
    epoch: u64,
    revision: u64,
    hash: String,
}
pub struct Session {
    pub id: String,
    options: OpenOptions,
    worker: Worker,
    epoch: u64,
    revision: u64,
    sequence: u64,
    observations: HashMap<String, Observation>,
    requests: HashMap<String, (Value, Value)>,
    out_dir: PathBuf,
    closed: bool,
}

impl Session {
    pub fn open(id: String, options: OpenOptions, out_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&out_dir)?;
        let worker = Worker::connect(&options)?;
        let s = Self {
            id,
            options,
            worker,
            epoch: 1,
            revision: 0,
            sequence: 0,
            observations: HashMap::new(),
            requests: HashMap::new(),
            out_dir,
            closed: false,
        };
        let start = Instant::now();
        loop {
            let st = s.worker.state.lock().unwrap();
            if st.frame.is_some() || !st.connected {
                break;
            }
            drop(st);
            if start.elapsed() > Duration::from_secs(15) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        Ok(s)
    }
    pub fn target(&self) -> &str {
        &self.options.target
    }
    pub fn is_closed(&self) -> bool {
        self.closed
    }
    pub fn status(&self) -> Value {
        let s = self.worker.state.lock().unwrap();
        json!({"session_id":self.id,"target":self.options.target,"profile":self.options.profile,"connection_epoch":self.epoch,"input_revision":self.revision,
            "connected":s.connected,"closed":self.closed,"video_update_seq":s.seq,"last_update_received_at_ms":s.last_update,"no_signal":s.no_signal,"error":s.error,
            "capabilities":{"keyboard":s.keyboard,"text":"printable-us-ascii","keyboard_layout":"US","mouse":false,"ocr":"optional-native-ocrs","ocr_engines":["ocrs","tesseract"],"video_encoding":"ast2100"}})
    }
    pub fn observe(&mut self, p: &Value) -> Result<Value> {
        ensure!(!self.closed, "SESSION_LOST: session is closed");
        let ocr_mode = crate::ocr::Mode::from_request(p)?;
        let since = p
            .pointer("/wait/since")
            .or_else(|| p.get("since"))
            .and_then(Value::as_str);
        let prior = if let Some(id) = since {
            Some(
                self.observations
                    .get(id)
                    .context("STALE_OBSERVATION: unknown observation")?,
            )
        } else {
            None
        };
        if let Some(o) = prior {
            ensure!(
                o.epoch == self.epoch,
                "STALE_OBSERVATION: observation belongs to earlier connection"
            );
        }
        let old_hash = prior.map(|o| o.hash.clone());
        let timeout = p
            .pointer("/wait/timeout_ms")
            .or_else(|| p.get("timeout_ms"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(10000);
        let start = Instant::now();
        let (frame, seq, last, connected, error, no_signal, changed) = loop {
            let st = self.worker.state.lock().unwrap();
            let hash = st
                .frame
                .as_ref()
                .map(|f| format!("{:x}", Sha256::digest(f.as_raw())));
            let changed = old_hash.as_ref().zip(hash.as_ref()).map(|(a, b)| a != b);
            if timeout == 0
                || changed == Some(true)
                || !st.connected
                || start.elapsed() >= Duration::from_millis(timeout)
            {
                break (
                    st.frame.clone(),
                    st.seq,
                    st.last_update,
                    st.connected,
                    st.error.clone(),
                    st.no_signal,
                    changed,
                );
            }
            drop(st);
            thread::sleep(Duration::from_millis(50));
        };
        self.sequence += 1;
        let oid = format!("{}-o{}", self.id, self.sequence);
        let mut image = Value::Null;
        let mut ocr = json!({"status":"disabled"});
        if let Some(ref frame) = frame {
            let path = self.out_dir.join(format!("{oid}.png"));
            let mut file = private_file(&path)?;
            frame.write_to(&mut file, image::ImageFormat::Png)?;
            file.flush()?;
            let hash = format!("{:x}", Sha256::digest(frame.as_raw()));
            self.observations.insert(
                oid.clone(),
                Observation {
                    epoch: self.epoch,
                    revision: self.revision,
                    hash: hash.clone(),
                },
            );
            image = json!({"path":path,"mime_type":"image/png","width":frame.width(),"height":frame.height(),"sha256_pixels":hash,"source":if connected&&!no_signal{"framebuffer"}else{"cached"}});
            ocr = crate::ocr::recognize(&path, ocr_mode);
        }
        Ok(
            json!({"observation_id":oid,"session_id":self.id,"connection_epoch":self.epoch,"input_revision":self.revision,"exported_at_ms":now_ms(),
            "image":image,"video":{"update_seq":seq,"last_update_received_at_ms":last,"changed_since":changed,"connected":connected,"no_signal":no_signal,"error":error},
            "wait_satisfied":if since.is_some(){changed.unwrap_or(false)}else{frame.is_some()},"ocr":ocr}),
        )
    }
    pub fn act(&mut self, p: &Value) -> Result<Value> {
        ensure!(!self.closed, "SESSION_LOST: session is closed");
        let rid = p
            .get("request_id")
            .and_then(Value::as_str)
            .context("INVALID_ARGUMENT: request_id is required")?;
        ensure!(
            !rid.is_empty() && rid.len() <= 128,
            "INVALID_ARGUMENT: request_id length"
        );
        if let Some((request, result)) = self.requests.get(rid) {
            ensure!(
                request == p,
                "REQUEST_ID_CONFLICT: same id with different payload"
            );
            return Ok(result.clone());
        }
        let based = p
            .get("based_on")
            .and_then(Value::as_str)
            .context("INVALID_ARGUMENT: based_on observation is required")?;
        let o = self
            .observations
            .get(based)
            .context("STALE_OBSERVATION: unknown observation")?;
        ensure!(
            o.epoch == self.epoch && o.revision == self.revision,
            "STALE_OBSERVATION: console input or connection changed"
        );
        let actions: Vec<Action> = serde_json::from_value(
            p.get("actions")
                .context("INVALID_ARGUMENT: actions required")?
                .clone(),
        )?;
        let events = keyboard::compile(&actions)?;
        let interval = key_event_interval(p)?;
        if let Some(after) = p.get("observe_after").filter(|v| v.is_object()) {
            crate::ocr::Mode::from_request(after)?;
        }
        ensure!(
            self.worker.state.lock().unwrap().connected,
            "DISCONNECTED: reconnect before input"
        );
        let wait = Duration::from_millis(
            (events.len() as u64) * (interval.as_millis() as u64 + 50) + 10000,
        );
        let (tx, rx) = mpsc::channel();
        self.worker
            .tx
            .send(Control::Keys(events, interval, tx))
            .context("DISCONNECTED: session worker stopped")?;
        self.revision += 1;
        let input = rx
            .recv_timeout(wait)
            .unwrap_or_else(|_| json!({"status":"unknown","error":"INPUT_UNCERTAIN"}));
        let mut result = json!({"session_id":self.id,"request_id":rid,"input":input});
        // Cache before observation, so screenshot/OCR failure never permits a replay.
        self.requests
            .insert(rid.into(), (p.clone(), result.clone()));
        if p.get("observe_after") != Some(&Value::Bool(false)) {
            let after = p
                .get("observe_after")
                .cloned()
                .filter(Value::is_object)
                .unwrap_or(json!({}));
            let delay = after
                .get("delay_ms")
                .and_then(Value::as_u64)
                .unwrap_or(500)
                .min(10000);
            thread::sleep(Duration::from_millis(delay));
            match self.observe(&after) {
                Ok(o) => result["observation"] = o,
                Err(e) => result["observation_error"] = json!(format!("{e:#}")),
            };
        }
        self.requests
            .insert(rid.into(), (p.clone(), result.clone()));
        Ok(result)
    }
    pub fn reconnect(&mut self) -> Result<Value> {
        ensure!(!self.closed, "SESSION_LOST: session is closed");
        self.worker.close();
        self.epoch += 1;
        self.worker = Worker::connect(&self.options)?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(15) {
            let s = self.worker.state.lock().unwrap();
            if s.frame.is_some() || !s.connected {
                break;
            }
            drop(s);
            thread::sleep(Duration::from_millis(50));
        }
        self.observe(&json!({}))
    }
    pub fn close(&mut self) -> Value {
        if !self.closed {
            self.worker.close();
            self.closed = true;
        }
        json!({"session_id":self.id,"closed":true})
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn key_event_interval(p: &Value) -> Result<Duration> {
    let ms = match p.get("key_event_interval_ms") {
        None => 30,
        Some(value) => value
            .as_u64()
            .context("INVALID_ARGUMENT: key_event_interval_ms must be an integer")?,
    };
    ensure!(
        (30..=1000).contains(&ms),
        "INVALID_ARGUMENT: key_event_interval_ms must be 30..1000"
    );
    Ok(Duration::from_millis(ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn mock_session() -> (Session, Arc<AtomicUsize>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let received = count.clone();
        let (tx, rx) = mpsc::channel();
        let join = thread::spawn(move || {
            while let Ok(control) = rx.recv() {
                match control {
                    Control::Keys(events, _, reply) => {
                        received.fetch_add(events.len(), Ordering::SeqCst);
                        reply.send(json!({"status":"submitted"})).unwrap();
                    }
                    Control::Close(reply) => {
                        let _ = reply.send(());
                        break;
                    }
                }
            }
        });
        let state = Arc::new(Mutex::new(State {
            frame: Some(RgbaImage::new(8, 8)),
            seq: 1,
            last_update: Some(now_ms()),
            error: None,
            connected: true,
            keyboard: true,
            no_signal: false,
        }));
        let session = Session {
            id: "test".into(),
            options: serde_json::from_value(json!({"target":"example.invalid"})).unwrap(),
            worker: Worker {
                tx,
                state,
                join: Some(join),
            },
            epoch: 1,
            revision: 0,
            sequence: 0,
            observations: HashMap::new(),
            requests: HashMap::new(),
            out_dir: dir.path().into(),
            closed: false,
        };
        (session, count, dir)
    }

    #[test]
    fn duplicate_input_is_cached_but_stale_and_conflicting_requests_are_rejected() {
        let (mut s, count, _dir) = mock_session();
        let o = s.observe(&json!({})).unwrap();
        let mut request = json!({"request_id":"r1","based_on":o["observation_id"],
            "actions":[{"type":"text","text":"a"}],"observe_after":false});
        let first = s.act(&request).unwrap();
        assert_eq!(s.act(&request).unwrap(), first);
        assert_eq!(count.load(Ordering::SeqCst), 2);
        request["actions"][0]["text"] = json!("b");
        assert!(
            s.act(&request)
                .unwrap_err()
                .to_string()
                .starts_with("REQUEST_ID_CONFLICT")
        );
        request["request_id"] = json!("r2");
        assert!(
            s.act(&request)
                .unwrap_err()
                .to_string()
                .starts_with("STALE_OBSERVATION")
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(s.revision, 1);
    }

    #[test]
    fn preflight_failure_and_old_connection_observation_send_nothing() {
        let (mut s, count, _dir) = mock_session();
        let o = s.observe(&json!({})).unwrap();
        let mut request = json!({"request_id":"r1","based_on":o["observation_id"],
            "actions":[{"type":"text","text":"valid"},{"type":"text","text":"\n"}],"observe_after":false});
        assert!(s.act(&request).is_err());
        assert_eq!(s.revision, 0);
        s.epoch += 1;
        request["actions"] = json!([{"type":"key","key":"Enter"}]);
        assert!(
            s.act(&request)
                .unwrap_err()
                .to_string()
                .starts_with("STALE_OBSERVATION")
        );
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn screenshot_failure_after_input_does_not_allow_replay() {
        let (mut s, count, dir) = mock_session();
        let o = s.observe(&json!({})).unwrap();
        s.out_dir = dir.path().join("missing-directory");
        let request = json!({"request_id":"r1","based_on":o["observation_id"],
            "actions":[{"type":"key","key":"Enter"}],"observe_after":{"delay_ms":0}});
        let first = s.act(&request).unwrap();
        assert!(first.get("observation_error").is_some());
        s.out_dir = dir.path().into();
        assert_eq!(s.act(&request).unwrap(), first);
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn key_timing_is_validated_before_any_input() {
        assert_eq!(
            key_event_interval(&json!({})).unwrap(),
            Duration::from_millis(30)
        );
        assert_eq!(
            key_event_interval(&json!({"key_event_interval_ms":150})).unwrap(),
            Duration::from_millis(150)
        );
        let (mut s, count, _dir) = mock_session();
        let o = s.observe(&json!({})).unwrap();
        for invalid in [json!(0), json!(1001), json!(-1), json!(30.5), json!("150")] {
            let request = json!({"request_id":"invalid-timing","based_on":o["observation_id"],
                "actions":[{"type":"key","key":"Enter"}],"key_event_interval_ms":invalid});
            assert!(
                s.act(&request)
                    .unwrap_err()
                    .to_string()
                    .starts_with("INVALID_ARGUMENT")
            );
        }
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert_eq!(s.revision, 0);
    }

    #[test]
    fn invalid_ocr_option_is_rejected_before_keyboard_input() {
        let (mut s, count, _dir) = mock_session();
        let o = s.observe(&json!({})).unwrap();
        let request = json!({"request_id":"bad-ocr","based_on":o["observation_id"],
            "actions":[{"type":"key","key":"Enter"}],"observe_after":{"ocr":"bad-engine"}});
        assert!(
            s.act(&request)
                .unwrap_err()
                .to_string()
                .starts_with("INVALID_ARGUMENT")
        );
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert_eq!(s.revision, 0);
    }
}
