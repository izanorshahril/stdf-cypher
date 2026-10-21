//! Low-level STDF V4 field codec.
//!
//! STDF records may legally be *truncated*: trailing optional fields can be
//! omitted when the record length ends early. [`FieldReader`] therefore
//! returns type defaults (0, empty string, NaN-free zeros) once it runs out
//! of bytes instead of failing, and exposes [`FieldReader::is_empty`] so the
//! caller can tell "field missing" apart from "field present".

use serde::Serialize;

/// Byte order of a STDF file, derived from `FAR.CPU_TYPE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub enum Endian {
    /// CPU_TYPE = 1 (Sun/Motorola, big endian)
    Big,
    /// CPU_TYPE = 2 (x86, little endian) and CPU_TYPE = 0 (VAX/PDP, handled as little endian)
    #[default]
    Little,
}

impl Endian {
    pub fn from_cpu_type(cpu: u8) -> Endian {
        match cpu {
            1 => Endian::Big,
            _ => Endian::Little,
        }
    }
    pub fn cpu_type(self) -> u8 {
        match self {
            Endian::Big => 1,
            Endian::Little => 2,
        }
    }
}

/// Sequential reader over the payload of one record.
pub struct FieldReader<'a> {
    buf: &'a [u8],
    pos: usize,
    endian: Endian,
}

macro_rules! read_num {
    ($name:ident, $t:ty, $n:expr) => {
        #[inline]
        pub fn $name(&mut self) -> $t {
            if self.pos + $n > self.buf.len() {
                self.pos = self.buf.len();
                return <$t>::default();
            }
            let mut b = [0u8; $n];
            b.copy_from_slice(&self.buf[self.pos..self.pos + $n]);
            self.pos += $n;
            match self.endian {
                Endian::Little => <$t>::from_le_bytes(b),
                Endian::Big => <$t>::from_be_bytes(b),
            }
        }
    };
}

impl<'a> FieldReader<'a> {
    pub fn new(buf: &'a [u8], endian: Endian) -> Self {
        Self {
            buf,
            pos: 0,
            endian,
        }
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pos >= self.buf.len()
    }
    #[inline]
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    read_num!(u2, u16, 2);
    read_num!(u4, u32, 4);
    read_num!(u8_, u64, 8);
    read_num!(i2, i16, 2);
    read_num!(i4, i32, 4);
    read_num!(r4, f32, 4);
    read_num!(r8, f64, 8);

    #[inline]
    pub fn u1(&mut self) -> u8 {
        if self.pos >= self.buf.len() {
            return 0;
        }
        let v = self.buf[self.pos];
        self.pos += 1;
        v
    }
    #[inline]
    pub fn i1(&mut self) -> i8 {
        self.u1() as i8
    }
    /// C1: single character
    pub fn c1(&mut self) -> char {
        if self.is_empty() {
            return ' ';
        }
        self.u1() as char
    }
    /// B1: one byte of bits
    pub fn b1(&mut self) -> u8 {
        self.u1()
    }
    fn bytes(&mut self, n: usize) -> &'a [u8] {
        let end = (self.pos + n).min(self.buf.len());
        let s = &self.buf[self.pos..end];
        self.pos = end;
        s
    }
    /// Cn: length-prefixed string
    pub fn cn(&mut self) -> String {
        let n = self.u1() as usize;
        let b = self.bytes(n);
        latin1(b)
    }
    /// Sn: U2-length-prefixed string
    pub fn sn(&mut self) -> String {
        let n = self.u2() as usize;
        latin1(self.bytes(n))
    }
    /// Cf: fixed-length string
    pub fn cf(&mut self, n: usize) -> String {
        latin1(self.bytes(n))
    }
    /// Bn: U1 count of bytes followed by that many bytes
    pub fn bn(&mut self) -> Vec<u8> {
        let n = self.u1() as usize;
        self.bytes(n).to_vec()
    }
    /// Dn: U2 count of *bits* followed by ceil(n/8) bytes
    pub fn dn(&mut self) -> BitField {
        let bits = self.u2() as usize;
        let bytes = self.bytes(bits.div_ceil(8)).to_vec();
        BitField { bits, bytes }
    }
    pub fn vec_u1(&mut self, k: usize) -> Vec<u8> {
        (0..k).map(|_| self.u1()).collect()
    }
    pub fn vec_u2(&mut self, k: usize) -> Vec<u16> {
        (0..k).map(|_| self.u2()).collect()
    }
    pub fn vec_r4(&mut self, k: usize) -> Vec<f32> {
        (0..k).map(|_| self.r4()).collect()
    }
    pub fn vec_cn(&mut self, k: usize) -> Vec<String> {
        (0..k).map(|_| self.cn()).collect()
    }
    /// kxN1: nibble array, two nibbles per byte, low nibble first
    pub fn vec_n1(&mut self, k: usize) -> Vec<u8> {
        let b = self.bytes(k.div_ceil(2));
        let mut out = Vec::with_capacity(k);
        for i in 0..k {
            let byte = b.get(i / 2).copied().unwrap_or(0);
            out.push(if i % 2 == 0 { byte & 0x0F } else { byte >> 4 });
        }
        out
    }
    /// Vn: generic data (GDR)
    pub fn vn(&mut self, k: usize) -> Vec<GenData> {
        let mut out = Vec::with_capacity(k);
        for _ in 0..k {
            if self.is_empty() {
                break;
            }
            let code = self.u1();
            let v = match code {
                0 => GenData::Pad,
                1 => GenData::U1(self.u1()),
                2 => GenData::U2(self.u2()),
                3 => GenData::U4(self.u4()),
                4 => GenData::I1(self.i1()),
                5 => GenData::I2(self.i2()),
                6 => GenData::I4(self.i4()),
                7 => GenData::R4(self.r4()),
                8 => GenData::R8(self.r8()),
                10 => GenData::Cn(self.cn()),
                11 => GenData::Bn(self.bn()),
                12 => GenData::Dn(self.dn()),
                13 => GenData::N1(self.u1() & 0x0F),
                _ => GenData::Pad,
            };
            out.push(v);
        }
        out
    }
}

