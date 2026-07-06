//! `.skp` fixed header: version string + stable format GUID, and the offset at
//! which the MFC object stream begins.
//!
//! Header layout (`FF FE FF <len:u8>` UTF-16LE string records):
//! ```text
//! 0x00  "SketchUp Model"   string record
//! 0x20  "{17.3.116}"       version string
//! 0x3A  16 bytes           format GUID
//! 0x4C  4 bytes            doc-id / seed (per-save noise)
//! 0x56  FFFF 0000 ... "CVersionMap"   first MFC class record
//! ```
//! Port of `parse_header` / `object_stream_start` in the Python reference.

use crate::carchive::decode_utf16le;

pub struct Header {
    pub version: String,
    pub format_guid: String,
}

/// Read a `FF FE FF <len:u8>` UTF-16LE string record at `off`; return the string
/// and the offset just past it. `None` if the record is malformed / truncated.
fn read_str_record(d: &[u8], off: usize) -> Option<(String, usize)> {
    if off + 4 > d.len() || &d[off..off + 3] != b"\xff\xfe\xff" {
        return None;
    }
    let n = d[off + 3] as usize;
    let end = off + 4 + n * 2;
    if end > d.len() {
        return None;
    }
    Some((decode_utf16le(&d[off + 4..end]), end))
}

pub fn parse_header(d: &[u8]) -> Option<Header> {
    let (_model_tag, off) = read_str_record(d, 0)?;
    let (version, off) = read_str_record(d, off)?;
    if off + 16 > d.len() {
        return None;
    }
    let format_guid = hex(&d[off..off + 16]);
    Some(Header {
        version,
        format_guid,
    })
}

/// Offset just past the fixed header (2 str recs, GUID, doc-name str rec, doc id).
pub fn object_stream_start(d: &[u8]) -> Option<usize> {
    let (_model_tag, off) = read_str_record(d, 0)?;
    let (_version, off) = read_str_record(d, off)?;
    let off = off + 16; // format GUID
    let (_doc_name, off) = read_str_record(d, off)?;
    Some(off + 4) // doc_id u32
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
