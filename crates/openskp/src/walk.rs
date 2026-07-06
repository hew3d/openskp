//! Geometry walk: locate the user-drawn entity stream(s), learn the per-file
//! class tags, and drive the [`CArchive`] reader through each definition's
//! geometry, resyncing over back-reference filler.
//!
//! Port of the walk/tag-detection functions in `tools/skpwalk.py`.

use std::collections::HashMap;

use crate::carchive::{CArchive, Child, Slot};

const GEOM_CLASSES: [&[u8]; 6] = [
    b"CEdge",
    b"CVertex",
    b"CFace",
    b"CLoop",
    b"CEdgeUse",
    b"CCurve",
];

fn is_geom_class(name: &[u8]) -> bool {
    GEOM_CLASSES.contains(&name)
}

fn u16le(d: &[u8], off: usize) -> Option<u16> {
    d.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn f64le(d: &[u8], off: usize) -> Option<f64> {
    d.get(off..off + 8).map(|b| {
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        f64::from_le_bytes(a)
    })
}

/// One walked geometry run: its start offset, the archive's full store map
/// (index 0 is the sentinel; `map[1..]` is the object pool), the top-level
/// objects read (the roots for back-reference tree traversal), and every
/// anomaly recorded while walking it (Phase 0.4).
pub struct Run {
    pub start: usize,
    /// Offset just past the last byte consumed.
    pub end: usize,
    /// Declared entity-list element count (Phase 1.1): the u32 immediately
    /// before the list, when plausible. Caps the walk exactly.
    pub frame: Option<usize>,
    /// The owning object's archive map index, as DECLARED by the file
    /// (Phase 1.2): the pre-list prelude is `<idx+1:u32> 00 00 <count:u32>`,
    /// and a `CComponentInstance`'s def-ref u16 equals this index for the
    /// referenced definition's run (validated corpus-wide, SKP_FORMAT §4h).
    pub def_index: Option<usize>,
    pub map: Vec<Slot>,
    pub objs: Vec<Child>,
    pub diagnostics: Vec<crate::Diagnostic>,
}

/// Plausibility cap for a declared entity-list count (pid-stress declares
/// 48,654; junk fragments show random u32s in the billions).
const MAX_PLAUSIBLE_LIST_COUNT: usize = 1_000_000;

/// The entity-list count u32 written immediately before the list (Phase 1.1
/// discovery, validated by construction across the corpus: a cube's list is
/// 18 = 12 CEdge + 6 CFace top-level elements — inline vertices/loops/
/// edge-uses are NOT list elements; pid-stress declares exactly 2,703×18 =
/// 48,654; nested-component's inner definition 19 = 18 + 1 child instance).
fn list_frame(d: &[u8], start: usize) -> Option<usize> {
    if start < 4 {
        return None;
    }
    let n = u32::from_le_bytes([d[start - 4], d[start - 3], d[start - 2], d[start - 1]]) as usize;
    if (1..=MAX_PLAUSIBLE_LIST_COUNT).contains(&n) {
        Some(n)
    } else {
        None
    }
}

/// The owning object's declared map index (Phase 1.2). The full pre-list
/// prelude is `<idx+1:u32> 00 00 <count:u32> <list>` — the u32 ten bytes
/// before the list is the local back-ref base minus one (SKP_FORMAT §4h), and
/// the referenced object's map index is that value minus one again:
/// corpus-wide, an instance's def-ref u16 equals it exactly (box-component
/// declares 24, its instance's def-ref is 23; two-components 22/88 → 21/87;
/// mixed-definition 90 → 89 — see the Phase 1.2 harvest in the plan).
fn list_decl(d: &[u8], start: usize) -> Option<usize> {
    if start < 10 || d[start - 6] != 0 || d[start - 5] != 0 {
        return None;
    }
    let v = u32::from_le_bytes([d[start - 10], d[start - 9], d[start - 8], d[start - 7]]) as usize;
    if (2..=MAX_PLAUSIBLE_LIST_COUNT).contains(&v) {
        Some(v - 1)
    } else {
        None
    }
}

/// Post-stall resync window (bytes scanned past a stall for the next entity
/// header). Wider than the 31-byte inter-object filler hop: a stall is a
/// desync event and each recovery is RECORDED as a [`crate::Diagnostic`], so
/// generosity costs visibility, not silence. Entities run ~30–80 bytes
/// (vertex 34, edge with two inline vertices ~80), so 256 spans several.
const RESYNC_WINDOW: usize = 256;

impl Run {
    /// Count pool objects of a class (resolved concrete topology).
    pub fn count(&self, class: &str) -> usize {
        self.map
            .iter()
            .filter(|s| matches!(s, Slot::Object(e) if e.class_name() == class))
            .count()
    }
}

/// One matched entity header: offset, class tag, pid, total header length
/// (2 tag bytes + the §4i pid field).
#[derive(Clone, Copy)]
struct Hdr {
    off: usize,
    tag: u16,
    #[allow(dead_code)] // asserted by unit tests; 0.6's all-header tag stats will consume it
    pid: u32,
    len: usize,
}

/// Parse the `00 00 <mask> <pid bytes>` field at `p` (SKP_FORMAT §4i): the
/// mask is a per-byte PRESENCE bitmask — bit k set means pid byte k is
/// stored (and nonzero), bit k clear means that byte is zero and omitted.
/// The stored bytes appear low-to-high. Encoding is canonical (the writer
/// never stores a zero byte), which is this scanner's strongest
/// false-positive filter. Raw-byte scanning accepts masks `0x02..=0x07`
/// (pids past u24 are corpus-unobserved; mask 0x01 alone is the `FF 7F`
/// big-tag false-positive shape and no genuine pid ≤ 0xFF exists in any
/// corpus file). Returns `(pid, field length incl. the 3 lead bytes)`.
fn pid_field(d: &[u8], p: usize) -> Option<(u32, usize)> {
    if p + 3 > d.len() || d[p] != 0 || d[p + 1] != 0 {
        return None;
    }
    let mask = d[p + 2];
    if !(0x02..=0x07).contains(&mask) {
        return None;
    }
    let width = mask.count_ones() as usize;
    let stored = d.get(p + 3..p + 3 + width)?;
    if stored.contains(&0) {
        return None; // canonical encodings only
    }
    let mut pid = 0u32;
    let mut i = 0;
    for k in 0..3 {
        if mask & (1 << k) != 0 {
            pid |= u32::from(stored[i]) << (8 * k);
            i += 1;
        }
    }
    Some((pid, 3 + width))
}

fn candidate_headers(d: &[u8]) -> Vec<Hdr> {
    let mut out = Vec::new();
    let n = d.len();
    let mut off = 0usize;
    while off + 7 <= n {
        let v = u16::from_le_bytes([d[off], d[off + 1]]);
        // 0xFFFF is the MFC new-class tag: `FF FF <schema:u16> <namelen:u16>`
        // shadows this pattern for 5- and 7-char class names (§4i).
        if (v & 0x8000) != 0 && v != 0xFFFF {
            if let Some((pid, flen)) = pid_field(d, off + 2) {
                out.push(Hdr {
                    off,
                    tag: v,
                    pid,
                    len: 2 + flen,
                });
                off += 2 + flen;
                continue;
            }
        }
        off += 1;
    }
    out
}

/// Map per-file class-ref tags -> classname by content (tags shift per file, so
/// classify by bytes not by hardcoded values). Statistics run over ALL
/// entity headers (Phase 0.6) — the content rules are self-validating, so
/// no pid/cluster pre-filter is needed.
pub fn detect_geometry_tags(d: &[u8]) -> HashMap<usize, String> {
    let mut verts: HashMap<usize, u32> = HashMap::new();
    let mut edges: HashMap<usize, u32> = HashMap::new();
    let mut tags: HashMap<usize, String> = HashMap::new();

    let entities = candidate_headers(d);

    // Pass 1: vertices (coordinate sanity) and faces (unit normal + loop tag).
    for &Hdr {
        off, tag: v, len, ..
    } in &entities
    {
        let idx = (v & 0x7FFF) as usize;
        let body_start = off + len;
        if body_start + 24 > d.len() {
            continue;
        }
        let (c0, c1, c2) = (
            f64le(d, body_start).unwrap(),
            f64le(d, body_start + 8).unwrap(),
            f64le(d, body_start + 16).unwrap(),
        );
        let coord_ok = [c0, c1, c2]
            .iter()
            .all(|&v| v == 0.0 || (1e-4 < v.abs() && v.abs() < 1e4));
        if coord_ok {
            *verts.entry(idx).or_insert(0) += 1;
            continue;
        }
        // face? plane must be a real unit normal AND be followed by a CLoop tag.
        let nb = body_start + 10;
        let nrm = match (f64le(d, nb), f64le(d, nb + 8), f64le(d, nb + 16)) {
            (Some(a), Some(b), Some(c)) => [a, b, c],
            _ => continue,
        };
        let mag = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
        let unit = nrm.iter().all(|c| c.abs() <= 1.0001) && (mag - 1.0).abs() < 1e-6;
        let p = body_start + 10 + 32 + 4;
        let ct = u16le(d, p).unwrap_or(0);
        if unit && (ct & 0x8000) != 0 && !verts.contains_key(&idx) {
            tags.insert(idx, "CFace".to_string());
            tags.insert((ct & 0x7FFF) as usize, "CLoop".to_string());
            let et = u16le(d, p + 7).unwrap_or(0);
            if et & 0x8000 != 0 {
                tags.insert((et & 0x7FFF) as usize, "CEdgeUse".to_string());
            }
        }
    }

    // Pass 2: edges — everything unclaimed, gated on positive evidence:
    // at least one occurrence must carry an INLINE first vertex whose tag
    // the content pass validated as CVertex. (MFC writes objects on first
    // encounter, so the first edge of any file inlines its vertices — the
    // file-wide CEdge tag always has this evidence. Phase 0.6 removed the
    // old template-pid evidence path along with the baseline.)
    for &Hdr {
        off, tag: v, len, ..
    } in &entities
    {
        let idx = (v & 0x7FFF) as usize;
        if verts.contains_key(&idx) || tags.contains_key(&idx) || edges.contains_key(&idx) {
            continue;
        }
        let body_start = off + len;
        // v0 slot: preamble already consumed (len), then the 10-byte drawbase.
        let v0 = body_start + 10;
        let inline_vertex_evidence = match u16le(d, v0) {
            Some(w) if (w & 0x8000) != 0 && w != 0xFFFF => {
                pid_field(d, v0 + 2).is_some() && verts.contains_key(&((w & 0x7FFF) as usize))
            }
            _ => false,
        };
        if inline_vertex_evidence {
            *edges.entry(idx).or_insert(0) += 1;
        }
    }
    for idx in verts.keys() {
        tags.entry(*idx).or_insert_with(|| "CVertex".to_string());
    }
    for idx in edges.keys() {
        tags.entry(*idx).or_insert_with(|| "CEdge".to_string());
    }
    tags
}

/// If the entity body at `body_off` is introduced by a lazy class definition
/// (`FFFF schema namelen <geom-class>`), return that `FFFF` offset.
fn lazy_def_start(d: &[u8], body_off: usize, back: usize) -> Option<usize> {
    if body_off < 6 {
        return None;
    }
    let low = body_off.saturating_sub(back);
    let mut p = body_off - 6;
    loop {
        if d.get(p) == Some(&0xFF) && d.get(p + 1) == Some(&0xFF) {
            if let Some(nlen) = u16le(d, p + 4).map(|n| n as usize) {
                if (3..=40).contains(&nlen)
                    && p + 6 + nlen == body_off
                    && d.get(p + 6..p + 6 + nlen).is_some_and(is_geom_class)
                {
                    return Some(p);
                }
            }
        }
        if p == low {
            break;
        }
        p -= 1;
    }
    None
}

/// Start of the user-geometry entity stream (handles top-level class-ref
/// geometry and component-definition geometry that begins with lazy class defs).
fn geometry_start(d: &[u8]) -> Option<usize> {
    let n = d.len();
    let mut markers: Vec<(usize, usize)> = Vec::new();
    let mut off = 0usize;
    while off + 5 <= n {
        if let Some((pid, _flen)) = pid_field(d, off) {
            // Plausibility bound: u16-range pids of real early geometry sit
            // well under 0x4000 (ported heuristic); pids past u16 (mask has
            // bit 2 set) carry no upper heuristic.
            let plausible = !(0x4000..=0xFFFF).contains(&pid);
            if plausible {
                if let Some(lazy) = lazy_def_start(d, off, 48) {
                    markers.push((lazy, off));
                } else if off >= 2 {
                    let tag = u16le(d, off - 2).unwrap_or(0);
                    if (tag & 0x8000) != 0 && tag != 0xFFFF {
                        markers.push((off - 2, off));
                    }
                }
            }
        }
        off += 1;
    }
    if markers.is_empty() {
        return None;
    }
    markers.sort();
    let mut clusters: Vec<Vec<(usize, usize)>> = Vec::new();
    let mut cur = vec![markers[0]];
    for &m in &markers[1..] {
        if m.0 - cur.last().unwrap().0 <= 2048 {
            cur.push(m);
        } else {
            clusters.push(std::mem::take(&mut cur));
            cur = vec![m];
        }
    }
    clusters.push(cur);
    clusters.into_iter().max_by_key(|c| c.len()).map(|c| c[0].0)
}

/// If `p` starts a lazy new-class record (`FF FF <schema:u16> <namelen:u16>
/// <name>`) for a plausible SketchUp class — ASCII alphanumeric,
/// 'C'-prefixed, which every version-map class is — followed by a valid
/// §4i pid field, return that pid. This is the entity-header shape a class
/// takes at its FIRST use in a list (dimension.skp's first
/// CDimensionLinear @0x12930 is the canonical non-geometry example).
fn lazy_header_pid(d: &[u8], p: usize) -> Option<u32> {
    if u16le(d, p)? != 0xFFFF {
        return None;
    }
    let nlen = u16le(d, p + 4)? as usize;
    if !(3..=40).contains(&nlen) {
        return None;
    }
    let name = d.get(p + 6..p + 6 + nlen)?;
    if name[0] != b'C' || !name.iter().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    pid_field(d, p + 6 + nlen).map(|(pid, _)| pid)
}

/// The pid carried by the entity header at `p` (class-ref or lazy new-class
/// def), if `p` is a header at all.
fn header_pid(d: &[u8], p: usize) -> Option<u32> {
    if p + 7 > d.len() {
        return None;
    }
    let v = u16::from_le_bytes([d[p], d[p + 1]]);
    if (v & 0x8000) != 0 && v != 0xFFFF {
        return pid_field(d, p + 2).map(|(pid, _)| pid);
    }
    lazy_header_pid(d, p)
}

/// Structural entity-list start candidates (Phase 1.3), in file order. A
/// list start bears one of two prelude signatures:
///
/// - **definition list**: `<decl:u32> 00 00 <count:u32> <list>` (§4h) —
///   the prelude that also declares the definition's map index;
/// - **model-root list**: the count u32 sits DIRECTLY after the preview
///   thumbnail PNG (`"IEND" <crc:4> <count:u32> <list>`) — verified on
///   box (count 18 @0x11e96), single-line (1), triangle (4), layers (54),
///   pid-stress (48,654).
///
/// Plus, in both forms: a valid entity header (Phase 0.6 removed the old
/// pid-baseline coupling — template content is real content).
/// This finds definition lists the densest-cluster heuristic provably
/// misses (mixed-definition's "0.5 box" @0x2f01, long-name @0x2795,
/// attributes @0x82e7, soft-smooth-edges' stale def @0x2657).
fn list_candidates(d: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    for p in 12..d.len().saturating_sub(7) {
        if list_frame(d, p).is_none() {
            continue;
        }
        let def_sig = list_decl(d, p).is_some();
        let root_sig = &d[p - 12..p - 8] == b"IEND";
        if !def_sig && !root_sig {
            continue;
        }
        if header_pid(d, p).is_none() {
            continue;
        }
        out.push(p);
    }
    out
}

/// Is `p` the start of an entity header (class-ref entity or lazy geometry def)?
fn is_header(d: &[u8], p: usize) -> bool {
    if p + 7 > d.len() {
        return false;
    }
    let v = u16::from_le_bytes([d[p], d[p + 1]]);
    // 0xFFFF has the high bit set but is the new-class tag — handled below,
    // never a class-ref entity (§4i's false-positive lesson).
    if (v & 0x8000) != 0 && v != 0xFFFF && pid_field(d, p + 2).is_some() {
        return true; // class-ref entity
    }
    lazy_header_pid(d, p).is_some()
}

/// Next header offset in `[from, from+window)`, if any.
fn next_header(d: &[u8], from: usize, window: usize) -> Option<usize> {
    let end = (from + window).min(d.len());
    (from..end).find(|&p| is_header(d, p))
}

/// Walk EVERY definition's geometry (each with a fresh archive — back-refs are
/// definition-local). Returns one [`Run`] per geometry run.
pub fn walk_definitions(d: &[u8]) -> Vec<Run> {
    let seed = detect_geometry_tags(d);
    // One context for the whole file (Phase 0.1); every per-run archive
    // carries it so body readers can consult the file's declared schemas.
    let ctx = crate::ctx::Ctx::of(d);
    let mut results = Vec::new();
    let n = d.len();
    if n < 7 {
        return results;
    }
    // Phase 1.3: structural anchor. The walk starts at the FIRST structural
    // list candidate; when a hop chain dies, it resumes at the next
    // unconsumed candidate — so every signature-bearing list is visited.
    // The densest-cluster heuristic survives only as the anchor of last
    // resort, RECORDED as Diagnostic::HeuristicAnchor.
    let candidates = list_candidates(d);
    let mut anchor_fallback: Vec<crate::Diagnostic> = Vec::new();
    let mut pos = match candidates.first().copied() {
        Some(a) => Some(a),
        None => {
            let c = geometry_start(d);
            if let Some(at) = c {
                anchor_fallback.push(crate::Diagnostic::HeuristicAnchor { at });
            }
            c
        }
    };
    let mut seen = std::collections::HashSet::new();
    while let Some(p) = pos {
        if p >= n - 7 || seen.contains(&p) {
            break;
        }
        seen.insert(p);
        let mut ar = CArchive::new(d, p);
        ar.seed_classes = seed.clone();
        ar.ctx = ctx.clone();
        let mut objs = Vec::new();
        let mut diagnostics: Vec<crate::Diagnostic> = Vec::new();
        // Phase 1.1: structural framing. When the file declares the list's
        // element count, the walk stops EXACTLY there — no trailing-junk
        // consumption, no gap-hop overshoot. Without a plausible frame the
        // gap/stall heuristics below carry the run (recorded, informational).
        let frame = list_frame(d, p);
        if frame.is_none() {
            diagnostics.push(crate::Diagnostic::HeuristicFraming { at: p });
        }
        diagnostics.append(&mut anchor_fallback); // attach to the first run
                                                  // Phase 1.2: only a structurally framed list has a trustworthy
                                                  // declared owner index (both come from the same prelude).
        let def_index = frame.and_then(|_| list_decl(d, p));
        let budget = frame.unwrap_or(usize::MAX);
        while objs.len() < budget && ar.pos < n - 7 {
            if !is_header(d, ar.pos) {
                match next_header(d, ar.pos + 1, 31) {
                    // range(pos+1, pos+32) => window of 31 past pos+1
                    Some(nx) => ar.pos = nx,
                    None => break,
                }
            }
            match ar.read_object() {
                Ok(c) => objs.push(c),
                Err(stall) => {
                    // Phase 0.4: a stall mid-run is a recorded event, never a
                    // silent end. The stream physically ending is Truncated;
                    // otherwise scan a bounded window for the next entity
                    // header — found = Resync (recover, keep walking), not
                    // found = the run's natural end (clean files end exactly
                    // this way; no diagnostic).
                    if stall.class == "<eof>" {
                        diagnostics.push(crate::Diagnostic::Truncated { at: stall.pos });
                        break;
                    }
                    // Scan origin past BOTH the stall point and wherever the
                    // failed read left the cursor (a deep stall can leave
                    // ar.pos ahead of stall.pos) — guarantees every resync
                    // moves strictly forward, so this cannot loop.
                    let scan_from = stall.pos.max(ar.pos) + 1;
                    match next_header(d, scan_from, RESYNC_WINDOW) {
                        Some(nx) => {
                            diagnostics.push(crate::Diagnostic::Resync {
                                at: stall.pos,
                                resumed_at: nx,
                                class: stall.class.clone(),
                            });
                            ar.pos = nx;
                        }
                        None => break,
                    }
                }
            }
        }
        // Phase 0.3 skips taken during this run become diagnostics too.
        diagnostics.extend(ar.skipped.iter().map(|s| crate::Diagnostic::Skipped {
            class: s.class.clone(),
            at: s.at,
            bytes: s.bytes,
        }));
        let end = ar.pos;
        results.push(Run {
            start: p,
            end,
            frame,
            def_index,
            map: std::mem::take(&mut ar.map),
            objs,
            diagnostics,
        });
        // Chain: the legacy inter-run hop first (fragment discovery on
        // under-read lists is pinned behavior); when it finds nothing, jump
        // to the next structural candidate past everything consumed.
        pos =
            next_header(d, end + 1, 1499).or_else(|| candidates.iter().copied().find(|&c| c > end));
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_field_follows_the_presence_bitmask() {
        // mask 03 → both low bytes stored, 5-byte field
        assert_eq!(pid_field(&[0, 0, 0x03, 0xd3, 0x17], 0), Some((6099, 5)));
        // mask 07 → three bytes stored, 6-byte field (pid-stress @0x23644)
        assert_eq!(
            pid_field(&[0, 0, 0x07, 0x79, 0x26, 0x01], 0),
            Some((75_385, 6))
        );
        // sparse masks: omitted bytes are zero (pid-stress @0x1b722, @0xbff8a)
        assert_eq!(pid_field(&[0, 0, 0x02, 0x15], 0), Some((0x1500, 4)));
        assert_eq!(
            pid_field(&[0, 0, 0x06, 0x5a, 0x01], 0),
            Some((0x01_5a00, 5))
        );
        assert_eq!(
            pid_field(&[0, 0, 0x05, 0x01, 0x02], 0),
            Some((0x02_0001, 5))
        );
        // canonical rule: a stored zero byte is a false positive
        assert_eq!(pid_field(&[0, 0, 0x07, 0x79, 0x26, 0x00], 0), None);
        // mask 01 alone is the FF 7F big-tag false-positive shape
        assert_eq!(pid_field(&[0, 0, 0x01, 0xff], 0), None);
        // masks past u24 are corpus-unobserved: scanner-rejected
        assert_eq!(pid_field(&[0, 0, 0x0f, 1, 2, 3, 4], 0), None);
        // lead bytes must be 00 00
        assert_eq!(pid_field(&[1, 0, 0x03, 0xd3, 0x17], 0), None);
    }

    #[test]
    fn new_class_records_are_not_class_ref_headers() {
        // `FF FF 00 00 07 00 CVertex 00 00 03 <pid>` — a new-class record for
        // a 7-char name. The naive class-ref match would read mask 07 here
        // (§4i); it must instead be recognized via the lazy-def branch only.
        let mut d = vec![0xFF, 0xFF, 0x00, 0x00, 0x07, 0x00];
        d.extend_from_slice(b"CVertex");
        d.extend_from_slice(&[0x00, 0x00, 0x03, 0xd4, 0x0c]);
        d.extend_from_slice(&[0u8; 24]);
        assert!(is_header(&d, 0), "lazy-def header must still match");
        assert!(
            candidate_headers(&d).is_empty(),
            "a new-class record must never be scanned as a class-ref entity"
        );
    }

    #[test]
    fn wide_pid_class_ref_headers_match() {
        // `1e 80 00 00 07 7a 26 01` — pid-stress @0x23644.
        let mut d = vec![0x1e, 0x80, 0x00, 0x00, 0x07, 0x7a, 0x26, 0x01];
        d.extend_from_slice(&[0u8; 8]);
        assert!(is_header(&d, 0));
        let hs = candidate_headers(&d);
        assert_eq!(hs.len(), 1);
        assert_eq!((hs[0].tag, hs[0].pid, hs[0].len), (0x801e, 75_386, 8));
    }
}
