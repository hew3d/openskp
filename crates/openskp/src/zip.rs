//! Minimal read-only ZIP access for the post-2017 container (SKP_FORMAT
//! §14): the central directory, stored and DEFLATE entries, and a CRC-32
//! check on every entry read. Follows the public PKWARE APPNOTE layout; no
//! ZIP64, encryption, or multi-disk support (none observed in `.skp` files).

const EOCD_SIG: u32 = 0x0605_4b50;
const CDIR_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;

/// Largest entry this reader inflates. Observed `model.dat` entries are a
/// few MB (theater-2026: 7.6 MB); the ceiling bounds memory on hostile
/// input without constraining real models.
const MAX_ENTRY: usize = 1 << 30;
/// Largest accepted inflate ratio. The most compressible entry in the
/// corpus is 5.9:1 (style XML; `model.dat` compresses less), so 100:1 is
/// ample for real files while bounding a crafted DEFLATE stream (which can
/// reach ~1032:1) to 100× its own size.
const MAX_DEFLATE_RATIO: usize = 100;

/// One central-directory entry.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    method: u16,
    crc32: u32,
    compressed: usize,
    pub size: usize,
    /// Absolute offset of the entry's local file header in the file.
    local: usize,
}

/// A ZIP archive embedded somewhere inside `data` (the `.skp` header
/// precedes it, so every stored offset is relative to the archive start).
pub struct Archive<'a> {
    data: &'a [u8],
    pub entries: Vec<Entry>,
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

impl<'a> Archive<'a> {
    /// Locate the end-of-central-directory record and read the directory
    /// of the archive that starts at `start` (in a `.skp`, right after the
    /// header string records).
    ///
    /// The EOCD is the last 22 bytes plus an optional comment (< 64 KiB)
    /// that must run exactly to the end of the file, and its directory must
    /// place the archive's first byte at `start` — which rejects an EOCD
    /// signature, or a whole second archive, hidden inside the comment.
    pub fn open(data: &'a [u8], start: usize) -> Result<Archive<'a>, String> {
        let lo = data.len().saturating_sub(22 + 0xFFFF);
        let mut found = None;
        for o in (lo..=data.len().saturating_sub(22)).rev() {
            let ends_file = u16_at(data, o + 20).map(|c| o + 22 + c as usize) == Some(data.len());
            if u32_at(data, o) != Some(EOCD_SIG) || !ends_file {
                continue;
            }
            let (Some(count), Some(cd_size), Some(cd_off)) = (
                u16_at(data, o + 10),
                u32_at(data, o + 12),
                u32_at(data, o + 16),
            ) else {
                continue;
            };
            // ZIP64 marks these fields with all-ones sentinels.
            if count == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_off == 0xFFFF_FFFF {
                return Err("ZIP64 archives are not supported".into());
            }
            // Stored offsets count from the archive's first byte; the
            // directory ends where the EOCD begins.
            let (cd_size, cd_off) = (cd_size as usize, cd_off as usize);
            if o.checked_sub(cd_size + cd_off) == Some(start) {
                found = Some((count as usize, cd_off));
                break;
            }
        }
        let (count, cd_off) =
            found.ok_or("no ZIP end-of-central-directory record for this archive")?;
        let base = start;
        let mut entries = Vec::with_capacity(count);
        let mut o = base + cd_off;
        for _ in 0..count {
            if u32_at(data, o) != Some(CDIR_SIG) {
                return Err(format!("bad central-directory entry at {o:#x}"));
            }
            let flags = u16_at(data, o + 8).ok_or("truncated entry")?;
            let method = u16_at(data, o + 10).ok_or("truncated entry")?;
            let crc32 = u32_at(data, o + 16).ok_or("truncated entry")?;
            let compressed = u32_at(data, o + 20).ok_or("truncated entry")? as usize;
            let size = u32_at(data, o + 24).ok_or("truncated entry")? as usize;
            let nlen = u16_at(data, o + 28).ok_or("truncated entry")? as usize;
            let xlen = u16_at(data, o + 30).ok_or("truncated entry")? as usize;
            let clen = u16_at(data, o + 32).ok_or("truncated entry")? as usize;
            let local = u32_at(data, o + 42).ok_or("truncated entry")? as usize;
            let name = data
                .get(o + 46..o + 46 + nlen)
                .ok_or("truncated entry name")?;
            if flags & 1 != 0 {
                return Err("encrypted ZIP entries are not supported".into());
            }
            entries.push(Entry {
                name: String::from_utf8_lossy(name).into_owned(),
                method,
                crc32,
                compressed,
                size,
                local: base + local,
            });
            o += 46 + nlen + xlen + clen;
        }
        Ok(Archive { data, entries })
    }

    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// The uncompressed bytes of `e`, CRC-checked.
    pub fn read_entry(&self, e: &Entry) -> Result<Vec<u8>, String> {
        let d = self.data;
        if u32_at(d, e.local) != Some(LOCAL_SIG) {
            return Err(format!("{}: bad local header", e.name));
        }
        let nlen = u16_at(d, e.local + 26).ok_or("truncated local header")? as usize;
        let xlen = u16_at(d, e.local + 28).ok_or("truncated local header")? as usize;
        let start = e.local + 30 + nlen + xlen;
        let raw = d
            .get(start..start + e.compressed)
            .ok_or_else(|| format!("{}: entry data overruns the file", e.name))?;
        // Bound memory before inflating: the declared size is untrusted.
        if e.size > MAX_ENTRY
            || e.size
                > e.compressed
                    .saturating_mul(MAX_DEFLATE_RATIO)
                    .saturating_add(1024)
        {
            return Err(format!("{}: implausible declared size {}", e.name, e.size));
        }
        let out = match e.method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, e.size)
                .map_err(|err| format!("{}: inflate failed: {err:?}", e.name))?,
            m => return Err(format!("{}: unsupported compression method {m}", e.name)),
        };
        if out.len() != e.size {
            return Err(format!(
                "{}: size {} != declared {}",
                e.name,
                out.len(),
                e.size
            ));
        }
        if crc32(&out) != e.crc32 {
            return Err(format!("{}: CRC-32 mismatch", e.name));
        }
        Ok(out)
    }

    /// [`read_entry`](Self::read_entry) by name; `Ok(None)` when absent.
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        self.entry(name).map(|e| self.read_entry(e)).transpose()
    }
}

