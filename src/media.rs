//! Persistent virtual-media sessions with explicit image ownership.
mod scsi;
use crate::vendor::supermicro::x10_fw_4_00::{PROFILE, media::Connection, web::WebSession};
use anyhow::{Context, Result, ensure};
pub use scsi::Kind;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountOptions {
    pub target: String,
    #[serde(default = "user")]
    pub username: String,
    #[serde(default = "password_env")]
    pub password_env: String,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default = "profile")]
    pub profile: String,
    pub image: PathBuf,
    pub kind: Kind,
    #[serde(default)]
    pub writable: bool,
    #[serde(default)]
    pub slot: u8,
}
fn user() -> String {
    "ADMIN".into()
}
fn password_env() -> String {
    "IKVM_PASS".into()
}
fn profile() -> String {
    PROFILE.into()
}
struct State {
    phase: &'static str,
    error: Option<String>,
    stats: scsi::Statistics,
}
pub struct Session {
    id: String,
    target: String,
    image: PathBuf,
    kind: Kind,
    writable: bool,
    size: u64,
    slot: u8,
    state: Arc<Mutex<State>>,
    stop: mpsc::Sender<()>,
    // The session retains the lock even if the worker unwinds unexpectedly.
    locked_image: Option<Arc<Mutex<scsi::Image>>>,
    join: Option<thread::JoinHandle<()>>,
}
impl Session {
    pub fn mount(id: String, options: MountOptions) -> Result<Self> {
        ensure!(
            options.profile == PROFILE,
            "UNSUPPORTED_PROFILE: {}",
            options.profile
        );
        ensure!(options.slot < 3, "INVALID_ARGUMENT: slot must be 0, 1 or 2");
        let image = scsi::Image::open(&options.image, options.kind, options.writable)?;
        let password = std::env::var(&options.password_env)
            .context("AUTH_FAILED: password environment variable is not set")?;
        let web = WebSession::login(
            &options.target,
            &options.username,
            &password,
            options.insecure,
        )?;
        let target = web.origin.as_str().trim_end_matches('/').to_string();
        let mut connection = Connection::mount(web, options.kind == Kind::Cdrom, options.slot)?;
        let state = Arc::new(Mutex::new(State {
            phase: "attached",
            error: None,
            stats: scsi::Statistics::default(),
        }));
        let (stop, rx) = mpsc::channel();
        let mut session = Self {
            id,
            target,
            image: image.path.clone(),
            kind: image.kind,
            writable: image.writable,
            size: image.size,
            slot: options.slot,
            state: state.clone(),
            stop,
            locked_image: None,
            join: None,
        };
        let image = Arc::new(Mutex::new(image));
        session.locked_image = Some(image.clone());
        session.join = Some(thread::spawn(move || {
            let mut image = image.lock().unwrap();
            let result = (|| -> Result<()> {
                loop {
                    if !matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                        image.sync()?;
                        connection.unmount()?;
                        return Ok(());
                    }
                    if let Some((command, out)) = connection.next_command()? {
                        let reply = image.execute(&command, &out)?;
                        // Publish durable counters even when the reply is lost after a write.
                        state.lock().unwrap().stats = image.stats.clone();
                        connection.respond(&command, &reply.data, reply.processed, reply.failed)?;
                    }
                }
            })();
            let sync_result = image.sync();
            let mut s = state.lock().unwrap();
            s.stats = image.stats.clone();
            match result.and(sync_result) {
                Ok(()) => s.phase = "detached",
                Err(e) => {
                    s.phase = "disconnected";
                    s.error = Some(format!("{e:#}"));
                }
            }
            drop(s);
        }));
        Ok(session)
    }
    pub fn status(&self) -> Value {
        let mut s = self.state.lock().unwrap();
        if s.phase == "attached" && self.join.as_ref().is_some_and(|j| j.is_finished()) {
            s.phase = "disconnected";
            s.error = Some(
                "MEDIA_WORKER_FAILED: worker terminated unexpectedly; detach is unconfirmed".into(),
            );
        }
        json!({"media_id":self.id,"target":self.target,"image":self.image,"kind":self.kind,
            "writable":self.writable,"size_bytes":self.size,"slot":self.slot,
            "state":s.phase,"error":s.error,"statistics":s.stats,
            "image_lock":if self.locked_image.is_some() {if self.writable {"exclusive"} else {"shared"}} else {"released"},
            "host_enumeration":"not_observed","reconnect":"manual"})
    }
    pub fn active(&self) -> bool {
        self.join.is_some()
    }
    pub fn target_matches(&self, target: &str, slot: u8) -> bool {
        let raw = if target.contains("://") {
            target.to_string()
        } else {
            format!("https://{target}")
        };
        self.slot == slot
            && reqwest::Url::parse(&raw)
                .is_ok_and(|url| self.target == url.as_str().trim_end_matches('/'))
    }
    pub fn unmount(&mut self) -> Value {
        let _ = self.stop.send(());
        if let Some(join) = self.join.take()
            && join.join().is_err()
        {
            let mut state = self.state.lock().unwrap();
            state.phase = "disconnected";
            state.error = Some(
                "MEDIA_WORKER_FAILED: worker terminated unexpectedly; detach is unconfirmed".into(),
            );
        }
        self.locked_image.take();
        self.status()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.unmount();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnected_worker_keeps_exclusive_lock_until_explicit_release() {
        check_lock_retention(false);
    }
    #[test]
    fn panicked_worker_keeps_exclusive_lock_until_explicit_release() {
        check_lock_retention(true);
    }
    fn check_lock_retention(panic: bool) {
        let f = tempfile::NamedTempFile::new().unwrap();
        f.as_file().set_len(4096).unwrap();
        let image = Arc::new(Mutex::new(
            scsi::Image::open(f.path(), Kind::Disk, true).unwrap(),
        ));
        let state = Arc::new(Mutex::new(State {
            phase: if panic { "attached" } else { "disconnected" },
            error: if panic {
                None
            } else {
                Some("test disconnect".into())
            },
            stats: scsi::Statistics::default(),
        }));
        let (stop, _rx) = mpsc::channel();
        let worker_image = image.clone();
        let join = thread::spawn(move || {
            let _image = worker_image.lock().unwrap();
            assert!(!panic, "simulated media worker panic");
        });
        while !join.is_finished() {
            thread::yield_now();
        }
        let mut session = Session {
            id: "m-test".into(),
            target: "https://example.invalid".into(),
            image: f.path().into(),
            kind: Kind::Disk,
            writable: true,
            size: 4096,
            slot: 0,
            state,
            stop,
            locked_image: Some(image),
            join: Some(join),
        };
        assert!(session.target_matches("https://EXAMPLE.invalid:443/", 0));
        assert!(!session.target_matches("example.invalid", 1));
        assert_eq!(session.status()["state"], "disconnected");
        assert_eq!(session.status()["image_lock"], "exclusive");
        assert!(scsi::Image::open(f.path(), Kind::Disk, true).is_err());
        assert!(scsi::Image::open(f.path(), Kind::Disk, false).is_err());
        assert_eq!(session.unmount()["image_lock"], "released");
        let reader = scsi::Image::open(f.path(), Kind::Disk, false).unwrap();
        let reader2 = scsi::Image::open(f.path(), Kind::Disk, false).unwrap();
        assert!(scsi::Image::open(f.path(), Kind::Disk, true).is_err());
        drop((reader, reader2));
        assert!(scsi::Image::open(f.path(), Kind::Disk, true).is_ok());
    }
}
