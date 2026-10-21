//! STDF writer: typed records or raw pass-through bytes.

use super::codec::Endian;
use super::records::Record;
use anyhow::Result;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

pub struct StdfWriter {
    out: Box<dyn Write + Send>,
    pub endian: Endian,
    pub records: u64,
}

impl StdfWriter {
    /// Create a file. A `.gz` extension produces gzip-compressed output.
    pub fn create(path: impl AsRef<Path>, endian: Endian) -> Result<Self> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let f = BufWriter::with_capacity(1 << 20, File::create(path)?);
        let out: Box<dyn Write + Send> = if path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("gz"))
            .unwrap_or(false)
        {
            Box::new(GzEncoder::new(f, Compression::default()))
        } else {
            Box::new(f)
        };
        Ok(Self {
            out,
            endian,
            records: 0,
        })
    }

    pub fn from_writer(out: Box<dyn Write + Send>, endian: Endian) -> Self {
        Self {
            out,
            endian,
            records: 0,
        }
    }

    pub fn write(&mut self, rec: &Record) -> Result<()> {
        self.out.write_all(&rec.encode(self.endian))?;
        self.records += 1;
        Ok(())
    }

    /// Write raw record bytes (header included) as read from a file of the same endianness.
    pub fn write_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.out.write_all(bytes)?;
        self.records += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        self.out.flush()?;
        Ok(())
    }
}
