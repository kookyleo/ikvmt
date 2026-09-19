//! Small file-backed SCSI target. Unknown commands fail rather than pretending success.
use crate::vendor::supermicro::x10_fw_4_00::media::{Command, MAX_TRANSFER};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions, TryLockError},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Cdrom,
    Disk,
}
#[derive(Default, Clone, Serialize)]
pub struct Statistics {
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub flushes: u64,
    pub commands: BTreeMap<String, u64>,
    pub rejected_commands: u64,
    pub last_rejection: Option<String>,
}
pub struct Image {
    file: File,
    pub path: PathBuf,
    pub size: u64,
    pub block_size: u32,
    pub kind: Kind,
    pub writable: bool,
    pub stats: Statistics,
    sense: [u8; 18],
}
pub struct Reply {
    pub data: Vec<u8>,
    pub processed: usize,
    pub failed: bool,
}

impl Image {
    pub fn open(path: &Path, kind: Kind, writable: bool) -> Result<Self> {
        ensure!(
            kind != Kind::Cdrom || !writable,
            "INVALID_ARGUMENT: CD-ROM is read-only; use kind disk for writable images"
        );
        let path = path.canonicalize()?;
        ensure!(
            path.metadata()?.is_file(),
            "INVALID_ARGUMENT: image must be a regular file, never a physical device"
        );
        let file = OpenOptions::new().read(true).write(writable).open(&path)?;
        ensure!(
            file.metadata()?.is_file(),
            "INVALID_ARGUMENT: image must be a regular file"
        );
        let lock = if writable {
            file.try_lock()
        } else {
            file.try_lock_shared()
        };
        lock.map_err(|e| match e {
            TryLockError::WouldBlock => anyhow::anyhow!("MEDIA_BUSY: image is already locked"),
            TryLockError::Error(e) => anyhow::anyhow!("MEDIA_IO: cannot lock image: {e}"),
        })?;
        let size = file.metadata()?.len();
        let block_size = if kind == Kind::Cdrom { 2048 } else { 512 };
        ensure!(
            size >= block_size as u64 && size % block_size as u64 == 0,
            "INVALID_ARGUMENT: image length must be a nonzero multiple of {block_size}"
        );
        Ok(Self {
            file,
            path,
            size,
            block_size,
            kind,
            writable,
            stats: Statistics::default(),
            sense: empty_sense(),
        })
    }
    pub fn sync(&mut self) -> Result<()> {
        if self.writable {
            self.file.sync_all()?;
            self.stats.flushes += 1;
        }
        Ok(())
    }
    fn fail(&mut self, op: u8, key: u8, asc: u8) -> Reply {
        self.sense = empty_sense();
        self.sense[2] = key;
        self.sense[12] = asc;
        self.stats.rejected_commands += 1;
        self.stats.last_rejection =
            Some(format!("opcode 0x{op:02x}, sense {key:02x}/{asc:02x}/00"));
        Reply {
            data: vec![],
            processed: 0,
            failed: true,
        }
    }
    pub fn execute(&mut self, cmd: &Command, out: &[u8]) -> Result<Reply> {
        let c = &cmd.cdb;
        let op = c[0];
        *self
            .stats
            .commands
            .entry(format!("0x{op:02x}"))
            .or_default() += 1;
        ensure!(
            cmd.transfer <= MAX_TRANSFER,
            "MEDIA_PROTOCOL: oversized SCSI transfer"
        );
        // Never interpret a truncated CDB or execute data-out commands with a data-in flag.
        let minimum = match op {
            0x88 | 0x8a | 0x91 | 0x9e => 16,
            0xa0 | 0xa8 | 0xaa | 0xbd | 0xbe => 12,
            0x23..=0x5f => 10,
            _ => 6,
        };
        if c.len() < minimum {
            return Ok(self.fail(op, 5, 0x24));
        }
        let is_read = matches!(op, 0x08 | 0x28 | 0xa8 | 0x88);
        let is_write = matches!(op, 0x0a | 0x2a | 0xaa | 0x8a);
        if is_read || is_write {
            if cmd.input != is_read {
                return Ok(self.fail(op, 5, 0x24));
            }
            if is_write && !self.writable {
                return Ok(self.fail(op, 7, 0x27));
            }
            let (lba, count) = match op {
                0x08 | 0x0a => (
                    ((u32::from(c[1] & 31) << 16) | (u32::from(c[2]) << 8) | u32::from(c[3]))
                        as u64,
                    if c[4] == 0 { 256 } else { u32::from(c[4]) },
                ),
                0x28 | 0x2a => (be32(&c[2..6]) as u64, be16(&c[7..9]) as u32),
                0xa8 | 0xaa => (be32(&c[2..6]) as u64, be32(&c[6..10])),
                _ => (u64::from_be_bytes(c[2..10].try_into()?), be32(&c[10..14])),
            };
            let length = u64::from(count) * u64::from(self.block_size);
            let offset = lba.checked_mul(self.block_size as u64);
            let Some(offset) =
                offset.filter(|n| n.checked_add(length).is_some_and(|end| end <= self.size))
            else {
                return Ok(self.fail(op, 5, 0x21));
            };
            if length != cmd.transfer as u64 || (is_write && out.len() != cmd.transfer) {
                return Ok(self.fail(op, 5, 0x24));
            }
            self.file.seek(SeekFrom::Start(offset))?;
            if is_read {
                let mut data = vec![0; length as usize];
                self.file.read_exact(&mut data)?;
                self.stats.bytes_read += length;
                return Ok(Reply {
                    data,
                    processed: length as usize,
                    failed: false,
                });
            }
            self.file.write_all(out)?;
            // WCE is advertised as zero: every successful write is durable locally.
            self.sync()?;
            self.stats.bytes_written += length;
            return Ok(Reply {
                data: vec![],
                processed: length as usize,
                failed: false,
            });
        }
        let mut data = match op {
            0x00 | 0x1b | 0x1e => {
                // TEST UNIT READY, START STOP, PREVENT ALLOW
                if cmd.transfer != 0 {
                    return Ok(self.fail(op, 5, 0x24));
                }
                if op == 0x1b {
                    self.sync()?;
                }
                vec![]
            }
            0x35 | 0x91 => {
                if cmd.transfer != 0 || cmd.input {
                    return Ok(self.fail(op, 5, 0x24));
                }
                self.sync()?;
                vec![]
            }
            0x03 => {
                let d = self.sense[..(c[4] as usize).min(18)].to_vec();
                self.sense = empty_sense();
                d
            }
            0x12 => {
                if c[1] & 1 != 0 || c[2] != 0 {
                    return Ok(self.fail(op, 5, 0x24));
                }
                let mut d = vec![0; 36];
                d[0] = self.device_type();
                d[1] = 128;
                d[2] = 5;
                d[3] = 2;
                d[4] = 31;
                d[8..16].copy_from_slice(b"IKVMT   ");
                d[16..32].copy_from_slice(if self.kind == Kind::Disk {
                    b"Virtual Disk    "
                } else {
                    b"Virtual CDROM   "
                });
                d[32..36].copy_from_slice(b"0001");
                d.truncate(be16(&c[3..5]) as usize);
                d
            }
            0x25 => {
                let mut d = ((self.size / self.block_size as u64 - 1).min(u32::MAX as u64) as u32)
                    .to_be_bytes()
                    .to_vec();
                d.extend(self.block_size.to_be_bytes());
                d
            }
            0x9e if c[1] & 31 == 0x10 => {
                let mut d = vec![0; 32];
                d[..8].copy_from_slice(&(self.size / self.block_size as u64 - 1).to_be_bytes());
                d[8..12].copy_from_slice(&self.block_size.to_be_bytes());
                d.truncate(be32(&c[10..14]) as usize);
                d
            }
            0x23 => {
                let mut d = vec![0, 0, 0, 8];
                d.extend(
                    ((self.size / self.block_size as u64).min(u32::MAX as u64) as u32)
                        .to_be_bytes(),
                );
                d.push(2);
                d.extend(&self.block_size.to_be_bytes()[1..]);
                d.truncate(be16(&c[7..9]) as usize);
                d
            }
            0x1a | 0x5a => {
                if c[2] & 0xc0 != 0 || c[3] != 0 {
                    return Ok(self.fail(op, 5, 0x24));
                }
                let page = c[2] & 63;
                let mut pages = vec![];
                if matches!(page, 8 | 63) {
                    // Caching page: WCE=0, no volatile write cache.
                    pages.extend([8, 18]);
                    pages.extend([0; 18]);
                } else if page == 0x2a && self.kind == Kind::Cdrom {
                    pages.extend([0x2a, 18, 3, 0, 0x71, 0, 0x29, 0]);
                    pages.extend([0; 12]);
                } else if page != 0 {
                    return Ok(self.fail(op, 5, 0x24));
                }
                let wp = if self.writable { 0 } else { 128 };
                let mut d = if op == 0x1a {
                    vec![3 + pages.len() as u8, 0, wp, 0]
                } else {
                    vec![0, 6 + pages.len() as u8, 0, wp, 0, 0, 0, 0]
                };
                d.extend(pages);
                d.truncate(if op == 0x1a {
                    c[4] as usize
                } else {
                    be16(&c[7..9]) as usize
                });
                d
            }
            0xa0 => {
                let mut d = vec![0; 16];
                d[3] = 8;
                d.truncate(be32(&c[6..10]) as usize);
                d
            }
            0x43 if self.kind == Kind::Cdrom => {
                // One data track and lead-out.
                if c[2] & 15 > 1 {
                    return Ok(self.fail(op, 5, 0x24));
                }
                let mut d = vec![
                    0, 18, 1, 1, 0, 0x14, 1, 0, 0, 0, 0, 0, 0, 0x14, 0xaa, 0, 0, 0, 0, 0,
                ];
                let blocks = (self.size / 2048) as u32;
                if c[1] & 2 != 0 {
                    d[9] = 0;
                    d[10] = 2;
                    let frames = blocks + 150;
                    d[17] = (frames / 4500) as u8;
                    d[18] = ((frames / 75) % 60) as u8;
                    d[19] = (frames % 75) as u8;
                } else {
                    d[16..20].copy_from_slice(&blocks.to_be_bytes());
                }
                if c[6] == 0xaa {
                    d.drain(4..12);
                    d[1] = 10;
                }
                if c[2] & 15 == 1 {
                    d.truncate(12);
                    d[1] = 10;
                }
                d.truncate(be16(&c[7..9]) as usize);
                d
            }
            0x46 if self.kind == Kind::Cdrom => {
                // CD-ROM current profile, profile-list feature if requested.
                let mut d = vec![0, 0, 0, 4, 0, 0, 0, 8];
                if be16(&c[2..4]) == 0 {
                    d.extend([0, 0, 3, 8, 0, 8, 1, 0, 0, 0, 0, 0]);
                    d[3] = 16;
                }
                d.truncate(be16(&c[7..9]) as usize);
                d
            }
            0x4a if self.kind == Kind::Cdrom => {
                let mut d = vec![0, 6, 4, 0x10, 0, 2, 0, 0];
                d.truncate(be16(&c[7..9]) as usize);
                d
            }
            _ => return Ok(self.fail(op, 5, 0x20)),
        };
        if !data.is_empty() && !cmd.input {
            return Ok(self.fail(op, 5, 0x24));
        }
        // A nonzero data-out transfer is never silently accepted by a no-op command.
        if !cmd.input && cmd.transfer != 0 {
            return Ok(self.fail(op, 5, 0x24));
        }
        data.truncate(cmd.transfer);
        let processed = data.len();
        Ok(Reply {
            data,
            processed,
            failed: false,
        })
    }
    fn device_type(&self) -> u8 {
        if self.kind == Kind::Cdrom { 5 } else { 0 }
    }
}
fn empty_sense() -> [u8; 18] {
    let mut s = [0; 18];
    s[0] = 0x70;
    s[7] = 10;
    s
}
fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes(b.try_into().unwrap())
}
fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes(b.try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command(op: u8, input: bool, lba: u32, blocks: u16) -> Command {
        let mut cdb = vec![0; 10];
        cdb[0] = op;
        cdb[2..6].copy_from_slice(&lba.to_be_bytes());
        cdb[7..9].copy_from_slice(&blocks.to_be_bytes());
        Command {
            device: 0,
            tag: [0; 4],
            transfer: blocks as usize * 512,
            input,
            cdb,
        }
    }
    #[test]
    fn writes_persist_and_bounds_and_write_protection_are_enforced() {
        let f = tempfile::NamedTempFile::new().unwrap();
        f.as_file().set_len(4096).unwrap();
        let mut image = Image::open(f.path(), Kind::Disk, true).unwrap();
        assert!(Image::open(f.path(), Kind::Disk, false).is_err());
        let write = command(0x2a, false, 2, 1);
        assert!(!image.execute(&write, &[0x5a; 512]).unwrap().failed);
        assert_eq!(
            image.execute(&command(0x28, true, 2, 1), &[]).unwrap().data,
            vec![0x5a; 512]
        );
        assert!(
            image
                .execute(&command(0x2a, false, 8, 1), &[1; 512])
                .unwrap()
                .failed
        );
        assert!(image.execute(&write, &[1; 511]).unwrap().failed);
        assert!(
            image
                .execute(&command(0x2a, true, 2, 1), &[1; 512])
                .unwrap()
                .failed
        );
        drop(image);
        let bytes = std::fs::read(f.path()).unwrap();
        assert_eq!(bytes.len(), 4096);
        assert_eq!(&bytes[1024..1536], &[0x5a; 512]);
        let mut image = Image::open(f.path(), Kind::Disk, false).unwrap();
        assert!(image.execute(&write, &[1; 512]).unwrap().failed);
        assert_eq!(image.sense[2], 7);
        assert_eq!(image.sense[12], 0x27);
        assert_eq!(std::fs::read(f.path()).unwrap(), bytes);
    }
    #[test]
    fn capacity_is_last_lba_and_unknown_commands_fail() {
        let f = tempfile::NamedTempFile::new().unwrap();
        f.as_file().set_len(4096).unwrap();
        let mut image = Image::open(f.path(), Kind::Disk, false).unwrap();
        let mut c = command(0x25, true, 0, 0);
        c.transfer = 8;
        assert_eq!(
            image.execute(&c, &[]).unwrap().data,
            [0, 0, 0, 7, 0, 0, 2, 0]
        );
        c.cdb[0] = 0xff;
        assert!(image.execute(&c, &[]).unwrap().failed);
        c.cdb = vec![0x28];
        assert!(image.execute(&c, &[]).unwrap().failed);
    }
}
