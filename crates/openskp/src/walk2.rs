//! The continuous single-archive walk of the model section (SKP_FORMAT §4s).
//!
//! **The entire `.skp` payload is ONE MFC `CArchive` from offset 0x50, with
//! one shared 1-based store map.** The legacy per-run fresh-map walk
//! ([`crate::walk`]) held on the minimal corpus only because each file's
//! single entity list sat near its own anchor; house.skp (47 definitions,
//! 58 groups) breaks it — its entity lists back-reference objects across
//! list boundaries. This walk starts a throwaway archive at the `CLayer`
//! new-class record, CALIBRATES the number of pre-model slots per file
//! (the composition of the pre-model region varies: box.skp's CLayer class
//! is slot 5, house.skp's is 31), then re-walks with the map pre-padded and
//! decodes the §4s model-section layout exactly:
//!
//! ```text
//! <count:u32> layer list          (count × CLayer)
//! <u16> <count:u32> definition list (count × CComponentDefinition;
//!                                  purged slots serialize as null tags)
//! CComponentDefinition ×N         trailing defs, back-to-back class-refs
//! <count:u32> ROOT entity list    (named groups, instances, loose geometry)
//! 58 79 f0 6a 00 …                the §4l root-record tail
//! ```
//!
//! Any structural surprise fails the whole attempt loudly (stage + offset)
//! and `Model::parse` falls back to the legacy path — the continuous walk
//! either follows the stream exactly or is not used at all.
//!
//! Port of `tools/contwalk.py` (frozen reference; zero desyncs on house.skp).

use crate::carchive::{CArchive, Child, Slot, Stall};
use crate::entity::Entity;

/// The `CLayer` new-class record: `FF FF <schema=2:u16> <namelen=6:u16>`
/// + the name — the model section's anchor (house.skp @0x1e4765).
const CLAYER_DECL: &[u8] = b"\xff\xff\x02\x00\x06\x00CLayer";

/// One decoded definition: its ACTUAL global map slot (what instance
/// def-refs carry — see `Entity::ComponentDef::decl_index` for the declared
/// one, which drifts past purged def-list slots) and its file span.
pub struct DefSpan {
    pub slot: usize,
    pub start: usize,
    pub end: usize,
}

/// A completed continuous walk: the ONE global store map plus the section
/// structure needed to build a [`crate::Model`].
pub struct Continuous {
    /// Calibrated pre-model slot count (house: 30; house-plus: 33).
    pub base: usize,
    pub map: Vec<Slot>,
    /// Layer-LIST object slots, list order (per-definition Layer0 copies
    /// are separate map objects and not listed here).
    pub layer_slots: Vec<usize>,
    /// Definitions in serialization order (def list + trailing).
    pub defs: Vec<DefSpan>,
    /// The root entity list's elements.
    pub roots: Vec<Child>,
    /// File offset of the root list's count u32.
    pub root_list_at: usize,
    /// First map slot belonging to the root list's objects.
    pub root_slot_floor: usize,
    /// Offset just past the last root element (the §4l tail follows).
    pub end: usize,
    /// Context-directed pad-slot bindings taken (slot, class) — §4s
    /// calibration evidence, surfaced as diagnostics.
    pub bound: Vec<(usize, String)>,
}

/// Why (and where) a continuous attempt died. `Model::parse` records it and
/// falls back to the legacy walk.
pub struct WalkFail {
    pub stage: &'static str,
    pub at: usize,
    pub detail: String,
}

impl WalkFail {
    fn from_stall(stage: &'static str, st: Stall) -> Self {
        WalkFail {
            stage,
            at: st.pos,
            detail: st.class,
        }
    }
}

