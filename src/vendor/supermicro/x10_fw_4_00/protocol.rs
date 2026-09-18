use super::web::WebSession;
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::VecDeque,
    io,
    net::TcpStream,
    time::{Duration, Instant},
};
use tungstenite::{
    Connector, Message, WebSocket, client::IntoClientRequest, stream::MaybeTlsStream,
};

pub type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

pub struct Connection {
    pub ws: Socket,
    pub buffer: VecDeque<u8>,
    pub width: u16,
    pub height: u16,
    pub keyboard: bool,
    pub web: WebSession,
}

pub fn update_request(width: u16, height: u16, incremental: bool) -> Vec<u8> {
    let mut b = vec![3, u8::from(incremental), 0, 0, 0, 0];
    b.extend(width.to_be_bytes());
    b.extend(height.to_be_bytes());
    b
}

pub fn auth_response(ticket: &str) -> Result<Vec<u8>> {
    ensure!(
        ticket.is_ascii() && ticket.len() >= 24,
        "invalid KVM ticket"
    );
    let mut response = vec![0; 48];
    response[..15].copy_from_slice(&ticket.as_bytes()[..15]);
    response[24..33].copy_from_slice(&ticket.as_bytes()[15..24]);
    Ok(response)
}

fn timeout_socket(ws: &Socket, d: Duration) -> Result<()> {
    let tcp = match ws.get_ref() {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::NativeTls(s) => s.get_ref(),
        _ => bail!("unsupported TLS backend"),
    };
    tcp.set_read_timeout(Some(d))?;
    tcp.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(())
}

impl Connection {
    pub fn connect(web: WebSession) -> Result<Self> {
        let mut req = web.ws_url.clone().into_client_request()?;
        req.headers_mut()
            .insert("Origin", web.origin.as_str().trim_end_matches('/').parse()?);
        req.headers_mut().insert("Cookie", web.cookie.parse()?);
        // Firmware websock.js deliberately omits subprotocols for this endpoint.
        let host = web.origin.host_str().context("missing host")?;
        use std::net::ToSocketAddrs;
        let addresses = (host, web.origin.port_or_known_default().unwrap())
            .to_socket_addrs()?
            .collect::<Vec<_>>();
        let mut tcp = None;
        for addr in addresses {
            if let Ok(s) = TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
                tcp = Some(s);
                break;
            }
        }
        let tcp = tcp.context("DISCONNECTED: cannot connect to BMC WebSocket port")?;
        tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
        tcp.set_write_timeout(Some(Duration::from_secs(10)))?;
        tcp.set_nodelay(true)?;
        let tls = native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(web.insecure)
            .danger_accept_invalid_hostnames(web.insecure)
            .build()?;
        let (ws, _reply) =
            tungstenite::client_tls_with_config(req, tcp, None, Some(Connector::NativeTls(tls)))
                .map_err(|e| anyhow::anyhow!("WebSocket upgrade failed: {e}"))?;
        let mut c = Self {
            ws,
            buffer: VecDeque::new(),
            width: 0,
            height: 0,
            keyboard: false,
            web,
        };
        let banner = c.exact(12)?;
        ensure!(
            banner == b"RFB 003.008\n" || banner == b"RFB 055.008\n",
            "UNSUPPORTED_PROFILE: unexpected RFB version"
        );
        c.send(banner)?;
        let n = c.exact(1)?[0] as usize;
        ensure!(n > 0, "AUTH_FAILED: no RFB security types");
        let types = c.exact(n)?;
        let scheme = if types.contains(&16) {
            16
        } else if types.contains(&15) {
            15
        } else {
            bail!("UNSUPPORTED_PROFILE: expected Insyde authentication")
        };
        c.send(vec![scheme])?;
        c.exact(24)?;
        c.send(auth_response(&c.web.ticket)?)?;
        let result = c.u32()?;
        ensure!(
            result == 0,
            "AUTH_FAILED: Insyde KVM authentication rejected (status {result})"
        );
        c.send(vec![1])?;
        let init = c.exact(24)?;
        c.width = u16::from_be_bytes([init[0], init[1]]);
        c.height = u16::from_be_bytes([init[2], init[3]]);
        let n = u32::from_be_bytes(init[20..24].try_into().unwrap()) as usize;
        ensure!(n < 65536, "invalid desktop name length");
        c.exact(n)?;
        let extension = c.exact(12)?;
        ensure!(
            extension[8] != 0,
            "BMC_BUSY: KVM video permission unavailable"
        );
        c.keyboard = extension[9] != 0;
        ensure!(
            c.width > 0 && c.height > 0 && c.width <= 1920 && c.height <= 1280,
            "unsupported initial dimensions"
        );
        c.send(update_request(c.width, c.height, false))?;
        timeout_socket(&c.ws, Duration::from_millis(25))?;
        Ok(c)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.exact(4)?.try_into().unwrap()))
    }
    fn exact(&mut self, n: usize) -> Result<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_secs(15);
        while self.buffer.len() < n {
            ensure!(Instant::now() < deadline, "RFB handshake timeout");
            self.receive()?;
        }
        Ok(self.buffer.drain(..n).collect())
    }
    pub fn send(&mut self, bytes: Vec<u8>) -> Result<()> {
        self.ws
            .send(Message::Binary(bytes.into()))
            .context("DISCONNECTED: WebSocket send failed")
    }
    pub fn receive(&mut self) -> Result<bool> {
        match self.ws.read() {
            Ok(Message::Binary(b)) => {
                ensure!(
                    self.buffer.len() + b.len() <= 32 * 1024 * 1024,
                    "receive buffer limit exceeded"
                );
                self.buffer.extend(b);
                Ok(true)
            }
            Ok(Message::Ping(_)) => {
                self.ws.flush()?;
                Ok(false)
            }
            Ok(Message::Pong(_)) => Ok(false),
            Ok(Message::Close(_)) => bail!("DISCONNECTED: BMC closed WebSocket"),
            Ok(_) => bail!("UNSUPPORTED_PROFILE: unexpected non-binary message"),
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(false)
            }
            Err(e) => Err(e).context("DISCONNECTED: WebSocket receive failed"),
        }
    }
    pub fn close(&mut self) {
        let _ = self.ws.close(None);
        self.web.logout();
    }
}

