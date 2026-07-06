//! A faithful Microsoft MFC `CArchive` object-stream reader.
//!
//! This is the engine the `.skp` format needs: it implements MFC's object
//! tagging protocol (null / object back-reference / new-class / existing-class,
//! with the big-tag escapes) and the shared 1-based map index that classes and
//! objects both consume. Body parsing is dispatched per class in [`entity`].
//!
//! Reference: MFC `CArchive::ReadObject` / `ReadClass`. Constants:
//! `wNullTag=0x0000  wNewClassTag=0xFFFF  wClassTag=0x8000  wBigObjectTag=0x7FFF
//! dwBigClassTag=0x80000000`. A class-ref tag is `(0x8000 | mapIndex)`; an object
//! back-ref tag is the object's mapIndex with the high bit clear. Both indices
//! live in one running counter.
//!
//! Port of `tools/carchive.py`.

use std::collections::HashMap;

use crate::entity::{read_body, Entity};

const W_NULL: u16 = 0x0000;
const W_NEW_CLASS: u16 = 0xFFFF;
const W_CLASS: u16 = 0x8000;
const W_BIG_OBJ: u16 = 0x7FFF;
const DW_BIG_CLASS: u32 = 0x8000_0000;

/// Raised when the walker reaches a class with no registered body reader, or
/// runs off the end of the stream. Carries the exact resume point — this is the
/// decode-as-you-go worklist / the signal to stop a geometry run.
#[derive(Debug, Clone)]
pub struct Stall {
    pub class: String,
    pub pos: usize,
    pub mapindex: usize,
    /// The class the READ SITE expected when an unresolved class-ref
    /// stalled (§4s context-directed binding) — diagnostic metadata for
    /// analyzing a failed continuous walk (a pad-bindable site vs a plain
    /// one). Stall-driven CALIBRATION on it is refuted: see
    /// `walk2::calibrate`'s def-list layer-pointer anchor.
    pub expected: Option<&'static str>,
}

impl Stall {
    pub(crate) fn new(class: impl Into<String>, pos: usize, mapindex: usize) -> Self {
        Stall {
            class: class.into(),
            pos,
            mapindex,
            expected: None,
        }
    }
}

/// The result of reading one tagged pointer. Mirrors Python's tri-state return
/// (`None` / an object / an unresolved `Ref`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Child {
    /// Null pointer.
    Null,
    /// Points at an object already present in the map (inline-new just parsed,
    /// or a back-ref to an existing object). Not a resolution constraint.
    Obj(usize),
    /// A back-reference whose target is not yet an object (its slot is still a
    /// class marker or out of range). Becomes a resolution constraint.
    Ref(usize),
}

/// One entry in the MFC store map: a class definition or a (partially or fully
/// read) object. Index 0 is a sentinel so indices are 1-based like MFC.
pub enum Slot {
    Sentinel,
    Class(String),
    Object(Entity),
    /// A pre-model archive slot the continuous walk (SKP_FORMAT §4s) did not
    /// read: the walk starts mid-archive at the CLayer decl, so the slots
    /// consumed by the header/thumbnail/material region are placeholders.
    /// A class-ref into pad territory resolves by the EXPECTED class at the
    /// read site ([`CArchive::read_object_expect`]) and the binding is
    /// recorded permanently (the slot becomes `Class`).
    Pad,
}

/// How to delimit an undecodable class body (Phase 0.3). Phase 2's
/// decode-or-skip campaign populates the default table in
/// [`crate::entity::default_skip_rule`]; [`CArchive::skip_rules`] overlays it
/// (tests, per-file experiments). Skipping is only legal when the byte span
/// is KNOWN — a guessed span would desync the shared map index.
#[derive(Clone, Copy)]
#[allow(dead_code)] // entity::default_skip_rule is empty until Phase 2 populates it; tests construct both variants
pub enum SkipRule {
    /// The body is exactly `n` bytes.
    Fixed(usize),
    /// Self-delimiting: inspect `(data, body_start)` and return the total
    /// body length, or `None` when it cannot be determined (→ stall).
    Dynamic(fn(&[u8], usize) -> Option<usize>),
}

/// One skipped-but-consumed object (Phase 0.3). Recorded on the archive;
/// Phase 0.4 lifts these into `Model` diagnostics.
#[derive(Debug, Clone)]
#[allow(dead_code)] // consumed by Phase 0.4's Diagnostic surfacing; until then only tests read it
pub struct SkipRecord {
    pub class: String,
    pub at: usize,
    pub bytes: usize,
}

