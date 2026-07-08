//! Material-manager region walk (SKP_FORMAT §4n, revised): parse the
//! CMaterial list as the object SEQUENCE it is, so face-material archive
//! slots link to materials exactly instead of by arithmetic.
//!
//! The §4s slot arithmetic assumed each material occupies `1 + own_dib`
//! slots. Real-world files break that: materials also own a small
//! texture-placement holder object (a class-ref + 3-byte body, present on
//! SOME materials, back-referenced by others), and renderer-plugin
//! materials carry attribute dictionaries — every such object shifts every
//! later slot. This walk parses each record's byte-exact layout (validated
//! record-adjacent across the corpus plus two multi-megabyte production
//! models) and counts owned objects directly:
//!
//! ```text
//! <count:u32> FFFF <schema=12> <9>CMaterial   — the manager's declared list
//! record 0:  00 00 00 + name + variant + tail  (3-byte preamble)
//! record 1+: name + variant + tail             (back-to-back, tagless)
//! variant:
//!   solid:    00 00 + RGBA + emptystr + 8B + opacity:f64
//!   textured: 01 00 00 00 + dib(class-ref + subtype:u32 + len:u32 +
//!             payload + optional u32) + w:f64 + h:f64 + filename + rich
//!   shared:   01 00 00 00 + BACK-REF word + w + h + filename + rich
//!   rich:     avg(4) + 00 + avg2(4) + emptystr + u32 + f64 + 4B
//! tail: 1 byte + holder pointer (0000 null | XXXX back-ref | class-ref +
//!       3B body = NEW holder slot) + optional attribute objects
//!       (class-ref + dict body, one slot each)
//! footer: 3 bytes, then the layer-list count:u32 (cross-checked)
//! ```
//!
//! The walk yields each material's slot offset RELATIVE to record 0's
//! object slot. The absolute anchor is solved from the face material
//! back-refs themselves: the unique shift under which every referenced
//! slot lands on a material (validated against a production model where a
//! known definition's faces pin material 0's absolute slot). Any parse or
//! anchoring surprise makes the caller fall back to the old arithmetic —
//! loudly, via a diagnostic, never silently.

/// One walked material record: its slot offset relative to material 0.
pub(crate) struct MatSlot {
    pub rel: usize,
    /// The record's name — the correlation key against the extracted
    /// material list (which can MISS records the walk finds).
    pub name: String,
    /// The record's inline CDib slot, relative like `rel`, when it owns
    /// one — the slot a shared-texture record's back-ref word names
    /// (house.skp's "[Wood Floor Light]1" refs 23 = the original's dib
    /// at rel 6 + anchor 17).
    pub dib_rel: Option<usize>,
}

/// Walk the CMaterial region of `d`. Returns per-material relative slots,
/// in file order, when the region parses record-adjacent END TO END
/// (count records + 3-byte footer + a plausible layer count). `None` on
/// any structural surprise.
pub(crate) fn walk_region(d: &[u8]) -> Option<Vec<MatSlot>> {
    let decl = find(d, b"\x09\x00CMaterial")?;
    if decl < 8 || &d[decl - 4..decl - 2] != b"\xff\xff" {
        return None;
    }
    let count = u32le(d, decl - 8)? as usize;
    if count == 0 || count > 100_000 {
        return None;
    }
    let mut p = decl + 2 + 9; // past namelen + "CMaterial"
    let mut out = Vec::with_capacity(count);
    let mut rel = 0usize;

    // Attribute objects parsed at the END of record k's byte span number
    // AFTER record k+1's own slot (they belong to the FOLLOWING material —
    // a renderer material's dictionaries serialize just before its name).
    let mut pending_attrs = 0usize;
    for k in 0..count {
        if k == 0 {
            p = p.checked_add(3)?; // 00 00 00 preamble
        }
        let start_rel = rel;
        rel += 1 + pending_attrs;
        let dib_at = rel; // an inline dib takes the record's next owned slot
        let mut dibs = 0usize;
        let mut attrs = 0usize;
        let (name, np) = record(d, p, &mut dibs, &mut attrs)?;
        rel += dibs;
        pending_attrs = attrs;
        out.push(MatSlot {
            rel: start_rel,
            name,
            dib_rel: (dibs > 0).then_some(dib_at),
        });
        p = np;
    }

    // The layer-list count follows the last record directly (sanity:
    // 1..=4096, matching walk2's layer-count plausibility gate).
    let layers = u32le(d, p)?;
    if !(1..=4096).contains(&layers) {
        return None;
    }
    Some(out)
}

