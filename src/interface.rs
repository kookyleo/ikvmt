use crate::console::{OpenOptions, Session, now_ms};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub struct Service {
    sessions: HashMap<String, Session>,
    media: HashMap<String, crate::media::Session>,
    out_dir: PathBuf,
    sequence: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MediaId {
    media_id: String,
}
impl Service {
    pub fn new(out_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&out_dir)?;
        Ok(Self {
            sessions: HashMap::new(),
            media: HashMap::new(),
            out_dir: std::fs::canonicalize(out_dir)?,
            sequence: 0,
        })
    }
    pub fn dispatch(&mut self, request: Value) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        match self.execute(&request) {
            Ok(result) => json!({"id":id,"result":result}),
            Err(e) => {
                let message = format!("{e:#}");
                let prefix = message.split(':').next().unwrap_or("ERROR");
                let code = if prefix.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                    prefix
                } else {
                    "ERROR"
                };
                json!({"id":id,"error":{"code":code,"message":message}})
            }
        }
    }
    fn execute(&mut self, r: &Value) -> Result<Value> {
        let method = r
            .get("method")
            .and_then(Value::as_str)
            .context("INVALID_ARGUMENT: method required")?;
        let p = r.get("params").cloned().unwrap_or(json!({}));
        ensure!(p.is_object(), "INVALID_ARGUMENT: params must be an object");
        match method {
            "media.mount" => {
                let options: crate::media::MountOptions = serde_json::from_value(p)
                    .context("INVALID_ARGUMENT: media.mount parameters")?;
                ensure!(
                    !self
                        .media
                        .values()
                        .any(|m| m.active() && m.target_matches(&options.target, options.slot)),
                    "MEDIA_BUSY: this process already serves this target slot; use media.list"
                );
                self.sequence += 1;
                let id = format!("m{}-{}", now_ms(), self.sequence);
                let media = crate::media::Session::mount(id.clone(), options)?;
                let result = media.status();
                self.media.insert(id, media);
                Ok(result)
            }
            "media.list" => {
                ensure!(
                    p.as_object().unwrap().is_empty(),
                    "INVALID_ARGUMENT: media.list takes no parameters"
                );
                Ok(json!(
                    self.media
                        .values()
                        .map(crate::media::Session::status)
                        .collect::<Vec<_>>()
                ))
            }
            "media.status" | "media.unmount" => {
                let params: MediaId = serde_json::from_value(p)
                    .context("INVALID_ARGUMENT: media_id required; no other parameters accepted")?;
                ensure!(
                    !params.media_id.is_empty(),
                    "INVALID_ARGUMENT: media_id must not be empty"
                );
                let media = self
                    .media
                    .get_mut(&params.media_id)
                    .context("SESSION_LOST: unknown media session; use media.list")?;
                Ok(if method == "media.unmount" {
                    media.unmount()
                } else {
                    media.status()
                })
            }
            "console.open" => {
                let o: OpenOptions = serde_json::from_value(p)
                    .context("INVALID_ARGUMENT: console.open parameters")?;
                ensure!(
                    !self
                        .sessions
                        .values()
                        .any(|s| !s.is_closed() && s.target() == o.target),
                    "BMC_BUSY: this process already has a session for this target"
                );
                self.sequence += 1;
                let id = format!("s{}-{}", now_ms(), self.sequence);
                let mut s = Session::open(id.clone(), o, self.out_dir.clone())?;
                let result = json!({"session":s.status(),"observation":s.observe(&json!({}))?});
                self.sessions.insert(id, s);
                Ok(result)
            }
            "console.list" => Ok(json!(
                self.sessions
                    .values()
                    .map(Session::status)
                    .collect::<Vec<_>>()
            )),
            "console.observe" | "console.act" | "console.reconnect" | "console.close"
            | "console.status" => {
                let sid = p
                    .get("session_id")
                    .and_then(Value::as_str)
                    .context("INVALID_ARGUMENT: session_id required")?;
                let s = self
                    .sessions
                    .get_mut(sid)
                    .context("SESSION_LOST: unknown session (service may have restarted)")?;
                match method {
                    "console.observe" => s.observe(&p),
                    "console.act" => s.act(&p),
                    "console.reconnect" => s.reconnect(),
                    "console.close" => Ok(s.close()),
                    "console.status" => Ok(s.status()),
                    _ => unreachable!(),
                }
            }
            _ => bail!("UNKNOWN_METHOD: {method}"),
        }
    }
}

pub fn lines(
    reader: impl BufRead,
    mut writer: impl Write,
    service: Arc<Mutex<Service>>,
) -> Result<()> {
    // Bounded line parsing avoids unbounded allocation from an accidental file input.
    let mut reader = reader;
    loop {
        let mut bytes = Vec::new();
        let n = std::io::Read::take(&mut reader, 1024 * 1024 + 1).read_until(b'\n', &mut bytes)?;
        if n == 0 {
            break;
        }
        ensure!(n <= 1024 * 1024, "request exceeds 1 MiB");
        let value = match serde_json::from_slice(&bytes) {
            Ok(r) => service.lock().unwrap().dispatch(r),
            Err(_) => {
                json!({"id":null,"error":{"code":"INVALID_JSON","message":"expected one JSON request per line"}})
            }
        };
        serde_json::to_writer(&mut writer, &value)?;
        writeln!(writer)?;
        writer.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn media_rejects_ignored_parameters_and_preserves_the_service() {
        let dir = tempfile::tempdir().unwrap();
        let mut service = Service::new(dir.path().into()).unwrap();
        for request in [
            json!({"method":"media.list","params":{"target":"example.invalid"}}),
            json!({"method":"media.status","params":{"session_id":"wrong-id-type"}}),
            json!({"method":"media.unmount","params":{"media_id":"x","force":true}}),
            json!({"method":"media.unmount","params":{"media_id":""}}),
            json!({"method":"media.mount","params":{"target":"example.invalid","image":"x","kind":"disk","writeable":true}}),
        ] {
            assert_eq!(
                service.dispatch(request)["error"]["code"],
                "INVALID_ARGUMENT"
            );
        }
        assert_eq!(
            service.dispatch(json!({"method":"media.status","params":{"media_id":"missing"}}))["error"]
                ["code"],
            "SESSION_LOST"
        );
        assert_eq!(
            service.dispatch(json!({"method":"media.list"}))["result"],
            json!([])
        );
    }
    #[test]
    fn jsonl_reports_bad_input_and_continues_without_exposing_raw_requests() {
        let dir = tempfile::tempdir().unwrap();
        let service = Arc::new(Mutex::new(Service::new(dir.path().into()).unwrap()));
        let input = b"{bad-input}\n{\"id\":2,\"method\":\"unknown\"}\n{\"id\":3,\"method\":\"console.list\"}\n";
        let mut out = Vec::new();
        lines(std::io::Cursor::new(input), &mut out, service).unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(!output.contains("bad-input"));
        let responses: Vec<Value> = output
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(responses[0]["error"]["code"], "INVALID_JSON");
        assert_eq!(responses[1]["error"]["code"], "UNKNOWN_METHOD");
        assert_eq!(responses[2], json!({"id":3,"result":[]}));
    }
}