/// The tiered outcome of reading one tagged object (Phase 0.3). The third
/// tier, Stalled, is the `Err(Stall)` arm of the surrounding `Result`.
#[derive(Debug, Clone)]
#[allow(dead_code)] // walk adopts the tiered entry in Phase 0.4; until then only tests match on it
pub enum ReadOutcome {
    /// A registered reader decoded the body (or the tag was a null/back-ref).
    Decoded(Child),
    /// No reader ran (unknown class, or Phase 0.2's schema downgrade), but a
    /// [`SkipRule`] delimited the body: bytes consumed, exactly one map slot
    /// kept (`Slot::Object(Entity::Other)`).
    Skipped {
        child: Child,
        class: String,
        bytes: usize,
    },
}

pub struct CArchive<'a> {
    pub d: &'a [u8],
    pub pos: usize,
    pub map: Vec<Slot>,
    pub classes: Vec<String>,
    /// Recursion depth at which each new-class record was read, parallel to
    /// `classes` (Phase 2.2): list-element classes declare at depth 1,
    /// classes first seen inside another body (CSkFont in a dimension)
    /// declare deeper. Used to resolve mid-stream global class-refs.
    class_depths: Vec<u32>,
    /// map_index -> classname, seeded for sub-stream walks so class-ref tags
    /// resolve without a full walk from byte 0.
    pub seed_classes: HashMap<usize, String>,
    /// File version + declared class schemas (Phase 0.1), when the caller has
    /// them. Body readers consult this instead of assuming 2017's schemas;
    /// `None` only during the bootstrap reads that BUILD the context
    /// (`inventory`, header probing).
    pub ctx: Option<crate::ctx::Ctx>,
    /// Schema numbers captured from new-class records THIS stream defined
    /// (Phase 0.2). Authoritative over `ctx.class_schemas` for dispatch: the
    /// in-stream declaration sits immediately before the bodies it governs.
    pub seen_schemas: HashMap<String, u16>,
    /// Per-archive [`SkipRule`] overlay (Phase 0.3); wins over
    /// `entity::default_skip_rule`.
    pub skip_rules: HashMap<String, SkipRule>,
    /// Every skip taken during this walk, in stream order (Phase 0.3).
    pub skipped: Vec<SkipRecord>,
    /// Continuous single-archive mode (SKP_FORMAT §4s): the map is GLOBAL
    /// (pre-padded to the calibrated base), entity preambles start with the
    /// nullable attribute-container pointer, the §4s body readers dispatch,
    /// and unresolved class-refs STALL instead of depth-guessing — the
    /// continuous walk either follows the stream exactly or dies loudly.
    pub continuous: bool,
    /// One-shot expected class for the NEXT tag read (context-directed pad
    /// binding, §4s): consumed at the outer tag only — nested reads inside
    /// the object's body never see it.
    expect: Option<&'static str>,
    /// Every pad-slot class binding taken (slot, class) — calibration
    /// evidence, surfaced as diagnostics by the continuous walk.
    pub bound: Vec<(usize, String)>,
    depth: u32,
}

impl<'a> CArchive<'a> {
    pub fn new(d: &'a [u8], pos: usize) -> Self {
        CArchive {
            d,
            pos,
            map: vec![Slot::Sentinel],
            classes: Vec::new(),
            class_depths: Vec::new(),
            seed_classes: HashMap::new(),
            ctx: None,
            seen_schemas: HashMap::new(),
            skip_rules: HashMap::new(),
            skipped: Vec::new(),
            continuous: false,
            expect: None,
            bound: Vec::new(),
            depth: 0,
        }
    }

    /// A continuous-mode archive (SKP_FORMAT §4s) whose map is pre-padded with
    /// `base` placeholder slots — the pre-model region's share of the ONE
    /// global store map.
    pub fn new_continuous(d: &'a [u8], pos: usize, base: usize) -> Self {
        let mut ar = CArchive::new(d, pos);
        ar.continuous = true;
        for _ in 0..base {
            ar.map.push(Slot::Pad);
        }
        ar
    }