#[derive(Debug)]
pub enum Event {
    Frame {
        width: u16,
        height: u16,
        encoding: i32,
        data: Vec<u8>,
    },
    NoSignal,
    Notice {
        code: u32,
    },
    Other,
}

/// Parse from the byte stream, never assume RFB fields align to WS messages.
pub fn parse(buffer: &mut VecDeque<u8>) -> Result<Option<Event>> {
    let b = buffer.make_contiguous();
    if b.is_empty() {
        return Ok(None);
    }
    let u32at = |i: usize| u32::from_be_bytes(b[i..i + 4].try_into().unwrap());
    let (len, event) = match b[0] {
        0 => {
            if b.len() < 24 {
                return Ok(None);
            }
            let rects = u16::from_be_bytes([b[2], b[3]]);
            ensure!(
                rects == 1,
                "UNSUPPORTED_PROFILE: expected one Insyde frame rectangle"
            );
            let (w, h) = (
                u16::from_be_bytes([b[8], b[9]]),
                u16::from_be_bytes([b[10], b[11]]),
            );
            let encoding = u32at(12) as i32;
            let n = u32at(20) as usize;
            ensure!(n <= 16 * 1024 * 1024, "invalid framebuffer message length");
            if b.len() < 24 + n {
                return Ok(None);
            }
            (
                24 + n,
                if n == 0 {
                    Event::NoSignal
                } else {
                    Event::Frame {
                        width: w,
                        height: h,
                        encoding,
                        data: b[24..24 + n].to_vec(),
                    }
                },
            )
        }
        2 => (1, Event::Other),
        22 => {
            if b.len() < 2 {
                return Ok(None);
            };
            (2, Event::Other)
        }
        55 => {
            if b.len() < 4 {
                return Ok(None);
            };
            (4, Event::Other)
        }
        57 => {
            if b.len() < 265 {
                return Ok(None);
            };
            (265, Event::Notice { code: u32at(5) })
        }
        4 => {
            if b.len() < 21 {
                return Ok(None);
            }
            let (w, h, valid) = (u32at(9) as usize, u32at(13) as usize, u32at(17));
            ensure!(w <= 256 && h <= 256, "invalid cursor dimensions");
            let n = 21 + if valid == 1 { 4 + w * h * 2 } else { 0 };
            if b.len() < n {
                return Ok(None);
            };
            (n, Event::Other)
        }
        t => bail!("UNSUPPORTED_PROFILE: unknown server message {t}; refusing to lose framing"),
    };
    buffer.drain(..len);
    Ok(Some(event))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uses_split_launcher_token() {
        let a = auth_response("0123456789abcdefghijklmn_extra_launcher_bytes").unwrap();
        assert_eq!(&a[..15], b"0123456789abcde");
        assert_eq!(&a[24..33], b"fghijklmn");
        assert_eq!(&a[15..24], &[0; 9]);
        assert_eq!(&a[33..], &[0; 15]);
        assert!(auth_response("short").is_err());
    }
    #[test]
    fn fragmented_frame_is_not_consumed_early() {
        let mut b = vec![
            0, 0, 0, 1, 0, 0, 0, 0, 2, 128, 1, 224, 0, 0, 0, 87, 0, 0, 0, 0, 0, 0, 0, 2,
        ];
        let mut q = VecDeque::from(b.clone());
        assert!(parse(&mut q).unwrap().is_none());
        assert_eq!(q.len(), 24);
        b.extend([3, 4]);
        q.extend([3, 4]);
        match parse(&mut q).unwrap().unwrap() {
            Event::Frame {
                width,
                height,
                data,
                ..
            } => {
                assert_eq!((width, height), (640, 480));
                assert_eq!(data, vec![3, 4]);
            }
            _ => panic!(),
        };
        assert!(q.is_empty());
    }
    #[test]
    fn every_frame_fragment_boundary_and_coalesced_message() {
        let mut frame = vec![
            0, 0, 0, 1, 0, 0, 0, 0, 0, 8, 0, 8, 0, 0, 0, 87, 0, 0, 0, 0, 0, 0, 0, 3,
        ];
        frame.extend([10, 11, 12]);
        for split in 0..frame.len() {
            let mut q = VecDeque::from(frame[..split].to_vec());
            assert!(parse(&mut q).unwrap().is_none());
            assert_eq!(q.len(), split);
            q.extend(&frame[split..]);
            q.push_back(2); // Bell in the same WebSocket message.
            assert!(matches!(parse(&mut q).unwrap(), Some(Event::Frame { .. })));
            assert!(matches!(parse(&mut q).unwrap(), Some(Event::Other)));
            assert!(q.is_empty());
        }
    }
    #[test]
    fn unknown_or_oversized_messages_fail_without_losing_framing() {
        let mut q = VecDeque::from(vec![255, 0, 0]);
        assert!(parse(&mut q).is_err());
        assert_eq!(q.len(), 3);
        let mut q = VecDeque::from(vec![
            0, 0, 0, 1, 0, 0, 0, 0, 0, 8, 0, 8, 0, 0, 0, 87, 0, 0, 0, 0, 127, 255, 255, 255,
        ]);
        assert!(parse(&mut q).is_err());
        assert_eq!(q.len(), 24);
    }
}
