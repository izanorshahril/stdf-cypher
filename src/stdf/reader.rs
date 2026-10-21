//! Streaming STDF reader. Handles plain and gzip-compressed files and
//! auto-detects byte order from the FAR record.

use super::codec::Endian;
use super::records::Record;
use anyhow::{bail, Context, Result};
use flate2::read::MultiGzDecoder;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// Header of one record.
#[derive(Debug, Clone, Copy)]
pub struct RecHeader {
    pub len: u16,
    pub typ: u8,
    pub sub: u8,
    /// byte offset of the header in the (decompressed) stream
    pub offset: u64,
}

pub struct StdfReader {
    inner: Box<dyn Read + Send>,
    pub endian: Endian,
    pub stdf_ver: u8,
    offset: u64,
    pending_far: Option<Vec<u8>>,
    buf: Vec<u8>,
}

impl StdfReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut magic = [0u8; 2];
        let n = f.read(&mut magic)?;
        drop(f);
        let f = File::open(path)?;
        let rdr: Box<dyn Read + Send> = if n == 2 && magic == [0x1f, 0x8b] {
            Box::new(BufReader::with_capacity(
                1 << 20,
                MultiGzDecoder::new(BufReader::new(f)),
            ))
        } else {
            Box::new(BufReader::with_capacity(1 << 20, f))
        };
        Self::from_reader(rdr).with_context(|| format!("reading {}", path.display()))
    }

    pub fn from_reader(mut inner: Box<dyn Read + Send>) -> Result<Self> {
        let mut hdr = [0u8; 4];
        inner
            .read_exact(&mut hdr)
            .context("file too short for STDF")?;
        if hdr[2] != 0 || hdr[3] != 10 {
            bail!(
                "not a STDF file: first record is not FAR (got type {} sub {})",
                hdr[2],
                hdr[3]
            );
        }
        let len_le = u16::from_le_bytes([hdr[0], hdr[1]]);
        let len_be = u16::from_be_bytes([hdr[0], hdr[1]]);
        let len = if len_le == 2 {
            len_le
        } else if len_be == 2 {
            len_be
        } else {
            len_le.min(len_be)
        } as usize;
        let mut body = vec![0u8; len];
        inner.read_exact(&mut body)?;
        let cpu = body.first().copied().unwrap_or(2);
        let endian = if len_be == 2 && len_le != 2 {
            Endian::Big
        } else if len_le == 2 && len_be != 2 {
            Endian::Little
        } else {
            Endian::from_cpu_type(cpu)
        };
        let stdf_ver = body.get(1).copied().unwrap_or(4);
        if stdf_ver != 4 {
            eprintln!("warning: STDF_VER = {stdf_ver}, this tool targets V4; continuing");
        }
        let mut far = hdr.to_vec();
        far.extend_from_slice(&body);
        Ok(Self {
            inner,
            endian,
            stdf_ver,
            offset: 0,
            pending_far: Some(far),
            buf: Vec::with_capacity(1024),
        })
    }

    /// Read next record into the internal buffer. Returns header and payload
    /// slice. The slice is valid until the next call.
    pub fn next_raw(&mut self) -> Result<Option<(RecHeader, &[u8])>> {
        if let Some(far) = self.pending_far.take() {
            self.buf = far[4..].to_vec();
            let h = RecHeader {
                len: self.buf.len() as u16,
                typ: 0,
                sub: 10,
                offset: 0,
            };
            self.offset = far.len() as u64;
            return Ok(Some((h, &self.buf)));
        }
        let mut hdr = [0u8; 4];
        match read_full(&mut self.inner, &mut hdr)? {
            0 => return Ok(None),
            4 => {}
            n => {
                eprintln!("warning: trailing {n} bytes at end of file ignored");
                return Ok(None);
            }
        }
        let len = match self.endian {
            Endian::Little => u16::from_le_bytes([hdr[0], hdr[1]]),
            Endian::Big => u16::from_be_bytes([hdr[0], hdr[1]]),
        };
        self.buf.resize(len as usize, 0);
        let got = read_full(&mut self.inner, &mut self.buf)?;
        if got < len as usize {
            eprintln!("warning: last record truncated ({got}/{len} bytes); stopping");
            return Ok(None);
        }
        let h = RecHeader {
            len,
            typ: hdr[2],
            sub: hdr[3],
            offset: self.offset,
        };
        self.offset += 4 + len as u64;
        Ok(Some((h, &self.buf)))
    }

    /// Next record as raw bytes including the 4-byte header (for pass-through writing).
    pub fn next_raw_full(&mut self, out: &mut Vec<u8>) -> Result<Option<RecHeader>> {
        let endian = self.endian;
        match self.next_raw()? {
            None => Ok(None),
            Some((h, data)) => {
                out.clear();
                match endian {
                    Endian::Little => out.extend_from_slice(&h.len.to_le_bytes()),
                    Endian::Big => out.extend_from_slice(&h.len.to_be_bytes()),
                }
                out.push(h.typ);
                out.push(h.sub);
                out.extend_from_slice(data);
                Ok(Some(h))
            }
        }
    }

    pub fn next_record(&mut self) -> Result<Option<(RecHeader, Record)>> {
        let endian = self.endian;
        Ok(self
            .next_raw()?
            .map(|(h, d)| (h, Record::parse(h.typ, h.sub, d, endian))))
    }
}

impl Iterator for StdfReader {
    type Item = Result<Record>;
    fn next(&mut self) -> Option<Self::Item> {
        match self.next_record() {
            Ok(Some((_, r))) => Some(Ok(r)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

fn read_full(r: &mut dyn Read, buf: &mut [u8]) -> Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(n)
}