/// Attempt the continuous walk. `Err` = the anchor/calibration failed or
/// the walk died before the root list completed.
pub fn walk(d: &[u8]) -> Result<Continuous, WalkFail> {
    let decl = find(d, CLAYER_DECL).ok_or(WalkFail {
        stage: "anchor",
        at: 0,
        detail: "no CLayer new-class record".into(),
    })?;
    if decl < 4 {
        return Err(WalkFail {
            stage: "anchor",
            at: decl,
            detail: "no room for the layer-list count".into(),
        });
    }
    let layer_count = u32le(d, decl - 4) as usize;
    if !(1..=4096).contains(&layer_count) {
        return Err(WalkFail {
            stage: "anchor",
            at: decl - 4,
            detail: format!("implausible layer count {layer_count}"),
        });
    }
    let ctx = crate::ctx::Ctx::of(d);

    let base = calibrate(d, decl, &ctx)?;
    // Each pre-model slot is a serialized object occupying at least one byte,
    // so a legitimate base cannot exceed the file length. Reject an
    // implausible base before pre-padding the map with it (a crafted
    // default-layer pointer could otherwise force a huge placeholder
    // allocation) — the legacy path serves the file instead.
    if base >= d.len() {
        return Err(WalkFail {
            stage: "calibration",
            at: decl,
            detail: format!("implausible pre-model base {base} (file is {} bytes)", d.len()),
        });
    }

    // ---- the real walk, map pre-padded to the calibrated base ----
    let mut ar = CArchive::new_continuous(d, decl, base);
    ar.ctx = ctx;

    let mut layer_slots = Vec::with_capacity(layer_count);
    for _ in 0..layer_count {
        match ar.read_object() {
            Ok(Child::Obj(i)) if matches!(&ar.map[i], Slot::Object(Entity::Layer { .. })) => {
                layer_slots.push(i);
            }
            Ok(_) => {
                return Err(WalkFail {
                    stage: "layer-list",
                    at: ar.pos,
                    detail: "non-CLayer element".into(),
                });
            }
            Err(st) => return Err(WalkFail::from_stall("layer-list", st)),
        }
    }

    // Definition list header: an OBJECT POINTER to the DEFAULT LAYER
    // (house `20 00` = slot 32, curve `11 00` = slot 17 — always the first
    // layer-list object, at base + 2) + u32 count. The pointer doubles as
    // a calibration cross-check: a wrong base cannot satisfy it, so the
    // walk dies HERE instead of desyncing into a body. Purged def slots
    // serialize as null tags (house also holds one stray object back-ref);
    // only real CComponentDefinition objects are collected.
    let at = ar.pos;
    let w = ar.u2().map_err(|s| WalkFail::from_stall("def-list", s))?;
    let layer_ptr = if w == 0x7FFF {
        // the MFC big-object escape (giant maps)
        ar.u4().map_err(|s| WalkFail::from_stall("def-list", s))? as usize
    } else {
        w as usize
    };
    if (w & 0x8000) != 0 || layer_slots.first() != Some(&layer_ptr) {
        return Err(WalkFail {
            stage: "def-list",
            at,
            detail: format!(
                "default-layer pointer {layer_ptr} != Layer0 slot {:?} (base miscalibrated?)",
                layer_slots.first()
            ),
        });
    }
    let ndef = ar.u4().map_err(|s| WalkFail::from_stall("def-list", s))? as usize;
    if ndef > 100_000 {
        return Err(WalkFail {
            stage: "def-list",
            at: ar.pos,
            detail: format!("implausible definition count {ndef}"),
        });
    }
    let mut defs: Vec<DefSpan> = Vec::with_capacity(ndef.min(4096));
    for _ in 0..ndef {
        let start = ar.pos;
        match ar.read_object() {
            Ok(Child::Obj(i))
                if matches!(&ar.map[i], Slot::Object(Entity::ComponentDef { .. })) =>
            {
                defs.push(DefSpan {
                    slot: i,
                    start,
                    end: ar.pos,
                });
            }
            Ok(_) => {} // purged slot (null tag / stray back-ref)
            Err(st) => return Err(WalkFail::from_stall("definitions", st)),
        }
    }

    // Trailing definitions, back-to-back while the next tag is a class-ref
    // resolving to the CComponentDefinition class (house: `23 80`; the last
    // ones are the model-root groups Group#2/Group#1). The prototype
    // hardcoded tag 0x8023 — resolving through the map generalizes it to
    // house-plus (0x8026).
    while let Some(w) = u16le(d, ar.pos) {
        if (w & 0x8000) == 0 || w == 0xFFFF {
            break;
        }
        let idx = (w & 0x7FFF) as usize;
        if !matches!(ar.map.get(idx), Some(Slot::Class(n)) if n == "CComponentDefinition") {
            break;
        }
        let start = ar.pos;
        match ar.read_object() {
            Ok(Child::Obj(i))
                if matches!(&ar.map[i], Slot::Object(Entity::ComponentDef { .. })) =>
            {
                defs.push(DefSpan {
                    slot: i,
                    start,
                    end: ar.pos,
                });
            }
            Ok(_) => {
                return Err(WalkFail {
                    stage: "trailing-defs",
                    at: start,
                    detail: "class-ref did not yield a definition".into(),
                });
            }
            Err(st) => return Err(WalkFail::from_stall("trailing-defs", st)),
        }
    }

    // The ROOT entity list — the model's own entities.
    let root_list_at = ar.pos;
    let rcount = ar.u4().map_err(|s| WalkFail::from_stall("root-list", s))? as usize;
    if rcount > 1_000_000 {
        return Err(WalkFail {
            stage: "root-list",
            at: root_list_at,
            detail: format!("implausible root count {rcount}"),
        });
    }
    let root_slot_floor = ar.map.len();
    let mut roots = Vec::with_capacity(rcount.min(4096));
    for _ in 0..rcount {
        match ar.read_object() {
            Ok(c) => roots.push(c),
            Err(st) => return Err(WalkFail::from_stall("root-list", st)),
        }
    }

    let bound = std::mem::take(&mut ar.bound);
    Ok(Continuous {
        base,
        map: std::mem::take(&mut ar.map),
        layer_slots,
        defs,
        roots,
        root_list_at,
        root_slot_floor,
        end: ar.pos,
        bound,
    })
}

