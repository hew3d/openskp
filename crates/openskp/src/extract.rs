//! Byte-scan extractors for records that sit outside the geometry pointer graph:
//! materials, layers, scenes, guides, attribute dictionaries, component
//! definitions/instances, and embedded images. Each anchors on a robust on-disk
//! signature (usually a `FF FE FF <len>` UTF-16 string record) rather than a
//! full walk.
//!
//! Port of the `skpparse.*` scanners, `skpwalk.component_*`, and
//! `extract_images.extract_images`.

use crate::{AttrValue, Attribute, Definition, Guide, Image, Instance, Layer, Material, INCH};

// ---- shared low-level helpers ----

fn u16le(d: &[u8], off: usize) -> Option<u16> {
    d.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}
fn u32le(d: &[u8], off: usize) -> Option<u32> {
    d.get(off..off + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn i32le(d: &[u8], off: usize) -> Option<i32> {
    u32le(d, off).map(|v| v as i32)
}
fn f64le(d: &[u8], off: usize) -> Option<f64> {
    d.get(off..off + 8).map(|b| {
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        f64::from_le_bytes(a)
    })
}

fn round_to(x: f64, ndigits: i32) -> f64 {
    let f = 10f64.powi(ndigits);
    (x * f).round() / f
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Strict UTF-16LE decode (skips on invalid, matching Python's try/except).
fn decode_strict(bytes: &[u8]) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .ok()
}

/// Approximates Python's `str.isprintable()` for the (ASCII-dominant) names we
/// filter: rejects control and separator characters, allows plain space.
fn is_printable(s: &str) -> bool {
    s.chars()
        .all(|c| c == ' ' || (!c.is_control() && !c.is_whitespace()))
}

/// All `FF FE FF <len:u8>` string-record markers as `(marker_start, len, end)`
/// where `end = marker_start + 4` (the start of the UTF-16 payload).
/// Non-overlapping, left-to-right (matches `re.finditer`).
fn str_markers(d: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    let n = d.len();
    let mut p = 0;
    while p + 4 <= n {
        if d[p] == 0xFF && d[p + 1] == 0xFE && d[p + 2] == 0xFF {
            out.push((p, d[p + 3] as usize, p + 4));
            p += 4;
        } else {
            p += 1;
        }
    }
    out
}

/// Decode the UTF-16 name at a marker `(len, end)`, if present and valid.
fn marker_name(d: &[u8], len: usize, end: usize) -> Option<String> {
    d.get(end..end + len * 2).and_then(decode_strict)
}

// ---- materials ----

/// All materials in file order, plus every shared-texture back-reference
/// as `(material index, referenced CDib global map slot)` — the caller
/// resolves those slots to owning materials once the §4s anchor is known
/// (the byte scan alone cannot place slots globally).
pub fn materials(d: &[u8]) -> (Vec<Material>, Vec<(usize, u16)>) {
    let n = d.len();
    let mut out = Vec::new();
    let mut shared_refs = Vec::new();
    for (_start, len, end) in str_markers(d) {
        if len == 0 {
            continue;
        }
        let name = match marker_name(d, len, end) {
            Some(s) => s,
            None => continue,
        };
        if !is_printable(&name) {
            continue;
        }
        let e = end + len * 2;
        if d.get(e..e + 2) == Some(b"\x00\x00") && d.get(e + 6..e + 10) == Some(b"\xff\xfe\xff\x00")
        {
            // solid: name + 00 00 + RGBA + empty texpath + 8B +
            // opacity:f64 + USE-OPACITY flag byte. The stored f64 is the
            // opacity slider's last position and applies ONLY when the
            // flag is set — materials routinely carry stale 0.0/0.5
            // values with the flag off and render fully opaque
            // (byte-proven against .dae diffuse-alpha ground truth across
            // the corpus and two production models).
            let opacity = match (f64le(d, e + 18), d.get(e + 26)) {
                (Some(v), Some(1)) => round_to(v, 4),
                (Some(_), Some(_)) => 1.0,
                _ => continue,
            };
            let rgba = match d.get(e + 2..e + 6) {
                Some(b) => [b[0], b[1], b[2], b[3]],
                None => continue,
            };
            out.push(Material::Solid {
                name,
                rgba,
                opacity,
            });
        } else if d.get(e..e + 6) == Some(b"\x01\x00\x00\x00\x03\x80") {
            // textured: name flag + CDib(0x8003) + image + applied size + filename.
            // The filename sits AFTER the inline image payload, whose length is
            // declared at e+10 (§4f / §4s CDib = subtype + len + payload) — so
            // skip it exactly instead of scanning a fixed window (a fixed 40 KB
            // window lost every texture bigger than it: house.skp's birch.jpg
            // is 1.7 MB, Wallpaper_Dog_Bone.jpg 56 KB).
            let mut tex = None;
            if let Some(ln) = u32le(d, e + 10) {
                let je = e + 14 + ln as usize;
                let win_end = (je + 512).min(n);
                if let Some(win) = d.get(je..win_end) {
                    for (_ws, wlen, wend) in str_markers(win) {
                        if let Some(s) = win.get(wend..wend + wlen * 2).and_then(decode_strict) {
                            let low = s.to_lowercase();
                            if [".jpg", ".jpeg", ".png", ".tif", ".bmp"]
                                .iter()
                                .any(|ext| low.ends_with(ext))
                            {
                                tex = Some(s);
                                break;
                            }
                        }
                    }
                }
            }
            // §4v: a subtype-1 (JPEG) payload is followed by a u32 (70/99
            // observed, TBD) BEFORE the applied-size f64 pair; a subtype-4
            // (PNG) payload is followed by w/h directly. The unconditional
            // +4 here used to report PNG applied sizes as 0×0.
            let asize = u32le(d, e + 10).and_then(|ln| {
                let mut je = e + 14 + ln as usize;
                if u32le(d, e + 6) == Some(1) {
                    je += 4;
                }
                match (f64le(d, je), f64le(d, je + 8)) {
                    (Some(w), Some(h)) => Some((round_to(w, 3), round_to(h, 3))),
                    _ => None,
                }
            });
            // Phase 3.5: the inline CDib payload starts at e+14 with the
            // declared length at e+10 — the exact bytes the standalone
            // image carver finds at the same offset.
            let image_bytes = u32le(d, e + 10).and_then(|ln| {
                let payload = d.get(e + 14..e + 14 + ln as usize)?;
                (payload.starts_with(b"\x89PNG") || payload.starts_with(b"\xff\xd8\xff"))
                    .then(|| payload.to_vec())
            });
            // §4v: the average RGBA sits right after the filename record —
            // the colour .dae exports use as the material's diffuse.
            let avg_rgba = u32le(d, e + 10).and_then(|ln| {
                let mut je = e + 14 + ln as usize;
                if u32le(d, e + 6) == Some(1) {
                    je += 4;
                }
                let fe = je + 16; // past the applied-size f64 pair
                let (flen, chars_at) = crate::carchive::mfc_strlen(d, fe)?;
                let ce = chars_at + flen * 2;
                d.get(ce..ce + 4).map(|b| [b[0], b[1], b[2], b[3]])
            });
            out.push(Material::Textured {
                name,
                texture: tex,
                applied_size_in: asize,
                image_bytes,
                avg_rgba,
            });
        } else if d.get(e..e + 4) == Some(b"\x01\x00\x00\x00")
            && d.get(e + 5).is_some_and(|&b| b & 0x80 == 0)
        {
            // textured, SHARED image (§4s addendum): the texture pointer is an
            // MFC back-ref u16 to an earlier material's CDib object instead of
            // an inline `03 80` — house.skp's "[Wood Floor Light]1" duplicate
            // refs slot 23, the original's dib. Layout: flag u32(1) + ref u16 +
            // f64 width + f64 height + filename + avg RGBA.
            let asize = match (f64le(d, e + 6), f64le(d, e + 14)) {
                (Some(w), Some(h)) if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 => {
                    Some((round_to(w, 3), round_to(h, 3)))
                }
                _ => continue,
            };
            let fe = e + 22;
            let tex = mfc_str_at(d, fe).filter(|s| {
                let low = s.to_lowercase();
                [".jpg", ".jpeg", ".png", ".tif", ".bmp"]
                    .iter()
                    .any(|ext| low.ends_with(ext))
            });
            if tex.is_none() {
                continue; // not the back-ref texture shape after all
            }
            let avg_rgba = crate::carchive::mfc_strlen(d, fe).and_then(|(flen, chars_at)| {
                let ce = chars_at + flen * 2;
                d.get(ce..ce + 4).map(|b| [b[0], b[1], b[2], b[3]])
            });
            // The back-ref word is the owning material's CDib global map
            // slot; the caller copies that material's bytes here.
            if let Some(r) = u16le(d, e + 4) {
                shared_refs.push((out.len(), r));
            }
            out.push(Material::Textured {
                name,
                texture: tex,
                applied_size_in: asize,
                image_bytes: None, // shared: bytes live on the referenced material
                avg_rgba,
            });
        }
    }
    (out, shared_refs)
}

/// Decode the MFC utf16 string record at exactly `off`, if one is there.
fn mfc_str_at(d: &[u8], off: usize) -> Option<String> {
    let (len, chars_at) = crate::carchive::mfc_strlen(d, off)?;
    d.get(chars_at..chars_at + len * 2).and_then(decode_strict)
}

// ---- scenes ----

pub fn scenes(d: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for (_start, len, end) in str_markers(d) {
        if len == 0 {
            continue;
        }
        let name = match marker_name(d, len, end) {
            Some(s) => s,
            None => continue,
        };
        let e = end + len * 2;
        if is_printable(&name)
            && d.get(e..e + 8) == Some(b"\xff\xfe\xff\x00\x7f\x00\x00\x00")
            && (u16le(d, e + 8).unwrap_or(0) & 0x8000) != 0
        {
            out.push(name);
        }
    }
    out
}

// ---- layers ----

pub fn layers(d: &[u8]) -> Vec<Layer> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (start, len, end) in str_markers(d) {
        if len == 0 {
            continue;
        }
        let s = match marker_name(d, len, end) {
            Some(s) => s,
            None => continue,
        };
        if let Some(display) = s.strip_prefix("Layer_") {
            if !seen.contains(display) && start >= 4 {
                let e = end + len * 2;
                seen.insert(display.to_string());
                // the u32 just before 'Layer_<name>' is the hidden flag (1=hidden).
                let hidden = u32le(d, start - 4).unwrap_or(0);
                let rgba = match d.get(e + 2..e + 6) {
                    Some(b) => [b[0], b[1], b[2], b[3]],
                    None => [0, 0, 0, 0],
                };
                out.push(Layer {
                    name: display.to_string(),
                    visible: hidden == 0,
                    rgba,
                });
            }
        }
    }
    out
}

// ---- guides ----

pub fn guides(d: &[u8]) -> Vec<Guide> {
    let needle = b"CConstructionLine";
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= d.len() {
        if &d[i..i + needle.len()] == needle {
            let p = i + needle.len() + 5 + 10; // skip preamble + drawbase
            if let (Some(px), Some(py), Some(pz), Some(dx), Some(dy), Some(dz)) = (
                f64le(d, p),
                f64le(d, p + 8),
                f64le(d, p + 16),
                f64le(d, p + 24),
                f64le(d, p + 32),
                f64le(d, p + 40),
            ) {
                let pt = [px, py, pz];
                let dr = [dx, dy, dz];
                let mag = (dr[0] * dr[0] + dr[1] * dr[1] + dr[2] * dr[2]).sqrt();
                if pt.iter().all(|x| x.abs() < 1e5) && (mag - 1.0).abs() < 1e-6 {
                    out.push(Guide {
                        point_m: [
                            round_to(pt[0] / INCH, 5),
                            round_to(pt[1] / INCH, 5),
                            round_to(pt[2] / INCH, 5),
                        ],
                        direction: [round_to(dr[0], 5), round_to(dr[1], 5), round_to(dr[2], 5)],
                    });
                }
            }
        }
        i += 1;
    }
    out
}

// ---- attributes ----

pub fn attributes(d: &[u8]) -> Vec<Attribute> {
    let n = d.len();
    let mut out = Vec::new();
    for (_start, len, end) in str_markers(d) {
        if len == 0 {
            continue;
        }
        let key = match marker_name(d, len, end) {
            Some(s) => s,
            None => continue,
        };
        // skip C[A-Z]... class names (schema byte collides with a type tag)
        let is_class = {
            let mut c = key.chars();
            matches!((c.next(), c.next()), (Some('C'), Some(b)) if b.is_ascii_uppercase())
        };
        if !is_printable(&key) || is_class {
            continue;
        }
        let e = end + len * 2;
        let t = if e < n { d[e] } else { 0 };
        match t {
            0x04 => {
                if let Some(v) = i32le(d, e + 1) {
                    if v.unsigned_abs() < 1_000_000 {
                        out.push(Attribute {
                            key,
                            value: AttrValue::Int(v),
                        });
                    }
                }
            }
            0x06 => {
                if let Some(v) = f64le(d, e + 1) {
                    if !v.is_nan() && v.abs() < 1e6 {
                        out.push(Attribute {
                            key,
                            value: AttrValue::F64(round_to(v, 6)),
                        });
                    }
                }
            }
            0x07 => {
                if let Some(b) = d.get(e + 1).copied() {
                    if b == 0 || b == 1 {
                        out.push(Attribute {
                            key,
                            value: AttrValue::Bool(b == 1),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

// ---- component definitions / instances / tree ----

struct RawDef {
    name: String,
    guid: String,
    off: usize,
}

fn component_definitions(d: &[u8]) -> Vec<RawDef> {
    let mut out = Vec::new();
    for (start, len, end) in str_markers(d) {
        if len == 0 {
            continue;
        }
        let name = match marker_name(d, len, end) {
            Some(s) => s,
            None => continue,
        };
        let after = end + len * 2;
        if is_printable(&name)
            && d.get(after..after + 8) == Some(b"\xff\xfe\xff\x00\xff\xfe\xff\x00")
            && start >= 18
        {
            out.push(RawDef {
                name,
                guid: hex(&d[start - 18..start - 2]),
                off: start,
            });
        }
    }
    out
}

struct RawInst {
    defref: u16,
    off: usize,
    translation_m: [f64; 3],
    transform: [f64; 13],
}

fn component_instances(d: &[u8]) -> Vec<RawInst> {
    let n = d.len();
    let mut out = Vec::new();
    let mut o = 16usize;
    while o + 104 < n {
        if o >= 14
            && d.get(o + 96..o + 104) == Some(b"\x00\x00\x00\x00\x00\x00\xf0\x3f")
            && d.get(o - 12..o - 2) == Some(b"\x00\x00\x00\x01\x01\x00\x00\x00\x00\x00")
        {
            let defref = u16le(d, o - 2).unwrap_or(0);
            if defref != 0 {
                let mut v = [0f64; 13];
                let mut ok = true;
                for (k, slot) in v.iter_mut().enumerate() {
                    match f64le(d, o + k * 8) {
                        Some(x) => *slot = x,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                let rows_unit = ok
                    && [0usize, 3, 6].iter().all(|&i| {
                        let m = (v[i] * v[i] + v[i + 1] * v[i + 1] + v[i + 2] * v[i + 2]).sqrt();
                        (m - 1.0).abs() < 1e-9
                    });
                if rows_unit {
                    out.push(RawInst {
                        defref,
                        off: o,
                        translation_m: [
                            round_to(v[9] / INCH, 5),
                            round_to(v[10] / INCH, 5),
                            round_to(v[11] / INCH, 5),
                        ],
                        transform: v,
                    });
                    o += 104;
                    continue;
                }
            }
        }
        o += 1;
    }
    out
}

/// Structural instance→definition link (Phase 1.2). The file DECLARES each
/// definition's archive map index in the pre-list prelude
/// (`GeometryRun::def_index`), and an instance's def-ref u16 is exactly that
/// index. What remains is pairing each declared run with its definition
/// NAME: a definition record serializes as `… <entity list> … <name> …`, so
/// the name record follows its list — each framed run claims the first
/// name record after `run.start`, bracketed by the next framed run's start.
/// Returns `defref → index into defs` only when the pairing is COMPLETE
/// (every definition claimed by exactly one declared run, every instance
/// def-ref covered); a partial pairing means some list never surfaced
/// structurally, and guessing would mislink.
fn structural_link(
    defs: &[RawDef],
    insts: &[RawInst],
    runs: &[crate::GeometryRun],
) -> Option<std::collections::HashMap<u16, usize>> {
    let declared: Vec<(usize, usize)> = runs // (start, def_index), file order
        .iter()
        .filter_map(|r| r.def_index.map(|i| (r.start, i)))
        .collect();
    if declared.len() < defs.len() {
        return None;
    }
    let mut link = std::collections::HashMap::new();
    let mut claimed = vec![false; defs.len()];
    for (k, &(start, idx)) in declared.iter().enumerate() {
        let upper = declared.get(k + 1).map_or(usize::MAX, |&(s, _)| s);
        let claim = defs
            .iter()
            .enumerate()
            .find(|(j, x)| !claimed[*j] && x.off > start && x.off < upper)
            .map(|(j, _)| j);
        if let Some(j) = claim {
            claimed[j] = true;
            if link.insert(idx as u16, j).is_some() {
                return None; // duplicate declared index — ambiguous
            }
        }
    }
    if !claimed.iter().all(|&c| c) {
        return None;
    }
    if !insts.iter().all(|i| link.contains_key(&i.defref)) {
        return None;
    }
    Some(link)
}

/// Component hierarchy: definitions (name+guid) and instances linked to
/// their definition name. Primary: the structural declared-map-index link
/// (Phase 1.2). Fallback (RECORDED as `Diagnostic::HeuristicLinkage`): the
/// previously validated positional zip — instance def-refs sorted ascending
/// correspond to definitions in serialization (file-offset) order.
#[allow(clippy::type_complexity)]
pub fn component_tree(
    d: &[u8],
    runs: &[crate::GeometryRun],
) -> (
    Vec<Definition>,
    Vec<Instance>,
    Vec<(u32, usize)>,
    Vec<crate::Diagnostic>,
) {
    let mut defs = component_definitions(d);
    defs.sort_by_key(|x| x.off);
    let insts = component_instances(d);
    let mut diagnostics = Vec::new();

    let link = match structural_link(&defs, &insts, runs) {
        Some(link) => link,
        None => {
            if !defs.is_empty() && !insts.is_empty() {
                diagnostics.push(crate::Diagnostic::HeuristicLinkage {
                    definitions: defs.len(),
                });
            }
            let mut drefs: Vec<u16> = insts.iter().map(|i| i.defref).collect();
            drefs.sort_unstable();
            drefs.dedup();
            if drefs.len() == defs.len() {
                drefs.iter().enumerate().map(|(k, &dr)| (dr, k)).collect()
            } else {
                std::collections::HashMap::new()
            }
        }
    };

    let definitions = defs
        .iter()
        .map(|x| Definition {
            name: x.name.clone(),
            guid: x.guid.clone(),
            map_index: None, // byte-scan path: the global slot is unknown
        })
        .collect();
    let instances = insts
        .iter()
        .map(|i| Instance {
            definition: link.get(&i.defref).map(|&k| defs[k].name.clone()),
            defref: i.defref as u32,
            offset: i.off,
            translation_m: i.translation_m,
            transform: i.transform,
            name: None,     // byte-scan path: instance names are unread
            is_group: None, // byte-scan path: CGroup/CComponentInstance alike
            material: None, // byte-scan path: the drawbase is unread
            hidden: None,
            layer: None,
        })
        .collect();
    let mut definition_links: Vec<(u32, usize)> =
        link.into_iter().map(|(dr, k)| (dr as u32, k)).collect();
    definition_links.sort_unstable();
    (definitions, instances, definition_links, diagnostics)
}

// ---- embedded images (CDib carving) ----

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
const JPG_MAGIC: &[u8] = b"\xff\xd8\xff";

pub fn images(d: &[u8]) -> Vec<Image> {
    let n = d.len();
    let mut out = Vec::new();
    let mut m = 0usize;
    while m < n {
        let (matched, mlen) = if d[m..].starts_with(PNG_MAGIC) {
            (true, PNG_MAGIC.len())
        } else if d[m..].starts_with(JPG_MAGIC) {
            (true, JPG_MAGIC.len())
        } else {
            (false, 1)
        };
        if matched {
            if m >= 8 {
                let fmt = u32le(d, m - 8).unwrap_or(0);
                let length = u32le(d, m - 4).unwrap_or(0) as usize;
                if (fmt == 1 || fmt == 4) && (8..=n - m).contains(&length) {
                    let kind = if d[m..].starts_with(PNG_MAGIC) {
                        "png"
                    } else {
                        "jpg"
                    };
                    out.push(Image {
                        kind: kind.to_string(),
                        bytes: length,
                    });
                }
            }
            m += mlen;
        } else {
            m += 1;
        }
    }
    out
}