/// CRC-32 (IEEE 802.3, reflected polynomial 0xEDB88320), as ZIP stores it.
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        t
    });
    !data.iter().fold(!0u32, |c, &b| {
        table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    /// A minimal one-entry archive with `comment` appended to its EOCD.
    fn archive(name: &str, body: &[u8], comment: &[u8]) -> Vec<u8> {
        let mut z = Vec::new();
        let crc = crc32(body).to_le_bytes();
        let (n, l) = (
            (name.len() as u16).to_le_bytes(),
            (body.len() as u32).to_le_bytes(),
        );
        z.extend([0x50, 0x4b, 3, 4, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend(crc);
        z.extend(l);
        z.extend(l);
        z.extend(n);
        z.extend([0, 0]);
        z.extend(name.as_bytes());
        z.extend(body);
        let cd = z.len();
        z.extend([0x50, 0x4b, 1, 2, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend(crc);
        z.extend(l);
        z.extend(l);
        z.extend(n);
        z.extend([0; 12]);
        z.extend(0u32.to_le_bytes());
        z.extend(name.as_bytes());
        let cd_len = (z.len() - cd) as u32;
        z.extend([0x50, 0x4b, 5, 6, 0, 0, 0, 0, 1, 0, 1, 0]);
        z.extend(cd_len.to_le_bytes());
        z.extend((cd as u32).to_le_bytes());
        z.extend((comment.len() as u16).to_le_bytes());
        z.extend(comment);
        z
    }

    #[test]
    fn eocd_must_end_the_file() {
        let outer = archive("model.dat", b"outer", b"");
        assert_eq!(
            Archive::open(&outer, 0).unwrap().entries[0].name,
            "model.dat"
        );
        // A whole second archive smuggled into the comment is not the one
        // read: its EOCD does not end the file.
        let inner = archive("hidden.dat", b"inner", b"");
        let mut outer = archive("model.dat", b"outer", &inner);
        let a = Archive::open(&outer, 0).unwrap();
        assert_eq!(a.entries.len(), 1);
        assert_eq!(a.entries[0].name, "model.dat");
        assert_eq!(a.read("model.dat").unwrap().unwrap(), b"outer");
        outer.push(0); // trailing garbage: no EOCD ends the file
        assert!(Archive::open(&outer, 0).is_err());
    }

    #[test]
    fn zip64_is_refused_by_name() {
        let mut z = archive("model.dat", b"abc", b"");
        let eocd = z.len() - 22;
        z[eocd + 12..eocd + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Archive::open(&z, 0).err().unwrap().contains("ZIP64"));
    }

    #[test]
    fn crc_catches_corruption_inflation_cannot() {
        // A stored entry has no inflate step: only the CRC-32 notices.
        let mut z = archive("model.dat", b"abcdef", b"");
        z[30 + 9 + 2] ^= 0x01; // one body byte
        let a = Archive::open(&z, 0).unwrap();
        assert!(a.read("model.dat").unwrap_err().contains("CRC-32"));
    }

    #[test]
    fn implausible_declared_sizes_are_refused() {
        let mut z = archive("model.dat", b"abc", b"");
        // Declare 4 GiB uncompressed in the central directory (offset 24 of
        // the central entry, which starts after the 30+9+3-byte local entry).
        let cd = 30 + 9 + 3;
        z[cd + 24..cd + 28].copy_from_slice(&u32::MAX.to_le_bytes());
        let a = Archive::open(&z, 0).unwrap();
        assert!(a.read("model.dat").unwrap_err().contains("implausible"));
    }

    #[test]
    fn reads_every_entry_of_every_2026_file() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
        let mut files: Vec<_> = std::fs::read_dir(root.join("2026"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("skp"))
            .collect();
        files.push(root.join("third-party/theater-2026.skp"));
        for p in files {
            let d = std::fs::read(&p).unwrap();
            let start = d.windows(4).position(|w| w == b"PK\x03\x04").unwrap();
            let a = Archive::open(&d, start).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            assert!(a.entry("model.dat").is_some(), "{}", p.display());
            for e in &a.entries {
                a.read_entry(e)
                    .unwrap_or_else(|err| panic!("{}: {err}", p.display()));
            }
        }
    }
}