/// Calibrate the pre-model slot count (§4s, revised twice):
///
/// - **Multi-layer files**: read the first layer with a throwaway
///   zero-pad archive; the 2nd list layer arrives as a class-ref to the
///   TRUE CLayer class slot (house `1f 80` = 31 → base 30).
/// - **Single-layer files**: the very next record is the definition-list
///   header, which OPENS with an object pointer to the DEFAULT LAYER —
///   always the slot right after the CLayer class, so
///   `base = pointer − 2` (curve `11 00` = 17 → base 15; house would be
///   `20 00` = 32 → 30, consistent).
///
/// Two earlier stall-driven heuristics are refuted and gone: calibrating
/// on the FIRST out-of-range class-ref (the frozen prototype) picks up the
/// attribute-container class in files whose first definition carries
/// attributes (old curve.skp → bogus base 4), and calibrating on the first
/// PLAIN-site stall picks up refs to classes declared INSIDE the throwaway
/// walk itself, whose true indexes are base-shifted — pin-*.skp/new
/// curve.skp's first such stall is the CVertex class at true slot 20 via
/// an edge's v1 (`14 80` @0x2fcd-era), overshooting base by exactly the
/// throwaway numbering delta (19 vs the true 15). The def-list layer
/// pointer is immune to both, and the main walk re-checks it after the
/// layer list, so a bad base dies loudly at the def-list header.
fn calibrate(d: &[u8], decl: usize, ctx: &Option<crate::ctx::Ctx>) -> Result<usize, WalkFail> {
    let mut ar = CArchive::new_continuous(d, decl, 0);
    ar.ctx = ctx.clone();
    if let Err(s) = ar.read_object() {
        return Err(WalkFail::from_stall("calibration", s));
    }
    let Some(w) = u16le(d, ar.pos) else {
        return Err(WalkFail {
            stage: "calibration",
            at: ar.pos,
            detail: "stream ends after the first layer".into(),
        });
    };
    // Multi-layer: 2nd layer's class-ref names the CLayer class slot.
    if (w & 0x8000) != 0 && w != 0xFFFF {
        return Ok((w & 0x7FFF) as usize - 1);
    }
    // Single-layer: the def-list header's default-layer object pointer.
    let ptr = if w == 0x7FFF {
        u16le(d, ar.pos + 2)
            .zip(u16le(d, ar.pos + 4))
            .map(|(lo, hi)| (hi as usize) << 16 | lo as usize)
            .unwrap_or(0)
    } else {
        w as usize
    };
    if (3..=1_000_000).contains(&ptr) {
        return Ok(ptr - 2);
    }
    Err(WalkFail {
        stage: "calibration",
        at: ar.pos,
        detail: format!("implausible default-layer pointer {ptr}"),
    })
}

/// Per-slot-range back-reference resolution audit, mirroring the prototype's
/// validator: edge endpoint refs must land on vertices, edge-use refs on
/// edges/loops — all against the GLOBAL map (a back-ref `r` IS `map[r]`).
/// Returns `(satisfied, constraints)`.
pub fn resolve_range(map: &[Slot], lo: usize, hi: usize) -> (usize, usize) {
    let is = |r: usize, want: &str| -> bool {
        matches!(map.get(r), Some(Slot::Object(e)) if e.class_name() == want)
    };
    let (mut sat, mut con) = (0usize, 0usize);
    for slot in &map[lo.min(map.len())..hi.min(map.len())] {
        match slot {
            Slot::Object(Entity::Edge { v0, v1, .. }) => {
                for c in [v0, v1] {
                    if let Child::Ref(r) = c {
                        con += 1;
                        sat += usize::from(is(*r, "CVertex"));
                    }
                }
            }
            Slot::Object(Entity::EdgeUse { edge, parent, .. }) => {
                if let Child::Ref(r) = edge {
                    con += 1;
                    sat += usize::from(is(*r, "CEdge"));
                }
                if let Child::Ref(r) = parent {
                    con += 1;
                    sat += usize::from(is(*r, "CLoop"));
                }
            }
            _ => {}
        }
    }
    (sat, con)
}

fn find(d: &[u8], needle: &[u8]) -> Option<usize> {
    d.windows(needle.len()).position(|w| w == needle)
}

fn u16le(d: &[u8], off: usize) -> Option<u16> {
    d.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32le(d: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([d[off], d[off + 1], d[off + 2], d[off + 3]])
}
