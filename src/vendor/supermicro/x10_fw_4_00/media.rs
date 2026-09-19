//! X10 4.00 virtual USB transport, separate from the RFB console connection.
//! PDU class is big endian; payload length and USB BOT fields are little endian.
use super::{
    protocol::{Socket, connect_socket, timeout_socket},
    web::WebSession,
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use tungstenite::Message;

pub const MAX_TRANSFER: usize = 1024 * 1024;

#[derive(Debug)]
pub struct Command {
    pub device: u8,
    pub tag: [u8; 4],
    pub transfer: usize,
    pub input: bool,
    pub cdb: Vec<u8>,
}

impl Command {
    pub fn parse(device: u8, body: &[u8]) -> Result<Self> {
        ensure!(
            body.len() == 31 && &body[..4] == b"USBC",
            "MEDIA_PROTOCOL: invalid CBW"
        );
        let transfer = u32::from_le_bytes(body[8..12].try_into()?) as usize;
        ensure!(
            transfer <= MAX_TRANSFER,
            "MEDIA_PROTOCOL: transfer exceeds 1 MiB"
        );
        ensure!(
            (1..=16).contains(&body[14]) && body[13] == 0 && body[12] & 0x7f == 0,
            "MEDIA_PROTOCOL: invalid CBW fields"
        );
        Ok(Self {
            device,
            tag: body[4..8].try_into()?,
            transfer,
            input: body[12] & 0x80 != 0,
            cdb: body[15..15 + body[14] as usize].to_vec(),
        })
    }
}

fn pdu(class: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = class.to_vec();
    b.extend((body.len() as u32).to_le_bytes());
    b.extend(body);
    b
}

fn mount_packet(ticket: &str, cdrom: bool, slot: u8) -> Result<Vec<u8>> {
    ensure!(
        ticket.is_ascii() && (24..=256).contains(&ticket.len()),
        "MEDIA_PROTOCOL: unsupported ticket length"
    );
    ensure!(slot < 3, "INVALID_ARGUMENT: slot must be 0, 1 or 2");
    let mut b = pdu([0, 0, 0, 1], &[0; 44]);
    b[8..23].copy_from_slice(&ticket.as_bytes()[..15]);
    let tail = &ticket.as_bytes()[15..ticket.len().min(35)];
    b[24..24 + tail.len()].copy_from_slice(tail);
    let stamp = (crate::console::now_ms() as u32).to_be_bytes();
    b[44..48].copy_from_slice(&stamp);
    b[48] = 1 | ((slot + 1) << 1);
    b[49] = if cdrom { 3 } else { 1 };
    // USB descriptors follow the 44-byte login body, outside its length field.
    // One mass-storage interface, SCSI transparent command set, bulk-only transport.
    let descriptors: &[&[u8]] = &[
        &[
            18, 1, 0, 2, 0, 0, 0, 64, 0x1f, 0x0b, 0xea, 3, 0, 2, 0, 0, 0, 1,
        ],
        &[
            9, 2, 39, 0, 1, 1, 0, 128, 100, 9, 4, 0, 0, 3, 8, 6, 80, 0, 7, 5, 1, 2, 0, 2, 255, 7,
            5, 130, 2, 0, 2, 255, 7, 5, 131, 3, 2, 0, 1,
        ],
        &[4, 3, 9, 4],
        &[
            34, 3, 70, 0, 108, 0, 97, 0, 115, 0, 104, 0, 32, 0, 68, 0, 105, 0, 115, 0, 107, 0, 32,
            0, 32, 0, 32, 0, 32, 0, 32, 0, 32, 0,
        ],
        &[
            34, 3, 52, 0, 69, 0, 56, 0, 70, 0, 48, 0, 57, 0, 50, 0, 67, 0, 51, 0, 70, 0, 68, 0, 55,
            0, 70, 0, 56, 0, 70, 0, 55, 0,
        ],
        &[
            26, 3, 83, 0, 78, 0, 48, 0, 48, 0, 48, 0, 80, 0, 81, 0, 73, 0, 48, 0, 48, 0, 57, 0, 32,
            0,
        ],
        &[0],
        &[10, 6, 0, 2, 0, 0, 0, 64, 1, 0],
    ];
    for d in descriptors {
        b.push(d.len() as u8);
        b.extend_from_slice(d);
    }
    Ok(b)
}

pub struct Connection {
    ws: Socket,
    buffer: VecDeque<u8>,
    pending: Option<Command>,
    _web: Option<WebSession>,
    last_receive: Instant,
}

impl Connection {
    fn open(web: WebSession) -> Result<Self> {
        let ws = connect_socket(&web, "/vm")?;
        timeout_socket(&ws, Duration::from_millis(100))?;
        Ok(Self {
            ws,
            buffer: VecDeque::new(),
            pending: None,
            _web: Some(web),
            last_receive: Instant::now(),
        })
    }

    /// Query occupied slots without replacing an existing user's media.
    pub fn slots(web: &WebSession) -> Result<Vec<u8>> {
        let mut ws = connect_socket(web, "/vm")?;
        ws.send(Message::Binary(pdu([0, 0, 0, 8], &[]).into()))?;
        let mut buffer = VecDeque::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        let result = loop {
            if let Some((_, body)) = take_pdu(&mut buffer)? {
                break body;
            }
            ensure!(Instant::now() < deadline, "MEDIA_TIMEOUT: device status");
            receive(&mut ws, &mut buffer)?;
        };
        let _ = ws.close(None);
        ensure!(result.len() >= 4, "MEDIA_PROTOCOL: short device status");
        Ok(result[1..4].to_vec())
    }

    pub fn mount(web: WebSession, cdrom: bool, slot: u8) -> Result<Self> {
        ensure!(slot < 3, "INVALID_ARGUMENT: slot must be 0, 1 or 2");
        let slots = Self::slots(&web)?;
        ensure!(
            slots[slot as usize] == 255,
            "MEDIA_BUSY: slot {slot} is already occupied"
        );
        let packet = mount_packet(&web.ticket, cdrom, slot)?;
        let mut c = Self::open(web)?;
        c.ws.send(Message::Binary(packet.into()))?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            ensure!(
                Instant::now() < deadline,
                "MEDIA_TIMEOUT: mount acknowledgement"
            );
            if let Some((header, body)) = c.poll_pdu()? {
                match header {
                    [0, 0, 0, 2] => {
                        let status = *body.first().context("MEDIA_PROTOCOL: empty mount status")?;
                        ensure!(status == 0, "MEDIA_REJECTED: BMC mount status {status}");
                        c.ws.send(Message::Binary(
                            pdu([0, 0, 0, 7], &[6, 3, 1, 16, 2, 32, 3, 48]).into(),
                        ))?;
                        return Ok(c);
                    }
                    [0, 0, 0, 4] => c.keep_alive()?,
                    _ => bail!("MEDIA_PROTOCOL: unexpected mount response {header:?}"),
                }
            }
        }
    }

    fn keep_alive(&mut self) -> Result<()> {
        self.ws
            .send(Message::Binary(pdu([0, 0, 0, 3], &[255; 4]).into()))?;
        Ok(())
    }
    fn poll_pdu(&mut self) -> Result<Option<([u8; 4], Vec<u8>)>> {
        if let Some(p) = take_pdu(&mut self.buffer)? {
            return Ok(Some(p));
        }
        if receive(&mut self.ws, &mut self.buffer)? {
            self.last_receive = Instant::now();
        }
        ensure!(
            self.last_receive.elapsed() < Duration::from_secs(60),
            "MEDIA_TIMEOUT: no BMC traffic for 60 seconds; reconnect manually"
        );
        take_pdu(&mut self.buffer)
    }
    pub fn next_command(&mut self) -> Result<Option<(Command, Vec<u8>)>> {
        let Some((header, body)) = self.poll_pdu()? else {
            return Ok(None);
        };
        if header == [0, 0, 0, 4] {
            self.keep_alive()?;
            return Ok(None);
        }
        if header == [0, 0, 0, 6] {
            bail!("MEDIA_DETACHED: BMC detached media");
        }
        if let Some(cmd) = self.pending.take() {
            ensure!(
                body.len() == cmd.transfer && header[2] == cmd.device,
                "MEDIA_PROTOCOL: invalid write payload"
            );
            return Ok(Some((cmd, body)));
        }
        let cmd = Command::parse(header[2], &body)?;
        if !cmd.input && cmd.transfer != 0 {
            self.pending = Some(cmd);
            Ok(None)
        } else {
            Ok(Some((cmd, vec![])))
        }
    }
    pub fn respond(
        &mut self,
        cmd: &Command,
        data: &[u8],
        processed: usize,
        failed: bool,
    ) -> Result<()> {
        ensure!(
            data.len() <= cmd.transfer && processed <= cmd.transfer,
            "MEDIA_PROTOCOL: response too large"
        );
        let mut out = if cmd.input {
            pdu([0x22, 0, cmd.device, 0], data)
        } else {
            vec![]
        };
        let mut csw = b"USBS".to_vec();
        csw.extend(cmd.tag);
        csw.extend(((cmd.transfer - processed) as u32).to_le_bytes());
        csw.push(u8::from(failed));
        out.extend(pdu([0x22, 0, cmd.device, 255], &csw));
        self.ws.send(Message::Binary(out.into()))?;
        Ok(())
    }
    pub fn unmount(&mut self) -> Result<()> {
        self.ws
            .send(Message::Binary(pdu([0, 0, 0, 5], &[]).into()))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some((header, _)) = self.poll_pdu()? {
                if header == [0, 0, 0, 6] {
                    let _ = self.ws.close(None);
                    return Ok(());
                }
                if header == [0, 0, 0, 4] {
                    self.keep_alive()?;
                }
            }
        }
        let _ = self.ws.close(None);
        bail!("MEDIA_TIMEOUT: detach not acknowledged")
    }
}

