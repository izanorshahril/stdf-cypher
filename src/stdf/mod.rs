//! STDF V4 (Standard Test Data Format) parsing and writing.

pub mod codec;
pub mod reader;
pub mod records;
pub mod writer;

pub use codec::{BitField, Endian, GenData};
pub use reader::{RecHeader, StdfReader};
pub use records::*;
pub use writer::StdfWriter;