/// Parse ONE record starting at its name marker; returns the offset just
/// past its tail. Bumps `rel` once per additional OWNED object (inline
/// dib, new holder, attribute object).
fn record(d: &[u8], mut p: usize, dibs: &mut usize, attrs: &mut usize) -> Option<(String, usize)> {
    let (name, np) = utf16(d, p)?;
    p = np;

    if d.get(p..p + 2)? == b"\x00\x00" {
        // solid: 00 00 + RGBA + emptystr + 8B + opacity
        p += 2 + 4;
        let (s, np) = utf16(d, p)?;
        if !s.is_empty() {
            return None;
        }
        p = np + 8 + 8;
    } else if d.get(p..p + 4)? == b"\x01\x00\x00\x00" {
        p += 4;
        let w = u16le(d, p)?;
        if (w & 0x8000) != 0 && w != 0xFFFF {
            // inline dib: class-ref + subtype + len + payload (+opt u32)
            *dibs += 1;
            p += 2;
            let _subtype = u32le(d, p)?;
            let len = u32le(d, p + 4)? as usize;
            p = p.checked_add(8)?.checked_add(len)?;
            // optional post-payload u32: the filename marker sits at
            // +16 (absent) or +20 (present) past the applied-size pair
            if d.get(p + 20..p + 23) == Some(b"\xff\xfe\xff") {
                p += 4;
            } else if d.get(p + 16..p + 19) != Some(b"\xff\xfe\xff") {
                return None;
            }
        } else {
            // shared image: back-ref word, no owned slot
            p += 2;
        }
        p += 16; // applied size w + h
        let (_fname, np) = utf16(d, p)?;
        p = np;
        // rich tail: avg + 00 + avg2 + emptystr + u32 + f64 + 4B
        p += 4 + 1 + 4;
        let (s, np) = utf16(d, p)?;
        if !s.is_empty() {
            return None;
        }
        p = np + 4 + 8 + 4;
    } else {
        return None;
    }

    // Common tail: 1 byte + HOLDER pointer (a new holder has a 0-byte
    // body and owns one slot) + ATTRIBUTES pointer (null, or a
    // CAttributeContainer class-ref whose body is a 3-byte preamble +
    // CAttributeNamed children until a null tag — one slot for the
    // container and one per dict) + a 1-byte trailer.
    p += 1;
    let w = u16le(d, p)?;
    p += 2;
    // A NEW holder (class-ref, empty body) does NOT consume a map slot:
    // production-model face refs pin material strides at 1 + inline dib
    // (+ attribute objects) with no holder contribution.
    let _ = w;
    let w2 = u16le(d, p)?;
    p += 2;
    if (w2 & 0x8000) != 0 && w2 != 0xFFFF {
        *attrs += 1; // the attribute container
        p += 3; // its pid-less preamble
        loop {
            let c = u16le(d, p)?;
            p += 2;
            if c == 0 {
                break; // container's null terminator
            }
            if (c & 0x8000) == 0 || c == 0xFFFF {
                return None; // grammar surprise
            }
            *attrs += 1; // one CAttributeNamed dict
            p += 3 + 4; // preamble + 4B
            let (_dict, np) = utf16(d, p)?;
            p = np;
            loop {
                let (key, np) = utf16(d, p)?;
                p = np;
                if key.is_empty() {
                    break;
                }
                let t = *d.get(p)?;
                p += 1;
                p = attr_value_end(d, p, t, 0)?;
            }
            p += 4; // dict u32 tail
        }
    }
    // null / back-ref attributes pointer: nothing owned.
    p += 1; // trailer
    Some((name, p))
}

/// Span of one typed attribute value (mirrors `entity::attr_value`).
fn attr_value_end(d: &[u8], p: usize, t: u8, depth: u32) -> Option<usize> {
    if depth > 8 {
        return None;
    }
    match t {
        0x00 => Some(p),
        0x04 => Some(p + 4),
        0x06 | 0x0c => Some(p + 8),
        0x07 => Some(p + 1),
        0x0a => utf16(d, p).map(|(_, np)| np),
        0x0b => {
            let n = u32le(d, p)? as usize;
            if n > 1_000_000 {
                return None;
            }
            let mut q = p + 4;
            for _ in 0..n {
                let et = *d.get(q)?;
                q = attr_value_end(d, q + 1, et, depth + 1)?;
            }
            Some(q)
        }
        _ => None,
    }
}