    // ---- primitives (bounds-checked: OOB stalls instead of panicking) ----
    fn need(&self, n: usize) -> Result<(), Stall> {
        if self.pos + n > self.d.len() {
            Err(Stall::new("<eof>", self.pos, self.map.len()))
        } else {
            Ok(())
        }
    }

    pub fn u1(&mut self) -> Result<u8, Stall> {
        self.need(1)?;
        let v = self.d[self.pos];
        self.pos += 1;
        Ok(v)
    }

    pub fn u2(&mut self) -> Result<u16, Stall> {
        self.need(2)?;
        let v = u16::from_le_bytes([self.d[self.pos], self.d[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    pub fn u4(&mut self) -> Result<u32, Stall> {
        self.need(4)?;
        let v = u32::from_le_bytes([
            self.d[self.pos],
            self.d[self.pos + 1],
            self.d[self.pos + 2],
            self.d[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }

    pub fn f8(&mut self) -> Result<f64, Stall> {
        self.need(8)?;
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.d[self.pos..self.pos + 8]);
        self.pos += 8;
        Ok(f64::from_le_bytes(b))
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], Stall> {
        self.need(n)?;
        let v = &self.d[self.pos..self.pos + n];
        self.pos += n;
        Ok(v)
    }

    /// UTF-16 string record: `FF FE FF` marker, MFC-escalated length, then
    /// that many UTF-16LE chars. See [`mfc_strlen`].
    pub fn utf16(&mut self) -> Result<String, Stall> {
        let (n, chars_at) = mfc_strlen(self.d, self.pos)
            .ok_or_else(|| Stall::new("<bad str record>", self.pos, self.map.len()))?;
        let end = chars_at + n * 2;
        if end > self.d.len() {
            return Err(Stall::new(
                "<truncated str record>",
                self.pos,
                self.map.len(),
            ));
        }
        let s = decode_utf16le(&self.d[chars_at..end]);
        self.pos = end;
        Ok(s)
    }

    // ---- the MFC object protocol ----

    // (See the free fn `mfc_strlen` below for the string-record length rules.)

    /// Read one tagged object as a bare [`Child`] — the shape body readers
    /// want (a skipped child is still a live map slot they can point at).
    pub fn read_object(&mut self) -> Result<Child, Stall> {
        self.read_object_outcome().map(|o| match o {
            ReadOutcome::Decoded(c) => c,
            ReadOutcome::Skipped { child, .. } => child,
        })
    }

    /// [`read_object`](Self::read_object) with a context-directed class
    /// expectation (§4s): if the tag is a class-ref into PAD territory, the
    /// slot binds permanently to `expected` (recorded in [`Self::bound`]).
    /// The expectation applies to the immediate tag only.
    pub fn read_object_expect(&mut self, expected: &'static str) -> Result<Child, Stall> {
        self.expect = Some(expected);
        let r = self.read_object();
        self.expect = None; // unconsumed (null/back-ref/new-class tag)
        r
    }

    /// Read one tagged object with the tiered outcome (Phase 0.3): Decoded /
    /// Skipped, with Stalled as the `Err` arm. Recursion-depth-guarded so a
    /// desynced walk stalls instead of overflowing the stack.
    pub fn read_object_outcome(&mut self) -> Result<ReadOutcome, Stall> {
        self.depth += 1;
        if self.depth > 400 {
            self.depth -= 1;
            return Err(Stall::new(
                "<recursion limit>",
                self.pos,
                self.depth as usize,
            ));
        }
        let r = self.read_object_inner();
        self.depth -= 1;
        r
    }

    fn read_object_inner(&mut self) -> Result<ReadOutcome, Stall> {
        let start = self.pos;
        // The expectation is consumed by THIS tag, whatever it turns out to
        // be — nested reads inside a body must never inherit it.
        let expect = self.expect.take();
        let wtag = self.u2()?;
        if std::env::var_os("SKP_TRACE").is_some() {
            eprintln!(
                "trace: @0x{start:x} wtag=0x{wtag:04x} depth={} map_len={}",
                self.depth,
                self.map.len()
            );
        }

        let (is_class, new_class, idx): (bool, bool, usize) = if wtag == W_NULL {
            return Ok(ReadOutcome::Decoded(Child::Null));
        } else if wtag == W_BIG_OBJ {
            let obtag = self.u4()?;
            let is_class = (obtag & DW_BIG_CLASS) != 0;
            let idx = (obtag & !DW_BIG_CLASS) as usize;
            (is_class, false, idx)
        } else if wtag == W_NEW_CLASS {
            (true, true, 0)
        } else {
            let is_class = (wtag & W_CLASS) != 0;
            let idx = (wtag & 0x7FFF) as usize;
            (is_class, false, idx)
        };

        if !is_class {
            // object back-reference
            if idx == 0 {
                return Ok(ReadOutcome::Decoded(Child::Null));
            }
            if idx < self.map.len() {
                if let Slot::Object(_) = self.map[idx] {
                    return Ok(ReadOutcome::Decoded(Child::Obj(idx)));
                }
            }
            return Ok(ReadOutcome::Decoded(Child::Ref(idx)));
        }

        let name: String = if new_class {
            // define a new class, then its object
            let schema = self.u2()?;
            let namelen = self.u2()? as usize;
            self.need(namelen)?;
            let raw = &self.d[self.pos..self.pos + namelen];
            if namelen > 64 || !raw.iter().all(|b| b.is_ascii()) {
                self.pos = start; // rewind: we've desynced
                return Err(Stall::new("<bad class name>", start, self.map.len()));
            }
            self.pos += namelen;
            // Phase 0.2: record the stream's own schema declaration for
            // dispatch-time range checks in entity::read_body.
            self.seen_schemas
                .insert(String::from_utf8_lossy(raw).into_owned(), schema);
            let name = String::from_utf8_lossy(raw).into_owned();
            self.map.push(Slot::Class(name.clone()));
            self.classes.push(name.clone());
            self.class_depths.push(self.depth);
            name
        } else {
            // object of an already-seen (or seeded) class
            let from_map = match self.map.get(idx) {
                Some(Slot::Class(n)) => Some(n.clone()),
                _ => None,
            };
            // Continuous mode (§4s): a class-ref into pad territory resolves
            // by the read site's EXPECTED class — CThumbnail's children are
            // CCamera then CDib, an entity preamble's pointer is the
            // attribute container, a container's below-base child is a named
            // dictionary — and the binding is permanent (+ recorded). Any
            // other unresolved ref stalls: no seeds, no depth-guessing — the
            // continuous walk follows the stream exactly or dies loudly.
            if self.continuous {
                match from_map {
                    Some(n) => n,
                    None => match (self.map.get(idx), expect) {
                        (Some(Slot::Pad), Some(e)) => {
                            self.map[idx] = Slot::Class(e.to_string());
                            self.bound.push((idx, e.to_string()));
                            e.to_string()
                        }
                        _ => {
                            self.pos = start;
                            let mut st = Stall::new(format!("<class-ref #{idx}>"), start, idx);
                            st.expected = expect; // calibration signal (§4s)
                            return Err(st);
                        }
                    },
                }
            } else {
                // Mid-stream archives see GLOBAL class-ref indexes their local
                // map can't hold. The seed covers classes declared BEFORE the
                // run (it only admits template-evidenced tags — see
                // detect_geometry_tags); classes declared by THIS stream are
                // matched by recursion depth, because list-element classes
                // declare at list level while nested helpers (CSkFont inside a
                // dimension body) declare deeper (Phase 2.2: the second
                // CDimensionLinear/CText arrives as a global class-ref right
                // after the first declared the class). Ambiguity stalls, loudly.
                let depth_match = || {
                    // Exact-depth candidates first; when none exist at this
                    // depth, a class that is unique across ALL declarations
                    // still resolves (cylinder.skp: CArcCurve declares inline
                    // at depth 2 inside the first edge, then the extrusion's
                    // second curve arrives as a depth-1 LIST ELEMENT ref).
                    let unique = |mut v: Vec<&String>| {
                        v.dedup();
                        (v.len() == 1).then(|| v[0].clone())
                    };
                    let same_depth: Vec<&String> = self
                        .classes
                        .iter()
                        .zip(self.class_depths.iter())
                        .filter(|(_, dep)| **dep == self.depth)
                        .map(|(n, _)| n)
                        .collect();
                    if same_depth.is_empty() {
                        unique(self.classes.iter().collect())
                    } else {
                        unique(same_depth)
                    }
                };
                let resolved = from_map
                    .or_else(|| self.seed_classes.get(&idx).cloned())
                    .or_else(depth_match);
                match resolved {
                    Some(n) => n,
                    None => {
                        return Err(Stall::new(format!("<class-ref #{idx}>"), start, idx));
                    }
                }
            }
        };

        // register the object BEFORE its body (cycle-safe: back-refs to it during
        // the body read resolve to this slot, matching MFC / the Python reference)
        let obj_index = self.map.len();
        self.map.push(Slot::Object(Entity::Other(name.clone())));

        match read_body(self, &name) {
            Ok(Some(entity)) => {
                self.map[obj_index] = Slot::Object(entity);
                Ok(ReadOutcome::Decoded(Child::Obj(obj_index)))
            }
            Ok(None) => {
                // No reader ran (unknown class, or Phase 0.2's schema
                // downgrade). If a SkipRule can delimit the body, consume it
                // and keep the placeholder slot — exactly one map slot, so
                // later back-refs stay aligned (Phase 0.3). Else rewind so
                // the caller sees the tag, and stall.
                let rule = self
                    .skip_rules
                    .get(&name)
                    .copied()
                    .or_else(|| crate::entity::default_skip_rule(&name));
                if let Some(rule) = rule {
                    let body_start = self.pos;
                    let n = match rule {
                        SkipRule::Fixed(n) => Some(n),
                        SkipRule::Dynamic(f) => f(self.d, body_start),
                    };
                    if let Some(n) = n {
                        if body_start + n <= self.d.len() {
                            self.pos = body_start + n;
                            self.skipped.push(SkipRecord {
                                class: name.clone(),
                                at: body_start,
                                bytes: n,
                            });
                            return Ok(ReadOutcome::Skipped {
                                child: Child::Obj(obj_index),
                                class: name,
                                bytes: n,
                            });
                        }
                    }
                }
                self.pos = start;
                Err(Stall::new(name, start, obj_index))
            }
            Err(stall) => Err(stall),
        }
    }
}

/// Decode UTF-16LE bytes, lossily (matches Python's `.decode('utf-16le')` for the
/// well-formed strings we encounter; replacement chars only on corrupt input).
pub fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Locate + measure an MFC UTF-16 string record at `off` (Phase 0.5).
///
/// Layout (public MFC `CArchive`/`CString` knowledge — `AfxReadStringLength`):
/// the byte length `0xFF` followed by the word `0xFEFF` is the Unicode
/// marker (on disk: `FF FE FF`), after which the CHARACTER count follows
/// with length escalation:
///   - `len:u8` if < 0xFF;
///   - else `0xFF` then `len:u16` if < 0xFFFF;
///   - else `FF FF` then `len:u32`.
///
/// (Class names in new-class records are a plain `u16` length — no
/// escalation exists for them in MFC; nothing to do there.)
///
/// Returns `(char_count, offset_of_first_char)`, or `None` if `off` does not
/// hold a well-formed record.
pub fn mfc_strlen(d: &[u8], off: usize) -> Option<(usize, usize)> {
    if off + 4 > d.len() || &d[off..off + 3] != b"\xff\xfe\xff" {
        return None;
    }
    let b = d[off + 3];
    if b < 0xFF {
        return Some((b as usize, off + 4));
    }
    if off + 6 > d.len() {
        return None;
    }
    let w = u16::from_le_bytes([d[off + 4], d[off + 5]]);
    if w < 0xFFFF {
        return Some((w as usize, off + 6));
    }
    if off + 10 > d.len() {
        return None;
    }
    let dw = u32::from_le_bytes([d[off + 6], d[off + 7], d[off + 8], d[off + 9]]);
    Some((dw as usize, off + 10))
}

#[cfg(test)]
mod strlen_tests {
    use super::*;

    /// Build a record: marker + escalated length + n×'a' (UTF-16LE).
    fn record(n: usize) -> Vec<u8> {
        let mut v: Vec<u8> = b"\xff\xfe\xff".to_vec();
        if n < 0xFF {
            v.push(n as u8);
        } else if n < 0xFFFF {
            v.push(0xFF);
            v.extend_from_slice(&(n as u16).to_le_bytes());
        } else {
            v.extend_from_slice(&[0xFF, 0xFF, 0xFF]);
            v.extend_from_slice(&(n as u32).to_le_bytes());
        }
        for _ in 0..n {
            v.extend_from_slice(&[b'a', 0]);
        }
        v
    }

    fn read(v: &[u8]) -> String {
        let mut ar = CArchive::new(v, 0);
        ar.utf16().expect("well-formed record")
    }

    #[test]
    fn short_record_reads_as_before() {
        let v = record(5);
        assert_eq!(mfc_strlen(&v, 0), Some((5, 4)));
        assert_eq!(read(&v), "aaaaa");
    }

    #[test]
    fn u16_escalated_record() {
        let v = record(300);
        assert_eq!(mfc_strlen(&v, 0), Some((300, 6)));
        let s = read(&v);
        assert_eq!(s.len(), 300);
    }

    #[test]
    fn u32_escalated_record() {
        let v = record(70_000);
        assert_eq!(mfc_strlen(&v, 0), Some((70_000, 10)));
        let s = read(&v);
        assert_eq!(s.len(), 70_000);
    }

    #[test]
    fn boundary_len_254_is_not_escalated() {
        let v = record(254);
        assert_eq!(mfc_strlen(&v, 0), Some((254, 4)));
        assert_eq!(read(&v).len(), 254);
    }

    #[test]
    fn truncated_and_bad_markers_are_none() {
        assert_eq!(mfc_strlen(b"\xff\xfe", 0), None);
        assert_eq!(mfc_strlen(b"\x00\x01\x02\x03", 0), None);
        assert_eq!(mfc_strlen(b"\xff\xfe\xff\xff\x2c", 0), None); // u16 cut off
    }
}

#[cfg(test)]
mod pid_width_tests {
    use super::*;
    use crate::entity::Entity;

    /// `FF FF <schema> <namelen> CVertex` + a body with an arbitrary §4i
    /// pid field: `00 00 <mask> <pid bytes>` + 3×f64.
    fn cvertex_stream(mask: u8, pid_bytes: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, 0xFF, 0, 0, 7, 0];
        v.extend_from_slice(b"CVertex");
        v.extend_from_slice(&[0, 0, mask]);
        v.extend_from_slice(pid_bytes);
        for c in [1.5f64, 2.5, 3.5] {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v
    }

    fn read_vertex(d: &[u8]) -> Result<(u32, f64, f64, f64), Stall> {
        let mut ar = CArchive::new(d, 0);
        let child = ar.read_object()?;
        match child {
            Child::Obj(idx) => match &ar.map[idx] {
                Slot::Object(Entity::Vertex { pid, x, y, z }) => Ok((*pid, *x, *y, *z)),
                _ => panic!("expected a Vertex in the map"),
            },
            other => panic!("expected Obj, got {other:?}"),
        }
    }

    #[test]
    fn mask_03_u16_pid_reads_as_before() {
        let d = cvertex_stream(0x03, &6099u16.to_le_bytes());
        assert_eq!(read_vertex(&d).unwrap(), (6099, 1.5, 2.5, 3.5));
    }

    #[test]
    fn mask_07_u24_pid_decodes() {
        // pid 75,385 — the SKP_FORMAT §4i evidence value (pid-stress @0x23656).
        let d = cvertex_stream(0x07, &[0x79, 0x26, 0x01]);
        assert_eq!(read_vertex(&d).unwrap(), (75_385, 1.5, 2.5, 3.5));
    }

    #[test]
    fn sparse_mask_05_omits_the_zero_middle_byte() {
        // pid 0x020001 = 131,073: byte1 is zero and omitted; mask 0b101.
        // pid-stress holds the complete 255-pid run 0x020001..=0x0200ff
        // in exactly this form.
        let d = cvertex_stream(0x05, &[0x01, 0x02]);
        assert_eq!(read_vertex(&d).unwrap(), (0x0002_0001, 1.5, 2.5, 3.5));
    }

    #[test]
    fn mask_0f_u32_pid_decodes_as_the_predicted_extension() {
        let d = cvertex_stream(0x0f, &0x0102_0304u32.to_le_bytes());
        assert_eq!(read_vertex(&d).unwrap(), (0x0102_0304, 1.5, 2.5, 3.5));
    }

    #[test]
    fn mask_past_u32_stalls_instead_of_misreading() {
        let d = cvertex_stream(0x1f, &[1, 2, 3, 4, 5]);
        let err = read_vertex(&d).unwrap_err();
        assert_eq!(err.class, "<bad pid preamble>");
    }
}

#[cfg(test)]
mod skip_tests {
    use super::*;

    /// `FF FF <schema> <namelen> <name> <body>` — one new-class record + its
    /// first object's body.
    fn new_class(schema: u16, name: &str, body: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, 0xFF];
        v.extend_from_slice(&schema.to_le_bytes());
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(name.as_bytes());
        v.extend_from_slice(body);
        v
    }

    /// A valid `CVertex` body: `00 00 03` + pid + 3×f64.
    fn cvertex_body(pid: u16, x: f64, y: f64, z: f64) -> Vec<u8> {
        let mut v = vec![0, 0, 3];
        v.extend_from_slice(&pid.to_le_bytes());
        for c in [x, y, z] {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v
    }

    #[test]
    fn fixed_skip_consumes_one_slot_and_the_walk_continues() {
        let mut d = new_class(1, "CFake", &[9u8; 7]);
        d.extend_from_slice(&new_class(0, "CVertex", &cvertex_body(7, 1.0, 2.0, 3.0)));

        let mut ar = CArchive::new(&d, 0);
        ar.skip_rules.insert("CFake".into(), SkipRule::Fixed(7));

        match ar.read_object_outcome().expect("skip, not stall") {
            ReadOutcome::Skipped { class, bytes, .. } => {
                assert_eq!(class, "CFake");
                assert_eq!(bytes, 7);
            }
            other => panic!("expected Skipped, got {other:?}"),
        }
        assert_eq!(ar.skipped.len(), 1);
        // Exactly one class slot + one object slot were consumed; the object
        // slot is the Entity::Other placeholder.
        assert!(matches!(&ar.map[2], Slot::Object(Entity::Other(n)) if n == "CFake"));

        // The next object decodes normally — the stream stayed in sync.
        match ar.read_object_outcome().expect("vertex decodes") {
            ReadOutcome::Decoded(Child::Obj(idx)) => match &ar.map[idx] {
                Slot::Object(Entity::Vertex { pid, x, y, z }) => {
                    assert_eq!((*pid, *x, *y, *z), (7, 1.0, 2.0, 3.0));
                }
                _ => panic!("expected a Vertex in the map"),
            },
            other => panic!("expected Decoded, got {other:?}"),
        }
    }

    #[test]
    fn without_a_rule_the_unknown_class_still_stalls() {
        let d = new_class(1, "CFake", &[9u8; 7]);
        let mut ar = CArchive::new(&d, 0);
        let err = ar.read_object_outcome().unwrap_err();
        assert_eq!(err.class, "CFake");
        assert_eq!(ar.pos, 0, "rewound so the caller sees the tag");
        assert!(ar.skipped.is_empty());
    }

    #[test]
    fn dynamic_rule_delimits_a_length_prefixed_body() {
        // CBlob body = u32 payload length + payload.
        let mut body = 5u32.to_le_bytes().to_vec();
        body.extend_from_slice(b"hello");
        let mut d = new_class(1, "CBlob", &body);
        d.extend_from_slice(&new_class(0, "CVertex", &cvertex_body(1, 4.0, 5.0, 6.0)));

        fn blob_len(d: &[u8], at: usize) -> Option<usize> {
            let n = u32::from_le_bytes([
                *d.get(at)?,
                *d.get(at + 1)?,
                *d.get(at + 2)?,
                *d.get(at + 3)?,
            ]) as usize;
            Some(4 + n)
        }

        let mut ar = CArchive::new(&d, 0);
        ar.skip_rules
            .insert("CBlob".into(), SkipRule::Dynamic(blob_len));

        match ar.read_object_outcome().expect("skip") {
            ReadOutcome::Skipped { bytes, .. } => assert_eq!(bytes, 9),
            other => panic!("expected Skipped, got {other:?}"),
        }
        assert!(matches!(
            ar.read_object_outcome().expect("vertex decodes"),
            ReadOutcome::Decoded(Child::Obj(_))
        ));
    }

    #[test]
    fn oversized_fixed_rule_stalls_instead_of_panicking() {
        let d = new_class(1, "CFake", &[9u8; 3]); // only 3 body bytes present
        let mut ar = CArchive::new(&d, 0);
        ar.skip_rules.insert("CFake".into(), SkipRule::Fixed(400));
        let err = ar.read_object_outcome().unwrap_err();
        assert_eq!(err.class, "CFake");
        assert!(ar.skipped.is_empty());
    }
}