fn take_pdu(buffer: &mut VecDeque<u8>) -> Result<Option<([u8; 4], Vec<u8>)>> {
    if buffer.len() < 8 {
        return Ok(None);
    }
    let n = u32::from_le_bytes([buffer[4], buffer[5], buffer[6], buffer[7]]) as usize;
    ensure!(n <= MAX_TRANSFER, "MEDIA_PROTOCOL: PDU exceeds 1 MiB");
    if buffer.len() < n + 8 {
        return Ok(None);
    }
    let header = [buffer[0], buffer[1], buffer[2], buffer[3]];
    buffer.drain(..8);
    Ok(Some((header, buffer.drain(..n).collect())))
}
fn receive(ws: &mut Socket, buffer: &mut VecDeque<u8>) -> Result<bool> {
    match ws.read() {
        Ok(Message::Binary(b)) => {
            ensure!(
                buffer.len() + b.len() <= 2 * MAX_TRANSFER + 128,
                "MEDIA_PROTOCOL: receive buffer limit"
            );
            buffer.extend(b);
        }
        Ok(Message::Ping(_)) => {
            ws.flush()?;
        }
        Ok(Message::Close(_)) => bail!("DISCONNECTED: virtual media WebSocket closed"),
        Ok(Message::Pong(_)) => {}
        Ok(_) => bail!("MEDIA_PROTOCOL: expected binary WebSocket message"),
        Err(tungstenite::Error::Io(e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(false);
        }
        Err(e) => return Err(e.into()),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnect_during_write_never_yields_an_executable_command() {
        use std::{
            net::{TcpListener, TcpStream},
            thread,
        };
        use tungstenite::{WebSocket, protocol::Role, stream::MaybeTlsStream};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let mut ws = WebSocket::from_raw_socket(tcp, Role::Server, None);
            let mut cbw = [0; 31];
            cbw[..4].copy_from_slice(b"USBC");
            cbw[8..12].copy_from_slice(&512u32.to_le_bytes());
            cbw[14] = 10;
            cbw[15] = 0x2a;
            cbw[23] = 1;
            ws.send(Message::Binary(pdu([0x11, 0, 2, 0], &cbw).into()))
                .unwrap();
            let payload = pdu([0x11, 0, 2, 0], &[0xa5; 512]);
            ws.send(Message::Binary(payload[..263].to_vec().into()))
                .unwrap();
            ws.close(None).unwrap();
        });
        let tcp = TcpStream::connect(address).unwrap();
        let ws = WebSocket::from_raw_socket(MaybeTlsStream::Plain(tcp), Role::Client, None);
        timeout_socket(&ws, Duration::from_millis(100)).unwrap();
        let mut c = Connection {
            ws,
            buffer: VecDeque::new(),
            pending: None,
            _web: None,
            last_receive: Instant::now(),
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(Instant::now() < deadline);
            match c.next_command() {
                Ok(None) => {}
                Ok(Some(_)) => panic!("incomplete write must not reach the file-backed target"),
                Err(e) => {
                    assert!(e.to_string().contains("DISCONNECTED"));
                    break;
                }
            }
        }
        assert!(c.pending.is_some());
        server.join().unwrap();
    }
    #[test]
    fn fragmented_and_coalesced_pdus() {
        let a = pdu([0, 0, 0, 4], &[1, 2]);
        let b = pdu([0x11, 0, 2, 0], &[3, 4, 5]);
        let mut q = VecDeque::new();
        for byte in &a[..a.len() - 1] {
            q.push_back(*byte);
            assert!(take_pdu(&mut q).unwrap().is_none());
        }
        q.push_back(a[a.len() - 1]);
        q.extend(b);
        assert_eq!(take_pdu(&mut q).unwrap().unwrap().1, vec![1, 2]);
        assert_eq!(take_pdu(&mut q).unwrap().unwrap().1, vec![3, 4, 5]);
        q.extend([0, 0, 0, 0, 255, 255, 255, 127]);
        assert!(take_pdu(&mut q).is_err());
    }
    #[test]
    fn validate_cbw_and_mount_ticket() {
        assert!(Command::parse(0, &[0; 31]).is_err());
        let mut b = [0; 31];
        b[..4].copy_from_slice(b"USBC");
        b[14] = 10;
        b[15] = 0x28;
        assert_eq!(Command::parse(0, &b).unwrap().cdb.len(), 10);
        b[14] = 17;
        assert!(Command::parse(0, &b).is_err());
        let p = mount_packet("abcdefghijklmnopqrstuvwx", false, 0).unwrap();
        assert_eq!(&p[8..23], b"abcdefghijklmno");
        assert_eq!(&p[24..33], b"pqrstuvwx");
        assert_eq!(p[49], 1);
        assert!(mount_packet("short", true, 0).is_err());
        let long = mount_packet(&"a".repeat(64), false, 0).unwrap();
        assert_eq!(&long[24..44], &[b'a'; 20]);
    }
}
