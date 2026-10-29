//! Streaming STDF reader. Handles plain and gzip-compressed files and
//! auto-detects byte order from the FAR record.

use super::codec::Endian;
use super::records::Record;
use anyhow::{bail, Context, Result};
use bzip2::read::MultiBzDecoder;
use flate2::read::{DeflateDecoder, MultiGzDecoder};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use zip::CompressionMethod;

/// Reads up to `buf.len()` bytes (fewer at the end of the file).
fn read_up_to(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// The bytes of a zip entry, checked against the CRC-32 and the size its directory records: a
/// damaged entry is an error rather than measurements that quietly change.
struct Checked<R> {
    inner: R,
    crc: u32,
    seen: u64,
    want_crc: u32,
    want_len: u64,
}

impl<R> Checked<R> {
    fn new(inner: R, want_crc: u32, want_len: u64) -> Self {
        Self {
            inner,
            crc: 0xFFFF_FFFF,
            seen: 0,
            want_crc,
            want_len,
        }
    }
}

impl<R: Read> Read for Checked<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n == 0 {
            if !buf.is_empty() && (self.seen != self.want_len || !self.crc != self.want_crc) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "zip entry does not match its checksum or size",
                ));
            }
            return Ok(0);
        }
        self.crc = crc32_update(self.crc, &buf[..n]);
        self.seen += n as u64;
        Ok(n)
    }
}

/// CRC-32 as zip records it: the reflected IEEE polynomial, one table entry per byte value.
const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for &b in bytes {
        crc = CRC_TABLE[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc
}

/// Header of one record.
#[derive(Debug, Clone, Copy)]
pub struct RecHeader {
    pub len: u16,
    pub typ: u8,
    pub sub: u8,
    /// byte offset of the header in the (decompressed) stream
    pub offset: u64,
}

/// Read buffer size, as rust-stdf uses (2 MiB).
const READ_BUF: usize = 2 << 20;

/// Reader input: a plain or gzip file, or any other stream. Matching on the variant keeps
/// the per-record reads static (no virtual call per read), which is most of the framing time.
pub enum Input {
    Plain(BufReader<File>),
    Gzip(Box<BufReader<MultiGzDecoder<BufReader<File>>>>),
    Other(Box<dyn Read + Send>),
}

impl Read for Input {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Input::Plain(r) => r.read(buf),
            Input::Gzip(r) => r.read(buf),
            Input::Other(r) => r.read(buf),
        }
    }
}

pub struct StdfReader {
    inner: Input,
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
        let mut magic = [0u8; 4];
        let n = read_up_to(&mut f, &mut magic)?;
        drop(f);
        if n == 4 && magic == [b'P', b'K', 3, 4] {
            return Self::open_zip(path).with_context(|| format!("reading {}", path.display()));
        }
        let f = File::open(path)?;
        let rdr = if n >= 2 && magic[..2] == [0x1f, 0x8b] {
            Input::Gzip(Box::new(BufReader::with_capacity(
                READ_BUF,
                MultiGzDecoder::new(BufReader::new(f)),
            )))
        } else if n >= 3 && &magic[..3] == b"BZh" {
            Input::Other(Box::new(BufReader::with_capacity(
                READ_BUF,
                MultiBzDecoder::new(BufReader::new(f)),
            )))
        } else {
            Input::Plain(BufReader::with_capacity(READ_BUF, f))
        };
        Self::from_input(rdr).with_context(|| format!("reading {}", path.display()))
    }

    /// A zip archive: its first file entry is the STDF. The zip crate reads the archive
    /// directory; the entry's bytes are then read here, stored or deflated.
    fn open_zip(path: &Path) -> Result<Self> {
        let mut archive =
            zip::ZipArchive::new(File::open(path)?).context("not a readable zip archive")?;
        let index = (0..archive.len())
            .find(|&i| archive.by_index(i).map(|e| !e.is_dir()).unwrap_or(false))
            .context("the zip archive has no file")?;
        let entry = archive.by_index(index)?;
        if entry.encrypted() {
            bail!("encrypted zip archives are not supported");
        }
        let (start, method, size) = (
            entry.data_start(),
            entry.compression(),
            entry.compressed_size(),
        );
        let (want_crc, want_len) = (entry.crc32(), entry.size());
        drop(entry);
        let mut f = File::open(path)?;
        f.seek(SeekFrom::Start(
            start.context("zip entry has no data offset")?,
        ))?;
        let rdr: Box<dyn Read + Send> = match method {
            CompressionMethod::Stored => Box::new(BufReader::with_capacity(READ_BUF, f.take(size))),
            CompressionMethod::Deflated => Box::new(BufReader::with_capacity(
                READ_BUF,
                DeflateDecoder::new(BufReader::new(f)),
            )),
            other => bail!(
                "zip entries compressed with {other:?} are not supported (stored and deflate are)"
            ),
        };
        Self::from_reader(Box::new(Checked::new(rdr, want_crc, want_len)))
    }

    pub fn from_reader(inner: Box<dyn Read + Send>) -> Result<Self> {
        Self::from_input(Input::Other(inner))
    }

    fn from_input(mut inner: Input) -> Result<Self> {
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

fn read_full<R: Read + ?Sized>(r: &mut R, buf: &mut [u8]) -> Result<usize> {
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