/// Exact face-material links from the walked region: the unique shift
/// placing every referenced slot on a material. `refs` are the distinct
/// face material back-ref slots; `bound` is an exclusive upper bound for
/// material slots (the pre-model slot count on the continuous path).
pub(crate) fn links_from_walk(
    slots: &[MatSlot],
    refs: &[u16],
    bound: usize,
) -> Option<Vec<(u16, usize)>> {
    if slots.is_empty() || refs.is_empty() {
        return None;
    }
    let rels: Vec<usize> = slots.iter().map(|s| s.rel).collect();
    let rmin = *refs.iter().min()? as usize;
    let mut solutions: Vec<usize> = Vec::new();
    for &r in &rels {
        let Some(shift) = rmin.checked_sub(r) else {
            continue;
        };
        if shift == 0 {
            continue; // slot 0 is the MFC null index
        }
        let set: std::collections::BTreeSet<usize> = rels.iter().map(|&x| x + shift).collect();
        if refs.iter().all(|&q| set.contains(&(q as usize)))
            && rels.iter().all(|&x| x + shift < bound)
        {
            solutions.push(shift);
        }
    }
    solutions.sort_unstable();
    solutions.dedup();
    let &shift = match solutions.as_slice() {
        [one] => one,
        // Multiple fits: ambiguous — refuse rather than guess.
        _ => return None,
    };
    Some(
        rels.iter()
            .enumerate()
            .filter_map(|(i, &x)| u16::try_from(x + shift).ok().map(|s| (s, i)))
            .collect(),
    )
}

fn find(d: &[u8], needle: &[u8]) -> Option<usize> {
    d.windows(needle.len()).position(|w| w == needle)
}

fn u16le(d: &[u8], p: usize) -> Option<u16> {
    d.get(p..p + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32le(d: &[u8], p: usize) -> Option<u32> {
    d.get(p..p + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// UTF-16 string record at exactly `p` -> (string, end offset).
fn utf16(d: &[u8], p: usize) -> Option<(String, usize)> {
    let (n, chars_at) = crate::carchive::mfc_strlen(d, p)?;
    let end = chars_at + n * 2;
    let raw = d.get(chars_at..end)?;
    let s: String = char::decode_utf16(
        raw.chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]])),
    )
    .map(|c| c.unwrap_or('\u{FFFD}'))
    .collect();
    Some((s, end))
}

#[cfg(test)]
mod specs {
    use super::*;
    use std::path::Path;

    fn corpus(name: &str) -> Vec<u8> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpus/2017")
            .join(name);
        std::fs::read(p).expect("corpus file")
    }

    /// The pinned end-to-end walks: record-adjacency across every corpus
    /// shape (solid, textured, shared, semi-transparent, back-ref holder).
    #[test]
    fn corpus_regions_walk_end_to_end() {
        for (file, mats) in [
            ("box-two-materials.skp", 3),
            ("material-one-face.skp", 2),
            ("house.skp", 9),
        ] {
            let d = corpus(file);
            let slots = walk_region(&d).unwrap_or_else(|| panic!("{file} must walk"));
            assert_eq!(slots.len(), mats, "{file} material count");
            eprintln!("{file}: last rel {}", slots.last().unwrap().rel);
        }
        // house.skp's dib layout: five inline dibs, each on the slot after
        // its material; the shared "[Wood Floor Light]1" (rel 11) owns
        // none — its back-ref 23 lands on rel 6 under the file's anchor 17.
        let slots = walk_region(&corpus("house.skp")).unwrap();
        let dibs: Vec<(usize, Option<usize>)> = slots.iter().map(|s| (s.rel, s.dib_rel)).collect();
        assert_eq!(
            dibs,
            [
                (0, None),
                (1, Some(2)),
                (3, None),
                (4, None),
                (5, Some(6)),
                (7, Some(8)),
                (9, Some(10)),
                (11, None),
                (12, Some(13)),
            ]
        );
    }

    /// The 10.7 MB production model: all 81 manager materials walk
    /// record-adjacent (skips silently without the sibling clone).
    #[test]
    fn theater_region_walks_end_to_end() {
        let p =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/third-party/theater-2017.skp");
        let Ok(d) = std::fs::read(p) else {
            return;
        };
        let slots = walk_region(&d).expect("theater walks");
        assert_eq!(slots.len(), 81);
        assert_eq!(slots.last().unwrap().rel, 96);
    }

    /// Anchoring: a unique shift is found, ambiguity is refused.
    #[test]
    fn links_anchor_on_unique_shift() {
        let slots = vec![
            MatSlot {
                rel: 0,
                name: "a".into(),
                dib_rel: Some(1),
            },
            MatSlot {
                rel: 2,
                name: "b".into(),
                dib_rel: None,
            },
            MatSlot {
                rel: 3,
                name: "c".into(),
                dib_rel: None,
            },
        ];
        // refs {20, 23} fit ONLY shift 20 (mat0->20, mat2->23).
        let links = links_from_walk(&slots, &[20, 23], 100).expect("unique");
        assert!(links.contains(&(20, 0)));
        assert!(links.contains(&(23, 2)));
        // refs {22} fits shifts 20 AND 22 and 19 -> ambiguous -> None.
        assert!(links_from_walk(&slots, &[22], 100).is_none());
    }
}