fn latin1(b: &[u8]) -> String {
    // STDF strings are ASCII; map bytes 1:1 so nothing is lost on odd testers.
    b.iter()
        .map(|&c| c as char)
        .collect::<String>()
        .trim_end_matches('\0')
        .to_string()
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct BitField {
    pub bits: usize,
    pub bytes: Vec<u8>,
}

impl BitField {
    pub fn get(&self, i: usize) -> bool {
        i < self.bits
            && self
                .bytes
                .get(i / 8)
                .map(|b| b >> (i % 8) & 1 == 1)
                .unwrap_or(false)
    }
    pub fn count_ones(&self) -> usize {
        (0..self.bits).filter(|&i| self.get(i)).count()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "t", content = "v")]
pub enum GenData {
    Pad,
    U1(u8),
    U2(u16),
    U4(u32),
    I1(i8),
    I2(i16),
    I4(i32),
    R4(f32),
    R8(f64),
    Cn(String),
    Bn(Vec<u8>),
    Dn(BitField),
    N1(u8),
}

/// Encoder that builds one record payload.
pub struct FieldWriter {
    pub buf: Vec<u8>,
    endian: Endian,
}

macro_rules! write_num {
    ($name:ident, $t:ty) => {
        #[inline]
        pub fn $name(&mut self, v: $t) -> &mut Self {
            match self.endian {
                Endian::Little => self.buf.extend_from_slice(&v.to_le_bytes()),
                Endian::Big => self.buf.extend_from_slice(&v.to_be_bytes()),
            }
            self
        }
    };
}

impl FieldWriter {
    pub fn new(endian: Endian) -> Self {
        Self {
            buf: Vec::with_capacity(64),
            endian,
        }
    }
    write_num!(u2, u16);
    write_num!(u4, u32);
    write_num!(i2, i16);
    write_num!(i4, i32);
    write_num!(r4, f32);
    write_num!(r8, f64);
    pub fn u1(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }
    pub fn i1(&mut self, v: i8) -> &mut Self {
        self.buf.push(v as u8);
        self
    }
    pub fn c1(&mut self, c: char) -> &mut Self {
        self.buf.push(if c.is_ascii() { c as u8 } else { b'?' });
        self
    }
    pub fn cn(&mut self, s: &str) -> &mut Self {
        let b: Vec<u8> = s
            .chars()
            .map(|c| if (c as u32) < 256 { c as u8 } else { b'?' })
            .take(255)
            .collect();
        self.buf.push(b.len() as u8);
        self.buf.extend_from_slice(&b);
        self
    }
    pub fn bn(&mut self, b: &[u8]) -> &mut Self {
        let n = b.len().min(255);
        self.buf.push(n as u8);
        self.buf.extend_from_slice(&b[..n]);
        self
    }
    pub fn dn(&mut self, d: &BitField) -> &mut Self {
        self.u2(d.bits as u16);
        let need = d.bits.div_ceil(8);
        for i in 0..need {
            self.buf.push(d.bytes.get(i).copied().unwrap_or(0));
        }
        self
    }
    pub fn n1s(&mut self, nibbles: &[u8]) -> &mut Self {
        for pair in nibbles.chunks(2) {
            let lo = pair[0] & 0x0F;
            let hi = pair.get(1).copied().unwrap_or(0) & 0x0F;
            self.buf.push(lo | (hi << 4));
        }
        self
    }
    /// Wrap the payload into a full record (header + payload).
    pub fn finish(&self, typ: u8, sub: u8) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.buf.len() + 4);
        let len = self.buf.len().min(u16::MAX as usize) as u16;
        match self.endian {
            Endian::Little => out.extend_from_slice(&len.to_le_bytes()),
            Endian::Big => out.extend_from_slice(&len.to_be_bytes()),
        }
        out.push(typ);
        out.push(sub);
        out.extend_from_slice(&self.buf[..len as usize]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn truncated_fields_default() {
        let data = [5u8, b'h', b'e'];
        let mut r = FieldReader::new(&data, Endian::Little);
        assert_eq!(r.cn(), "he");
        assert_eq!(r.u4(), 0);
        assert!(r.is_empty());
    }
    #[test]
    fn roundtrip_both_endians() {
        for e in [Endian::Little, Endian::Big] {
            let mut w = FieldWriter::new(e);
            w.u2(0xBEEF).r4(1.5).cn("abc").i2(-7).n1s(&[1, 2, 3]);
            let mut r = FieldReader::new(&w.buf, e);
            assert_eq!(r.u2(), 0xBEEF);
            assert_eq!(r.r4(), 1.5);
            assert_eq!(r.cn(), "abc");
            assert_eq!(r.i2(), -7);
            assert_eq!(r.vec_n1(3), vec![1, 2, 3]);
        }
    }
}
