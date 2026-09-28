//! Parsed entity types and their MFC `Serialize()` body readers.
//!
//! Coverage: `CVersionMap`, `CVertex`, `CEdge`, `CFace`, `CLoop`, `CEdgeUse`,
//! `CArcCurve`. Classes without a reader make [`read_body`] return `Ok(None)`,
//! which the archive turns into a `Stall` (the resume point / worklist).
//!
//! Port of the `r_c*` readers in `tools/skpwalk.py`.

use crate::carchive::{CArchive, Child, SkipRule, Stall};

/// Default [`SkipRule`]s: classes we can DELIMIT but not decode (Phase 0.3
/// mechanism). Empty today — every currently-known class either has a real
/// reader or an unmeasured body. Phase 2's decode-or-skip campaign fills
/// this in per class, each entry justified by corpus evidence (a guessed
/// span would desync the shared map index, which is worse than a stall).
pub fn default_skip_rule(_name: &str) -> Option<SkipRule> {
    None
}

/// One decoded object. Child pointers are [`Child`] (null / object / unresolved
/// back-ref), resolved later by [`crate::resolve`].
///
/// The record fields (coordinates, flags, plane, transform params) are fully
/// decoded and retained here; surfacing them through the public API as concrete
/// mesh geometry (vertex positions + connectivity) is the next milestone. Until
/// then several fields are read by the walker/resolver but not yet re-exported.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Entity {
    Vertex {
        pid: u32,
        x: f64,
        y: f64,
        z: f64,
    },
    Edge {
        pid: u32,
        soft: bool,
        smooth: bool,
        hidden: bool,
        layer: u16,
        v0: Child,
        v1: Child,
        curve: Child,
    },
    Face {
        pid: u32,
        front_material: u16,
        back_material: u16,
        hidden: bool,
        layer: u16,
        /// Boxed: the map holds one slot per object of a multi-million
        /// object model, so the inline variant size is what bounds memory.
        plane: Box<[f64; 4]>,
        loops: Vec<Child>,
        /// The preamble's attribute-container pointer (§4s) — on textured
        /// faces an inline `CAttributeContainer` holding the face's
        /// `CFaceTextureCoords` (null on the legacy path's fixed read).
        attrs: Child,
    },
    Loop {
        edge_uses: Vec<Child>,
    },
    EdgeUse {
        edge: Child,
        dir: u8,
        parent: Child,
    },
    ArcCurve {
        pid: u32,
        params: Box<[f64; 14]>,
    },
    /// A linear dimension (SKP_FORMAT §10.1).
    Dimension {
        pid: u32,
        /// Boxed: the payload is far larger than a map slot (see `Ftc`).
        body: Box<DimensionBody>,
    },
    /// A text annotation — leader or screen text (SKP_FORMAT §10.2). The
    /// fields after `content` are decoded only when the middle has the
    /// documented length; otherwise they stay at their defaults.
    Text {
        pid: u32,
        /// Boxed: the payload is far larger than a map slot (see `Ftc`).
        body: Box<TextBody>,
    },
    /// A font object (inline in dimension/text bodies).
    Font {
        name: String,
        bold: bool,
        italic: bool,
        size_pt: u32,
    },
    /// An in-list component/group instance (Phase 3.3, SKP_FORMAT §4o):
    /// the def-ref is the referenced definition's declared map index (on the
    /// continuous path: its GLOBAL map slot, §4s). §4o REVISION (§4s
    /// addendum): the def-ref is an MFC OBJECT POINTER read through the tag
    /// protocol — byte-identical to the old raw u16 for slots < 0x8000, but
    /// big maps (theater-2017) escalate via the `7F FF` big-tag escape, so
    /// the index is u32 here. `CGroup` shares this layout byte-for-byte
    /// (§4s: a group is an instance of an anonymous definition) —
    /// `is_group` keeps the class identity.
    InstancePlaced {
        pid: u32,
        defref: u32,
        transform: Box<[f64; 13]>,
        name: String,
        /// File offset of the 13-f64 transform block (top-level detection).
        tf_at: usize,
        hidden: bool,
        layer: u16,
        /// Drawbase matref (§4q, same slot as a face's): a material painted
        /// ON the instance, inherited by default-material faces below it
        /// (house: the walls' texture lives here, not on the faces).
        material: u16,
        is_group: bool,
    },
    /// A section plane (exact extent — SKP_FORMAT §4l, §10.3).
    SectionPlane {
        pid: u32,
        plane: [f64; 4],
        hidden: bool,
    },
    /// A tape-measure guide point (Phase 2.2, typed-dimension-proven).
    ConstructionPoint {
        pid: u32,
        point_in: [f64; 3],
    },
    /// A tape-measure guide LINE — schema 1 (guide.skp root list, §4l
    /// single-instance rules): preamble + drawbase + point (inches) +
    /// unit direction + line-parameter bounds (±1e30 = infinite) + 7B
    /// tail (zeros observed, TBD). Extent pinned by the §4l root tail
    /// resuming byte-exactly.
    ConstructionLine {
        pid: u32,
        /// point (3), unit direction (3), line-parameter bounds (2); boxed.
        params: Box<[f64; 8]>,
    },
    /// A placed image entity (exact extent — SKP_FORMAT §4l, §10.4).
    Image {
        pid: u32,
    },
    VersionMap {
        entries: Vec<(String, u32)>,
    },
    /// A layer record — schema 2 (SKP_FORMAT §4s; first house decl @0x1e4765):
    /// name, hidden flag, internal `Layer_<name>` string, display RGBA.
    Layer {
        pid: u32,
        name: String,
        hidden: bool,
        rgba: [u8; 4],
    },
    /// A component/group definition — schema 10 (SKP_FORMAT §4s). `decl_index`
    /// is the §4h-prelude-declared GLOBAL map index (`decl − 1`); the ACTUAL
    /// slot the object landed on is what instances' def-refs carry (house's
    /// "Birch Plywood" declares 2731 but sits at 2728 — the two purged
    /// def-list slots shifted the writer's numbering, and the read-side
    /// index is the one the def-refs match).
    ComponentDef {
        pid: u32,
        /// Name, description, GUID and timestamp, boxed (see `Face::plane`).
        meta: Box<DefMeta>,
        /// Declared global map index (§4h prelude `decl − 1`; 0 when the
        /// prelude declared nothing).
        decl_index: usize,
        /// Declared entity-list element count.
        count: usize,
        entities: Vec<Child>,
    },
    /// A definition preview — schema 1 (§4s): camera + nullable image.
    Thumbnail {
        camera: Child,
        image: Child,
    },
    /// A camera — schema 5 (§4s): NO preamble; 137B raw + u16 + desc + 33B.
    Camera {
        desc: String,
    },
    /// An embedded image — schema 3 (§4f/§4s): subtype (1=JPEG, 4=PNG) +
    /// length + payload (payload offset kept, bytes stay in the file slice).
    Dib {
        subtype: u32,
        at: usize,
        len: usize,
    },
    /// An attribute container — schema 0 (§4s): children until a null tag.
    AttrContainer {
        children: Vec<Child>,
    },
    /// A named attribute dictionary — schema 1 (§4s): typed key/values.
    AttrNamed {
        name: String,
        entries: usize,
    },
    /// Per-face texture placement — schema 4, SOLVED (§4u layout, §4v
    /// semantics): two row-major 3×3 PROJECTIVE matrices mapping texture
    /// space (inches) to the face-local frame as `[t_u, t_v, 1] · K`
    /// (row-vector convention), each followed by 3 TBD f64s (zero on user
    /// faces), then the front/back pin lists and two flag u32s.
    FaceTextureCoords {
        pid: u32,
        ftc: Box<Ftc>,
    },
    /// A welded/freehand curve — schema 4 (§4s addendum: theater-2017
    /// @0x56df8d; every default-template figure carries them, curve.skp
    /// decl @0x80b1): preamble + u8 + u32 member-edge count. NO owned
    /// children — member edges serialize as ordinary list/loop elements
    /// and each back-refs the curve via its curve pointer (the same
    /// association direction as CArcCurve).
    Curve {
        pid: u32,
        members: u32,
    },
    /// A definition-tail relationship record — schema 0 (§4s addendum,
    /// REVISED again on a third-party apartment model; see
    /// [`r_crelationship`]): two object pointers plus a nullable pointer to
    /// the NEXT relationship. Relationships form a singly linked chain
    /// hanging off the definition tail; the provenance block (GUID + name +
    /// description + source path + timestamp) that earlier readings placed
    /// inside this record belongs to the DEFINITION tail and follows the
    /// chain head pointer unconditionally.
    Relationship {
        pid: u32,
        a: Child,
        b: Child,
        next: Child,
    },
    /// A material-manager record — schema 12 (§4n, archive-walked; see
    /// [`r_cmaterial`]): name + the texture image pointer (null on solid
    /// materials; an inline or shared `CDib` object on textured ones).
    Material {
        pid: u32,
        name: String,
        dib: Child,
    },
    /// Placeholder registered before a body is read, and the value left for a
    /// class we can decode a tag for but have no body reader (a stall point).
    Other(String),
}

/// A definition's provenance block (§7.1).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DefMeta {
    pub name: String,
    pub desc: String,
    pub guid: String,
    pub timestamp: u32,
    /// Component behaviour (§7.1), decoded from the bytes after the
    /// timestamp.
    pub behaviour: crate::settings::Behaviour,
}

/// A linear dimension's payload (§10.1), boxed behind
/// [`Entity::Dimension`].
#[derive(Debug, Clone)]
pub struct DimensionBody {
    pub text_override: String,
    pub font: Child,
    /// `(kind, point in inches, entity)` for the start and end anchors.
    pub anchors: [(u32, [f64; 3], Child); 2],
    pub normal: [f64; 3],
    pub x_axis: [f64; 3],
    /// Inches.
    pub offset: f64,
    pub text_position: u32,
    pub aligned: bool,
}

/// A text annotation's payload (§10.2), boxed behind [`Entity::Text`].
#[derive(Debug, Clone)]
pub struct TextBody {
    pub content: String,
    pub font: Child,
    pub screen_position: [f64; 2],
    pub anchor_kind: u32,
    /// Inches.
    pub anchor_point: [f64; 3],
    pub anchor_entity: Child,
    /// Inches for a pushpin leader, screen space otherwise.
    pub leader_offset: [f64; 3],
    pub leader: u32,
    pub arrow: u32,
}

/// The per-face texture placement payload (§9), boxed behind
/// [`Entity::FaceTextureCoords`].
#[derive(Debug, Clone)]
pub struct Ftc {
    /// Front map k0..k8 (row-major 3×3, texture-in → face-local-in).
    pub front: [f64; 9],
    /// k9..k11 — zero on every authored user face; TBD (§4v).
    pub front_extra: [f64; 3],
    /// Back map k12..k20.
    pub back: [f64; 9],
    /// k21..k23 — TBD (the template figure carries (0,−1,0) here).
    pub back_extra: [f64; 3],
    /// §4u pins `(anchor_u, anchor_v, face_x, face_y)`, all inches in
    /// the §4v frame; `[anchor,1]·K` reproduces `face` exactly.
    pub front_pins: Vec<[f64; 4]>,
    pub back_pins: Vec<[f64; 4]>,
    /// Observed (1,0) affine-era / (0,1) pin-era / (0,3) figure — TBD.
    pub flags: [u32; 2],
}

impl Entity {
    /// The persistent id of an entity that has one.
    pub fn pid(&self) -> Option<u32> {
        match self {
            Entity::Vertex { pid, .. } => Some(*pid),
            Entity::Edge { pid, .. } => Some(*pid),
            Entity::Face { pid, .. } => Some(*pid),
            Entity::ArcCurve { pid, .. } => Some(*pid),
            Entity::Dimension { pid, .. } => Some(*pid),
            Entity::Text { pid, .. } => Some(*pid),
            Entity::InstancePlaced { pid, .. } => Some(*pid),
            Entity::SectionPlane { pid, .. } => Some(*pid),
            Entity::ConstructionPoint { pid, .. } => Some(*pid),
            Entity::ConstructionLine { pid, .. } => Some(*pid),
            Entity::Image { pid, .. } => Some(*pid),
            Entity::Layer { pid, .. } => Some(*pid),
            Entity::ComponentDef { pid, .. } => Some(*pid),
            Entity::FaceTextureCoords { pid, .. } => Some(*pid),
            Entity::Curve { pid, .. } => Some(*pid),
            Entity::Relationship { pid, .. } => Some(*pid),
            _ => None,
        }
    }
    /// The MFC class name of this object (used by counting + resolution).
    pub fn class_name(&self) -> &str {
        match self {
            Entity::Vertex { .. } => "CVertex",
            Entity::Edge { .. } => "CEdge",
            Entity::Face { .. } => "CFace",
            Entity::Loop { .. } => "CLoop",
            Entity::EdgeUse { .. } => "CEdgeUse",
            Entity::ArcCurve { .. } => "CArcCurve",
            Entity::Dimension { .. } => "CDimensionLinear",
            Entity::Text { .. } => "CText",
            Entity::Font { .. } => "CSkFont",
            Entity::InstancePlaced { is_group, .. } => {
                if *is_group {
                    "CGroup"
                } else {
                    "CComponentInstance"
                }
            }
            Entity::SectionPlane { .. } => "CSectionPlane",
            Entity::ConstructionPoint { .. } => "CConstructionPoint",
            Entity::ConstructionLine { .. } => "CConstructionLine",
            Entity::Image { .. } => "CImage",
            Entity::VersionMap { .. } => "CVersionMap",
            Entity::Layer { .. } => "CLayer",
            Entity::ComponentDef { .. } => "CComponentDefinition",
            Entity::Thumbnail { .. } => "CThumbnail",
            Entity::Camera { .. } => "CCamera",
            Entity::Dib { .. } => "CDib",
            Entity::AttrContainer { .. } => "CAttributeContainer",
            Entity::AttrNamed { .. } => "CAttributeNamed",
            Entity::FaceTextureCoords { .. } => "CFaceTextureCoords",
            Entity::Curve { .. } => "CCurve",
            Entity::Relationship { .. } => "CRelationship",
            Entity::Material { .. } => "CMaterial",
            Entity::Other(name) => name,
        }
    }
}

/// `CEntity` preamble, REINTERPRETED (SKP_FORMAT §4s, superseding §4i's
/// "`00 00` lead bytes"): a NULLABLE OBJECT POINTER (the entity's attribute
/// container — the null tag `00 00` for plain entities, which is why the
/// old fixed-3-byte read held on the minimal corpus) + the pid
/// presence-mask field. The mask is a per-byte presence bitmask: bit k set
/// = pid byte k stored (low-to-high), bit k clear = that byte is zero and
/// omitted. Any mask ≤ 0x0f (pid fits u32) decodes unambiguously here
/// because class dispatch already confirmed we are inside a real body; a
/// mask past 0x0f means a desynced stream: rewind and stall.
///
/// On the CONTINUOUS path the pointer is read as a real object (textured
/// faces carry `05 80` — an inline `CAttributeContainer` holding the
/// face's `CFaceTextureCoords`; §4m's "class-ref #107 per-entity
/// structure" was the same pointer non-null). On the legacy run-based
/// path the old fixed `00 00 <mask>` read is kept byte-for-byte so the
/// long-pinned resync histograms (attributes.skp) stay stable.
fn entity_preamble(ar: &mut CArchive) -> Result<u32, Stall> {
    entity_preamble_with_attrs(ar).map(|(pid, _)| pid)
}

/// [`entity_preamble`] keeping the attribute-container pointer — CFace needs
/// it to reach its `CFaceTextureCoords` (§4v).
fn entity_preamble_with_attrs(ar: &mut CArchive) -> Result<(u32, Child), Stall> {
    let start = ar.pos;
    let attrs = if ar.continuous {
        ar.read_object_expect("CAttributeContainer")?
    } else {
        ar.take(2)?; // legacy: the two lead bytes (a null pointer in truth)
        Child::Null
    };
    let pid = match pid_field(ar) {
        Ok(pid) => pid,
        Err(mut st) => {
            // Rewind to the record start so the caller sees the whole body.
            st.pos = start;
            ar.pos = start;
            return Err(st);
        }
    };
    Ok((pid, attrs))
}

/// One §4i presence-masked pid field: `<mask:u8>` + popcount(mask) bytes,
/// ascending significance. A mask past 0x0f (pid wider than u32) means a
/// desynced stream: stall.
fn pid_field(ar: &mut CArchive) -> Result<u32, Stall> {
    let at = ar.pos;
    let mask = ar.u1()?;
    if mask > 0x0f {
        let mapindex = ar.map.len();
        ar.pos = at;
        return Err(Stall::new("<bad pid preamble>", at, mapindex));
    }
    let raw = ar.take(mask.count_ones() as usize)?;
    let mut pid = 0u32;
    let mut i = 0;
    for k in 0..4 {
        if mask & (1 << k) != 0 {
            pid |= u32::from(raw[i]) << (8 * k);
            i += 1;
        }
    }
    Ok(pid)
}

/// A class body reader.
type Reader = fn(&mut CArchive) -> Result<Entity, Stall>;

/// Reader registry keyed by `(class, accepted schema range)` — Phase 0.2.
///
/// The ranges are the schemas OBSERVED + VALIDATED across the 2013–2017
/// corpus (`CVersionMap` declares them; see the file-by-file harvest in the
/// plan's 0.2 notes): CVertex=0, CEdge=2, CEdgeUse=1, CFace=3, CLoop=1 are
/// stable 2013→2017. `CArcCurve` is declared 2 in 2013/14 files but our
/// reader is validated ONLY on schema 3 (2015+; the arc/circle corpus is
/// 2017) — so its range stays `3..=3` and a 2013 arc degrades to skip until
/// an arc-v2013 corpus file validates (or refutes) layout equality.
/// `CVersionMap` is not listed in its own map: full range, structural reader.
const REGISTRY: &[(&str, std::ops::RangeInclusive<u32>, Reader)] = &[
    ("CVersionMap", 0..=u32::MAX, r_cversionmap),
    ("CVertex", 0..=0, r_cvertex),
    ("CEdge", 2..=2, r_cedge),
    ("CFace", 3..=3, r_cface),
    ("CLoop", 1..=1, r_cloop),
    ("CEdgeUse", 1..=1, r_cedgeuse),
    ("CArcCurve", 3..=3, r_carccurve),
    ("CDimensionLinear", 6..=6, r_cdimensionlinear),
    ("CText", 9..=9, r_ctext),
    ("CSkFont", 1..=1, r_cskfont),
    ("CComponentInstance", 5..=5, r_ccomponentinstance),
    ("CSectionPlane", 2..=2, r_csectionplane),
    ("CConstructionPoint", 0..=0, r_cconstructionpoint),
    ("CImage", 1..=1, r_cimage),
];

/// Continuous-walk reader registry (SKP_FORMAT §4s) — consulted FIRST when
/// the archive is in continuous mode, and never on the legacy run-based
/// path (whose long-pinned stall/resync behavior must stay bit-identical).
/// Schema ranges are the house.skp/house-plus.skp declarations the readers
/// were decoded against.
const CONT_REGISTRY: &[(&str, std::ops::RangeInclusive<u32>, Reader)] = &[
    ("CLayer", 2..=2, r_clayer),
    ("CComponentDefinition", 10..=10, r_ccomponentdefinition),
    ("CThumbnail", 1..=1, r_cthumbnail),
    ("CCamera", 5..=5, r_ccamera),
    ("CDib", 3..=3, r_cdib),
    ("CGroup", 1..=1, r_cgroup),
    ("CAttributeContainer", 0..=0, r_cattributecontainer),
    ("CAttributeNamed", 1..=1, r_cattributenamed),
    ("CFaceTextureCoords", 4..=4, r_cfacetexturecoords),
    // Continuous-only on purpose: the LEGACY path's pinned stall/resync
    // histograms (attributes.skp counts one CCurve resync) must not move.
    ("CCurve", 4..=4, r_ccurve),
    ("CRelationship", 0..=0, r_crelationship),
    ("CConstructionLine", 1..=1, r_cconstructionline),
    ("CMaterial", 12..=12, r_cmaterial),
];

/// Dispatch a class body reader. `Ok(None)` = no reader will run: either the
/// class is unknown, or it is known but the FILE declares a schema outside
/// the validated range (Phase 0.2's downgrade — a mismatched reader must
/// never run over a changed layout; the archive turns `None` into a
/// `Stall`, which the walk resyncs past, and Phase 0.3 will make an explicit
/// `Skipped` tier).
///
/// Schema source, in authority order: the stream's own new-class declaration
/// (`seen_schemas`), else the file-wide `CVersionMap` via `ctx`. A seeded
/// sub-walk with neither proceeds unchecked, matching pre-registry behavior.
pub fn read_body(ar: &mut CArchive, name: &str) -> Result<Option<Entity>, Stall> {
    let cont_hit = ar
        .continuous
        .then(|| CONT_REGISTRY.iter().find(|(n, _, _)| *n == name))
        .flatten();
    let Some((_, range, reader)) =
        cont_hit.or_else(|| REGISTRY.iter().find(|(n, _, _)| *n == name))
    else {
        return Ok(None);
    };
    let declared = ar
        .seen_schemas
        .get(name)
        .map(|s| *s as u32)
        .or_else(|| ar.ctx.as_ref().and_then(|c| c.schema(name)));
    if let Some(s) = declared {
        if !range.contains(&s) {
            return Ok(None);
        }
    }
    Ok(Some(reader(ar)?))
}

/// list of (classname, schema) until the 'End-Of-Version-Map' sentinel.
fn r_cversionmap(ar: &mut CArchive) -> Result<Entity, Stall> {
    let mut entries = Vec::new();
    loop {
        let name = ar.utf16()?;
        let schema = ar.u4()?;
        if name == "End-Of-Version-Map" {
            break;
        }
        entries.push((name, schema));
    }
    Ok(Entity::VersionMap { entries })
}

fn r_cvertex(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    let x = ar.f8()?;
    let y = ar.f8()?;
    let z = ar.f8()?;
    Ok(Entity::Vertex { pid, x, y, z })
}

fn r_cedge(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    let drawbase = ar.take(10)?; // CDrawingElement base (SKP_FORMAT §4q)
    let soft = drawbase[5] != 0;
    let smooth = drawbase[6] != 0;
    let hidden = drawbase[2] != 0;
    let layer = u16::from_le_bytes([drawbase[8], drawbase[9]]);
    let v0 = ar.read_object()?; // endpoint: inline CVertex or back-ref
    let v1 = ar.read_object()?;
    let curve = ar.read_object()?; // associated CCurve (null for a straight edge)
    Ok(Entity::Edge {
        pid,
        soft,
        smooth,
        hidden,
        layer,
        v0,
        v1,
        curve,
    })
}

fn r_cedgeuse(ar: &mut CArchive) -> Result<Entity, Stall> {
    ar.take(3)?; // 00 00 00
    let edge = ar.read_object()?; // CEdge (back-ref)
    let dir = ar.u1()?; // loop traversal sense
    let parent = ar.read_object()?; // owning CLoop (back-ref)
    Ok(Entity::EdgeUse { edge, dir, parent })
}

fn r_cloop(ar: &mut CArchive) -> Result<Entity, Stall> {
    ar.take(5)?; // loop base (00 00 00 01 01)
    let mut edge_uses = Vec::new();
    loop {
        let eu = ar.read_object()?; // null-terminated CEdgeUse list
        if eu == Child::Null {
            break;
        }
        edge_uses.push(eu);
    }
    Ok(Entity::Loop { edge_uses })
}

fn r_cface(ar: &mut CArchive) -> Result<Entity, Stall> {
    let (pid, attrs) = entity_preamble_with_attrs(ar)?;
    let drawbase = ar.take(10)?; // SKP_FORMAT §4q
    let front_material = u16::from_le_bytes([drawbase[0], drawbase[1]]); // 0 = none
    let hidden = drawbase[2] != 0;
    let layer = u16::from_le_bytes([drawbase[8], drawbase[9]]);
    let mut plane = [0.0f64; 4];
    for slot in plane.iter_mut() {
        *slot = ar.f8()?; // A,B,C,D (unit normal + offset)
    }
    let count = ar.u4()? as usize;
    // The loop count is an untrusted u32: pre-reserving `count` slots would
    // let a crafted face (count 0xFFFFFFFF) demand a multi-gigabyte
    // allocation before a single loop is read. Cap the reservation (a real
    // face has a handful of loops); the loop still reads `count` elements and
    // stalls naturally at EOF on a truncated/hostile stream.
    let mut loops = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        loops.push(ar.read_object()?);
    }
    let back_material = ar.u2()?; // back-face material u16 (0 = none)
    Ok(Entity::Face {
        pid,
        front_material,
        back_material,
        hidden,
        layer,
        plane: Box::new(plane),
        loops,
        attrs,
    })
}

/// `CSkFont` schema 1 (SKP_FORMAT §4k, extent corrected): §4i preamble
/// (pid-less on every authored file — `00 00 00` — but a third-party
/// house's dimension fonts carry pids: `00 00 03 <pid>`) + utf16 name +
/// 15-byte fixed tail (u16 + u32 height + f64 size + u8). §4k's original
/// 14 was one short: with 15, the dimension tail is a uniform 165 whether
/// the font is inline or a back-ref — all four dims across
/// dimension.skp/house-plus/mixed-definition share a byte-identical tail
/// prefix (`00 01 00 00 00 02 00 00 00 04 …`) under this alignment, and
/// the back-ref-font dims measure exactly 165 to the next record tag.
/// (CText sites never noticed: their reader re-anchors by scanning for the
/// pre-string signature.)
fn r_cskfont(ar: &mut CArchive) -> Result<Entity, Stall> {
    entity_preamble(ar)?;
    let name = ar.utf16()?;
    let t = ar.take(15)?; // bold, italic, u32 size, u8, f64 (§10.1)
    Ok(Entity::Font {
        name,
        bold: t[0] != 0,
        italic: t[1] != 0,
        size_pt: u32::from_le_bytes([t[2], t[3], t[4], t[5]]),
    })
}

/// `CDimensionLinear` schema 6 (SKP_FORMAT §4k, tail DECODED on
/// guest-house, byte-identical to the old fixed-165 reading on the
/// corpus): preamble + 10-byte drawbase (matref binds the dimension's
/// material) + text-override string + font object (inline or back-ref) +
/// u32 flags + TWO ANCHOR BLOCKS (`u32(2) u32(4) 3×f64 point +
/// entity OBJECT POINTER + u16 + u32 ref-count + count × object pointers +
/// u32(0)`) + 6×f64 2D basis + u32 + 2×f64 (offset, reserved) + u32.
/// The pointers are back-ref words on the corpus (no map slots) but
/// escalate through `7F FF` + u32 on giant maps (guest-house dims
/// @0x3671f3b carry 3-ref anchor lists with escalated pointers), which is
/// why the fixed-length reading desynced there.
fn r_cdimensionlinear(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    ar.take(10)?; // drawbase (matref u16 + flags)
    let text_override = ar.utf16()?;
    let font = ar.read_object()?;
    let aligned = ar.take(1)?[0] != 0; // u8 after the font object (present for
                                       // inline AND back-ref fonts — guest-house
                                       // dim #2 pins it outside the CSkFont record)
    ar.take(4)?; // u32 arrow
    let mut anchors = [(0u32, [0.0; 3], Child::Null), (0u32, [0.0; 3], Child::Null)];
    for a in anchors.iter_mut() {
        let kind = ar.u4()?;
        ar.take(4)?; // u32(4)
        let point = [ar.f8()?, ar.f8()?, ar.f8()?];
        let entity = ar.read_object()?; // anchor entity pointer
        ar.take(2)?; // u16
        let n = ar.u4()? as usize;
        if n > 64 {
            return Err(Stall::new("<dim anchor refs>", ar.pos, ar.map.len()));
        }
        for _ in 0..n {
            ar.read_object()?; // instance path
        }
        ar.take(4)?; // trailing u32 (0)
        *a = (kind, point, entity);
    }
    let normal = [ar.f8()?, ar.f8()?, ar.f8()?];
    let x_axis = [ar.f8()?, ar.f8()?, ar.f8()?];
    ar.take(4)?; // u32 (1 or 2)
    let offset = ar.f8()?;
    ar.take(8)?; // f64 0
    let text_position = ar.u4()?;
    Ok(Entity::Dimension {
        pid,
        body: Box::new(DimensionBody {
            text_override,
            font,
            anchors,
            normal,
            x_axis,
            offset,
            text_position,
            aligned,
        }),
    })
}

/// `CText` schema 9 (SKP_FORMAT §4k): preamble + 10-byte drawbase + font
/// object + a VARIANT-length middle (leader text carries anchor coords, a
/// leader vector and an anchor back-ref word; screen text carries screen
/// fractions — text.skp holds one of each), then the content string and a
/// 5-byte tail. Both variants end the middle with the same 11-byte block
/// immediately before the string marker — the reader scans (bounded) for
/// that signature, so consumption is exact without decoding the variant
/// fields; a miss stalls loudly rather than guessing.
fn r_ctext(ar: &mut CArchive) -> Result<Entity, Stall> {
    const PRE_STRING: [u8; 11] = [1, 0, 0, 0, 1, 0, 3, 0, 0, 0, 1];
    let pid = entity_preamble(ar)?;
    ar.take(10)?; // drawbase
    let font = ar.read_object()?;
    let start = ar.pos;
    let end = ar.d.len().min(start + 512).saturating_sub(14);
    let Some(p) = (start..end)
        .find(|&p| ar.d[p..p + 11] == PRE_STRING && ar.d[p + 11..p + 14] == [0xff, 0xfe, 0xff])
    else {
        let mapindex = ar.map.len();
        ar.pos = start;
        return Err(Stall::new("<ctext middle unbounded>", start, mapindex));
    };
    // The middle (§10.2): 2 f64 screen position, anchor (u32 kind, u32 4,
    // 3 f64 point, entity reference, u16, u32 n + n references, u32), 3 f64
    // leader offset, 3 f64 view direction, u32 leader, u32, u8, u8, u32
    // arrow, u8 — decoded by walking it, and kept only when the walk
    // lands exactly on the string marker.
    ar.pos = start;
    let mut mid = None;
    let walk = |ar: &mut CArchive| -> Result<_, Stall> {
        let screen = [ar.f8()?, ar.f8()?];
        let kind = ar.u4()?;
        ar.take(4)?;
        let point = [ar.f8()?, ar.f8()?, ar.f8()?];
        let entity = ar.read_object()?;
        ar.take(2)?;
        let n = ar.u4()? as usize;
        if n > 64 {
            return Err(Stall::new("<text anchor refs>", ar.pos, ar.map.len()));
        }
        for _ in 0..n {
            ar.read_object()?;
        }
        ar.take(4)?;
        let offset = [ar.f8()?, ar.f8()?, ar.f8()?];
        ar.take(24)?; // view direction
        let leader = ar.u4()?;
        ar.take(6)?; // u32, u8, u8
        let arrow = ar.u4()?;
        ar.take(1)?;
        Ok((screen, kind, point, entity, offset, leader, arrow))
    };
    let saved = ar.map.len();
    if let Ok(m) = walk(ar) {
        if ar.pos == p + 11 {
            mid = Some(m);
        }
    }
    if mid.is_none() {
        ar.map.truncate(saved);
    }
    ar.pos = p + 11;
    let content = ar.utf16()?;
    ar.take(5)?;
    let (screen_position, anchor_kind, anchor_point, anchor_entity, leader_offset, leader, arrow) =
        mid.unwrap_or(([0.0; 2], 0, [0.0; 3], Child::Null, [0.0; 3], 0, 0));
    Ok(Entity::Text {
        pid,
        body: Box::new(TextBody {
            content,
            font,
            screen_position,
            anchor_kind,
            anchor_point,
            anchor_entity,
            leader_offset,
            leader,
            arrow,
        }),
    })
}

/// `CSectionPlane` schema 2 (SKP_FORMAT §4l): preamble + drawbase + plane
/// 4×f64 — nothing else. section-plane.skp's plane decodes to
/// (0, 0, -1, 19.685" = 0.5 m), the authored mid-box horizontal cut; its
/// "trailing u32" was the root tail's zero run behind the list's last
/// element. A third-party house places section planes mid-definition with
/// the definition tail hard behind them, pinning the extent.
fn r_csectionplane(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    let drawbase = ar.take(10)?; // §4q
    let hidden = drawbase[2] != 0;
    let mut plane = [0.0f64; 4];
    for slot in plane.iter_mut() {
        *slot = ar.f8()?;
    }
    Ok(Entity::SectionPlane { pid, plane, hidden })
}

/// `CConstructionPoint` schema 0 (SKP_FORMAT §4l): preamble + drawbase +
/// position 3×f64 + reference point 3×f64 + ONE tail byte. The corpus
/// instance's position is EXACTLY the authored (1 m, 2 m, 3 m) —
/// typed-dimension proof of the layout. The tail was read as a u32 (1)
/// while the only instance sat last in a root list, ahead of the §4l zero
/// run; a third-party bathroom model places guide points mid-definition
/// with the next entity's class-ref hard behind them, pinning the tail to
/// exactly one byte (0 there, 1 in the corpus).
fn r_cconstructionpoint(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    ar.take(10)?; // drawbase
    let mut point_in = [0.0f64; 3];
    for slot in point_in.iter_mut() {
        *slot = ar.f8()?;
    }
    ar.take(24)?; // reference point 3×f64 (the tape-measure anchor)
    ar.take(1)?; // u8 flag (1 in the corpus instance, 0 mid-definition)
    Ok(Entity::ConstructionPoint { pid, point_in })
}

/// `CImage` schema 1 (SKP_FORMAT §4l, revised on guest-house): preamble,
/// drawbase(10), a LAYER OBJECT POINTER (a plain back-ref word in the
/// corpus instance — `17 00`; the `7F FF` + u32 big-object escape on giant
/// maps — guest-house @0x1d18aff), a 13×f64 placement block
/// (inch-per-pixel scale plus pose, instance-transform-shaped), a utf16
/// source path (empty in all three pinned instances) and a 16-byte GUID —
/// NOTHING after the GUID: house-plus's next entity's new-class record
/// starts immediately behind it. The old "106-byte block + GUID + u32"
/// reading had fused the short pointer form into the block, desynced
/// 4 bytes on the escaped form (guest-house @0x1d18aff) and swallowed the
/// first 4 bytes of the FOLLOWING record as a phantom u32 (harmless only
/// when the image was the last root entity, as in image.skp). Pixel data
/// is NOT inline (it lives with the embedded CDibs).
fn r_cimage(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    ar.take(10)?; // drawbase
    let _layer = ar.read_object()?; // layer pointer (escalates on big maps)
    ar.take(104)?; // 13×f64 placement block
    let _path = ar.utf16()?;
    ar.take(16)?; // GUID
    Ok(Entity::Image { pid })
}

/// `CComponentInstance` schema 5 (SKP_FORMAT §4o): preamble + drawbase +
/// def-ref u16 (the definition's declared map index; on the continuous
/// path its GLOBAL map slot, §4s) + 13×f64 transform + instance name +
/// 16-byte GUID. Extent pinned to zero slack by the two adjacent instances
/// in box-component-two-instances.skp.
fn r_ccomponentinstance(ar: &mut CArchive) -> Result<Entity, Stall> {
    r_instance_like(ar, false)
}

/// `CGroup` schema 1 (SKP_FORMAT §4s): IDENTICAL layout to
/// `CComponentInstance` — a group is a component instance with an
/// anonymous definition, byte-proven on house.skp's 58 groups.
fn r_cgroup(ar: &mut CArchive) -> Result<Entity, Stall> {
    r_instance_like(ar, true)
}

fn r_instance_like(ar: &mut CArchive, is_group: bool) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    let drawbase = ar.take(10)?; // SKP_FORMAT §4q
    let material = u16::from_le_bytes([drawbase[0], drawbase[1]]); // 0 = none
    let hidden = drawbase[2] != 0;
    let layer = u16::from_le_bytes([drawbase[8], drawbase[9]]);
    // §4o revision (§4s addendum): the def-ref is an MFC OBJECT POINTER —
    // a back-ref word for slots < 0x8000 (byte-identical to the old raw
    // u16 read), escalating through `7F FF` + u32 on big maps
    // (theater-2017 @0x606d24-era defs).
    let defref = match ar.read_object()? {
        Child::Obj(i) | Child::Ref(i) => u32::try_from(i).unwrap_or(u32::MAX),
        Child::Null => 0,
    };
    let tf_at = ar.pos;
    let mut transform = [0.0f64; 13];
    for slot in transform.iter_mut() {
        *slot = ar.f8()?;
    }
    let name = ar.utf16()?;
    ar.take(16)?; // GUID
    Ok(Entity::InstancePlaced {
        pid,
        defref,
        transform: Box::new(transform),
        name,
        tf_at,
        hidden,
        layer,
        material,
        is_group,
    })
}

fn r_carccurve(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?; // 00 00 <mask> + pid
    ar.take(5)?; // 00 0c 00 00 00
    let mut params = [0.0f64; 14];
    for slot in params.iter_mut() {
        *slot = ar.f8()?; // arc geometry (center/axes/radius/angles)
    }
    Ok(Entity::ArcCurve {
        pid,
        params: Box::new(params),
    })
}

// ---- continuous-walk bodies (SKP_FORMAT §4s; port of tools/contwalk.py) ----

/// `CLayer` schema 2 (§4s; house first decl @0x1e4765): preamble + display
/// name + hidden:u8 + u16 + a pid-style presence mask with its bytes (§4i
/// encoding; mask 0 on every authored file, 2–3 pid bytes on a
/// third-party house whose layers were stamped by a plugin) + internal
/// `Layer_<name>` + u16 + RGBA + utf16 + 21B tail (holds an f64, 0.0/0.5
/// observed). The old "hidden:u32" reading fused the hidden byte, the u16
/// and an empty mask — byte-identical whenever the mask is 0.
fn r_clayer(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    let name = ar.utf16()?;
    let hidden = ar.u1()? != 0;
    ar.take(2)?; // u16
    pid_field(ar)?; // secondary pid (semantics unknown)
    let _internal = ar.utf16()?;
    ar.take(2)?; // u16
    let b = ar.take(4)?;
    let rgba = [b[0], b[1], b[2], b[3]];
    ar.utf16()?;
    ar.take(21)?;
    Ok(Entity::Layer {
        pid,
        name,
        hidden,
        rgba,
    })
}

/// Plausibility cap for a definition's declared entity count (§4s guard —
/// a garbage count means the walk desynced; stall rather than spin).
const MAX_DEF_ENTITIES: usize = 1_000_000;

/// `CComponentDefinition` schema 10 (§4s): preamble (attr-ptr may be
/// non-null: GSU/GeoReference dicts on warehouse defs) + 10B drawbase +
/// 12B mid + u32 layer-count + that many layer objects (each def carries
/// its own Layer0 copy — 50 CLayer objects in house) + §4h prelude
/// `<decl:u32> 00 00 <count:u32>` + count entities + tail: 6B + GUID(16) +
/// name + desc + utf16 + u32 UNIX timestamp + a 42–47B midtail (not fully
/// pinned — located structurally by scanning ≤96 bytes for the CThumbnail
/// new-class record or a class-ref to the already-mapped CThumbnail
/// class) + CThumbnail object.
fn r_ccomponentdefinition(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    ar.take(10)?; // drawbase
    ar.take(12)?; // mid block
    let nlay = ar.u4()? as usize;
    if nlay > 64 {
        return Err(Stall::new("<def layer-count>", ar.pos, ar.map.len()));
    }
    for _ in 0..nlay {
        ar.read_object()?; // inline CLayer copy or back-ref to a list layer
    }
    // §4h prelude REVISED (§4s addendum, theater-2017 @0x606d24):
    // `<decl:u16, 0x7FFF-escalated to u32> <u32 0> <count:u32>` — for
    // decl < 0x7FFF byte-identical to the old `<decl:u32> 00 00 <count>`
    // reading (the u16's high-zero half merges into the zero block).
    // decl − 1 = the def's declared GLOBAL map index.
    let mut decl = ar.u2()? as usize;
    if decl == 0x7FFF {
        decl = ar.u4()? as usize;
    }
    let z = ar.take(4)?;
    if z != [0, 0, 0, 0] {
        return Err(Stall::new("<def prelude>", ar.pos, ar.map.len()));
    }
    let count = ar.u4()? as usize;
    // `decl` is normally the writer-side map index + 1, but third-party
    // models (a large bathroom scene, 2017-saved) declare 0 on some
    // definitions — a missing declaration, not a desynced stream, since
    // the entity list and tail behind it read exactly. A zero declaration
    // simply carries no index.
    if count > MAX_DEF_ENTITIES {
        return Err(Stall::new("<def prelude>", ar.pos, ar.map.len()));
    }
    let mut entities = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        entities.push(ar.read_object()?);
    }
    // Definition tail REVISED (§4s addendum, third revision): `<u32> <one
    // NULLABLE object pointer: the head of a CRelationship chain> <GUID(16)>
    // <name> <description> <source path> <u32 UNIX timestamp>`. house's
    // "6 mystery bytes" are the u32 (0) + a null head pointer; theater's
    // "Group#119" and the guest-house library components carry u32 1 + an
    // inline CRelationship whose `next` is null; a third-party apartment
    // model carries u32 1 + a TWO-node chain (the head's `next` inlines a
    // second CRelationship). The provenance block always follows the head
    // pointer — the earlier "definition provenance is optional when a
    // relationship carries it" rule was this same layout misread with the
    // block inside the relationship record.
    let rel_flag = ar.u4()?;
    if rel_flag > 4096 {
        return Err(Stall::new("<def relationships>", ar.pos, ar.map.len()));
    }
    ar.read_object_expect("CRelationship")?; // chain head (nullable)
    let guid_bytes = ar.take(16)?;
    let guid: String = guid_bytes.iter().map(|b| format!("{b:02x}")).collect();
    let name = ar.utf16()?;
    let desc = ar.utf16()?;
    ar.utf16()?; // source path (library provenance)
    let timestamp = ar.u4()?;
    // Behaviour (§7.1): after the timestamp come a u32 and 24 bytes, then
    // u8 glues, u8 cuts, u32 plane, u8 bits (1 faces the camera, 2 shadows
    // face the sun).
    let p = ar.pos;
    let behaviour = match ar.d.get(p + 28..p + 35) {
        Some(b) => crate::settings::Behaviour {
            glues_to_surface: b[0] != 0,
            cuts_opening: b[1] != 0,
            glue_plane: u32::from_le_bytes([b[2], b[3], b[4], b[5]]),
            always_faces_camera: b[6] & 1 != 0,
            shadows_face_sun: b[6] & 2 != 0,
        },
        None => Default::default(),
    };
    // Midtail: scan for the thumbnail structurally (§4s: the block between
    // the timestamp and the thumbnail varies 42–47 bytes and is not pinned).
    let mut thumb_at = None;
    for q in p..(p + 96).min(ar.d.len().saturating_sub(10)) {
        let w = u16::from_le_bytes([ar.d[q], ar.d[q + 1]]);
        if w == 0xFFFF && &ar.d[q + 2..q + 10] == b"\x01\x00\x0a\x00CThu" {
            thumb_at = Some(q);
            break;
        }
        if (w & 0x8000) != 0 && w != 0xFFFF {
            let idx = (w & 0x7FFF) as usize;
            if matches!(ar.map.get(idx), Some(crate::carchive::Slot::Class(n)) if n == "CThumbnail")
            {
                thumb_at = Some(q);
                break;
            }
        }
    }
    let Some(q) = thumb_at else {
        return Err(Stall::new("<def midtail unbounded>", p, ar.map.len()));
    };
    ar.take(q - p)?;
    ar.read_object()?; // the CThumbnail
    Ok(Entity::ComponentDef {
        pid,
        meta: Box::new(DefMeta {
            name,
            desc,
            guid,
            timestamp,
            behaviour,
        }),
        decl_index: decl.saturating_sub(1),
        count,
        entities,
    })
}

/// `CThumbnail` schema 1 (§4s): 3B (null attr-ptr + mask 0) + camera
/// object + nullable image. The children are the two canonical pad-slot
/// binding sites: pre-model class-refs resolve as CCamera then CDib.
fn r_cthumbnail(ar: &mut CArchive) -> Result<Entity, Stall> {
    ar.take(3)?;
    let camera = ar.read_object_expect("CCamera")?;
    let image = ar.read_object_expect("CDib")?;
    Ok(Entity::Thumbnail { camera, image })
}

/// `CCamera` schema 5 (§4s): NO preamble — 137B raw (eye/target/up f64s,
/// fields TBD) + u16 + utf16 description + 33B tail.
fn r_ccamera(ar: &mut CArchive) -> Result<Entity, Stall> {
    ar.take(137)?;
    ar.take(2)?; // u16
    let desc = ar.utf16()?;
    ar.take(33)?;
    Ok(Entity::Camera { desc })
}

/// `CDib` schema 3 (§4f/§4s): u32 subtype (1=JPEG, 4=PNG) + u32 length +
/// payload. Exactly that — width/height/filename after a material's
/// texture belong to `CMaterial`, not `CDib`.
fn r_cdib(ar: &mut CArchive) -> Result<Entity, Stall> {
    let subtype = ar.u4()?;
    let len = ar.u4()? as usize;
    let at = ar.pos;
    ar.take(len)?;
    Ok(Entity::Dib { subtype, at, len })
}

/// `CAttributeContainer` schema 0 (§4s): preamble + child objects until a
/// null tag. A child class-ref below the calibrated base is a named
/// dictionary (the §4s context-directed binding).
fn r_cattributecontainer(ar: &mut CArchive) -> Result<Entity, Stall> {
    entity_preamble(ar)?;
    let mut children = Vec::new();
    loop {
        let c = ar.read_object_expect("CAttributeNamed")?;
        if c == Child::Null {
            break;
        }
        children.push(c);
    }
    Ok(Entity::AttrContainer { children })
}

/// Consume one typed attribute value (§4s): 0x00 nil (ZERO value bytes —
/// theater-2017 dynamic-component dicts), 0x04 i32, 0x06 f64, 0x07
/// bool(u8), 0x0a utf16, 0x0b typed array (`u32 count + count ×
/// (element-type:u8 + value)`, recursive — each element carries its OWN
/// type byte: guest-house "CWSVelocities" is 10 × (0x06 + f64), and the
/// theater-2017 "UserIdsKey"-style single-element lists that pinned the
/// old "one shared element type" reading are byte-identical either way at
/// count 1). An unknown type stalls loudly (the walk must not guess a
/// span).
fn attr_value(ar: &mut CArchive, t: u8, depth: u32) -> Result<(), Stall> {
    match t {
        0x00 => {} // nil: no value bytes
        0x04 => {
            ar.take(4)?;
        }
        0x06 => {
            ar.take(8)?;
        }
        0x07 => {
            ar.take(1)?;
        }
        0x0a => {
            ar.utf16()?;
        }
        // 8-byte scalar (guest-house "2Dtools" dict, key "z" — an f64-like
        // coordinate; span pinned by the empty-key terminator right behind).
        0x0c => {
            ar.take(8)?;
        }
        // 3×f64 point/vector (a third-party house's "FredoTools_ThruPaint"
        // dict, key "i_r": a 4-element array of them, each element carrying
        // its own 0x11 type byte; the next key's string marker sits exactly
        // 24 bytes behind each).
        0x11 => {
            ar.take(24)?;
        }
        0x0b => {
            let n = ar.u4()? as usize;
            if n > 1_000_000 || depth > 8 {
                return Err(Stall::new("<attr array bounds>", ar.pos, ar.map.len()));
            }
            for _ in 0..n {
                let et = ar.u1()?;
                attr_value(ar, et, depth + 1)?;
            }
        }
        other => {
            return Err(Stall::new(
                format!("<attr type {other:#x}>"),
                ar.pos,
                ar.map.len(),
            ));
        }
    }
    Ok(())
}

/// `CAttributeNamed` schema 1 (§4s): preamble + 4B + dictionary name +
/// entries `[key + type:u8 + value]*` until an empty key + u32. Value
/// types in [`attr_value`].
fn r_cattributenamed(ar: &mut CArchive) -> Result<Entity, Stall> {
    entity_preamble(ar)?;
    ar.take(4)?;
    let name = ar.utf16()?;
    let mut entries = 0usize;
    loop {
        let key = ar.utf16()?;
        if key.is_empty() {
            break;
        }
        let t = ar.u1()?;
        attr_value(ar, t, 0)?;
        entries += 1;
    }
    ar.take(4)?; // u32 tail
    Ok(Entity::AttrNamed { name, entries })
}

/// `CFaceTextureCoords` schema 4 — (§4u, pin-* corpus):
/// preamble + u32 + 24×f64 affine block + TWO pin lists (front then
/// back), each `u32 count + count × (anchor_u, anchor_v, face_x, face_y)`
/// f64 quads in inches (x components negated), + u32 + u32 flags. The
/// old "25 f64 + u32 + u32" reading was the count-0/count-0 case
/// (24 f64 + four u32s) — byte-identical when no pins exist. Pinned by
/// pin-one/pin-four (back-side pins at authored midpoints) and
/// theater-2017 def[277] (front-side pins: a birch texture fitted to a
/// 4×8-foot plywood sheet), every instance resuming its face exactly.
fn r_cfacetexturecoords(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    ar.take(4)?;
    let mut k = [0.0f64; 24]; // two 3×3 projective maps + extras (§4v)
    for slot in k.iter_mut() {
        *slot = ar.f8()?;
    }
    let mut pins: [Vec<[f64; 4]>; 2] = [Vec::new(), Vec::new()];
    for side in pins.iter_mut() {
        let n = ar.u4()?; // per-side pin count (front, then back)
        if n > 4 {
            // no SketchUp mapping has more than 4 pins; a bigger value
            // means a desynced stream — stall rather than consume wildly
            return Err(Stall::new("<ftc pin count>", ar.pos, ar.map.len()));
        }
        for _ in 0..n {
            let mut p = [0.0f64; 4]; // (anchor_u, anchor_v, face_x, face_y)
            for c in p.iter_mut() {
                *c = ar.f8()?;
            }
            side.push(p);
        }
    }
    let flags = [ar.u4()?, ar.u4()?];
    let [front_pins, back_pins] = pins;
    Ok(Entity::FaceTextureCoords {
        pid,
        ftc: Box::new(Ftc {
            front: k[0..9].try_into().unwrap(),
            front_extra: k[9..12].try_into().unwrap(),
            back: k[12..21].try_into().unwrap(),
            back_extra: k[21..24].try_into().unwrap(),
            front_pins,
            back_pins,
            flags,
        }),
    })
}

/// `CConstructionLine` schema 1 (guide.skp @0x11b63, §4l single-instance
/// rules): preamble + drawbase(10) + 8×f64 (point, unit direction, the
/// two line-parameter bounds — ±1e30 on an infinite guide) + 4B tail.
/// The old 7-byte tail was pinned on guide.skp, whose guide sits in a
/// zero run the extra bytes hid in; guest-house's mid-definition guides
/// (@0x3623ea4) put the NEXT record's class-ref hard behind the record
/// and pin the tail to exactly 4.
fn r_cconstructionline(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    ar.take(10)?; // drawbase (§4q)
    let mut v = [0.0f64; 8];
    for slot in v.iter_mut() {
        *slot = ar.f8()?;
    }
    ar.take(4)?; // tail (zeros observed)
    Ok(Entity::ConstructionLine {
        pid,
        params: Box::new(v),
    })
}

/// `CCurve` schema 4 (§4s addendum; theater-2017 @0x56df8d, curve.skp decl
/// @0x80b1): preamble + u8 + u32 member-edge count. NO owned children —
/// member edges are ordinary list/loop elements back-referencing the curve
/// through their curve pointer (same direction as CArcCurve). Registered
/// for the CONTINUOUS walk only: the legacy path's pinned attributes.skp
/// resync histogram counts a CCurve stall and must not move.
fn r_ccurve(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    ar.take(1)?; // u8 flag
    let members = ar.u4()?;
    Ok(Entity::Curve { pid, members })
}

/// `CRelationship` schema 0 (§4s addendum, REVISED twice): preamble + TWO
/// OBJECT POINTERS (short back-ref words on theater — the old reading's
/// "u32 0x27ac353e" was really `3e 35` + `ac 27`; the `7F FF` + u32
/// big-object escape on giant maps) + a NULLABLE OBJECT POINTER to the next
/// relationship in the chain. On every corpus instance the next pointer is
/// null (`00 00`), which the guest-house-era reading took for a u16 ahead of
/// a provenance block; a third-party apartment model puts an inline
/// class-ref there — a second CRelationship — followed by the definition's
/// provenance block, which is therefore the DEFINITION tail's and not this
/// record's (see [`r_ccomponentdefinition`]).
fn r_crelationship(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    let a = ar.read_object()?; // object pointer A
    let b = ar.read_object()?; // object pointer B
    let next = ar.read_object_expect("CRelationship")?; // chain link
    Ok(Entity::Relationship { pid, a, b, next })
}

/// `CMaterial` schema 12 (§4n, §8.1) as the tagged list element it is:
/// preamble (the attribute pointer carries renderer-plugin dictionaries —
/// V-Ray's "VRayInfo"/"VRayPlugins" on a third-party bathroom model) +
/// name + `u8` textured flag + [textured: a NULLABLE attribute-container
/// pointer for the texture itself (null on every authored file; a
/// "VRayTextureHelper" dictionary on a third-party house's "Two Sided"
/// material)] + `u8` (0 observed) + [textured: CDib object pointer
/// (inline new-class/class-ref, or a back-ref to another material's dib
/// when the image is shared) + an optional u32 after JPEG payloads +
/// applied size w,h f64 + filename + average RGBA + u8] + RGBA + utf16 +
/// 8 bytes + opacity f64 + use-opacity flag u8. The material list is
/// `<count:u32>` + count tagged records (record 0 declares the class);
/// the byte-scan walker's "holder pointer + attribute objects + trailer"
/// tail was this framing read out of phase — the next record's class-ref
/// tag plus its `00 00 00` preamble — which broke on the first record
/// with a non-null attribute pointer. The earlier "u16 flag + u16 0"
/// reading of the textured head was `u8 1 + null pointer + u8 0` — byte
/// identical until the pointer is non-null.
fn r_cmaterial(ar: &mut CArchive) -> Result<Entity, Stall> {
    let pid = entity_preamble(ar)?;
    let name = ar.utf16()?;
    let textured = ar.u1()?;
    let mut dib = Child::Null;
    match textured {
        0 => {
            ar.take(1)?; // u8 (0 observed)
        }
        1 => {
            ar.read_object_expect("CAttributeContainer")?; // texture attributes
            ar.take(1)?; // u8 (0 observed)
            dib = ar.read_object_expect("CDib")?;
            // JPEG payloads carry a trailing u32 (70/99 observed): the
            // filename marker sits 16 bytes past here without it, 20 with.
            let p = ar.pos;
            if ar.d.get(p + 20..p + 23) == Some(b"\xff\xfe\xff") {
                ar.take(4)?;
            } else if ar.d.get(p + 16..p + 19) != Some(b"\xff\xfe\xff") {
                return Err(Stall::new("<material texture tail>", p, ar.map.len()));
            }
            ar.take(16)?; // applied size w, h (inches)
            ar.utf16()?; // texture filename
            ar.take(4)?; // average RGBA
            ar.take(1)?;
        }
        _ => return Err(Stall::new("<material texture flag>", ar.pos, ar.map.len())),
    }
    ar.take(4)?; // RGBA (solid color; textured: second average color)
    let empty = ar.utf16()?;
    if !empty.is_empty() {
        return Err(Stall::new("<material tail>", ar.pos, ar.map.len()));
    }
    ar.take(8)?;
    ar.f8()?; // opacity slider value
    ar.take(1)?; // use-opacity flag
    Ok(Entity::Material { pid, name, dib })
}

#[cfg(test)]
mod alloc_dos_tests {
    use crate::carchive::CArchive;

    /// A `CFace` whose declared loop count is `0xFFFFFFFF` must STALL on the
    /// truncated stream, not attempt a multi-gigabyte reservation. Before the
    /// `count.min(4096)` cap this called `Vec::with_capacity(4.29e9)` and
    /// aborted the process on the allocation.
    #[test]
    fn hostile_face_loop_count_stalls_without_giant_alloc() {
        // FF FF <schema=3> <namelen=5> "CFace"
        let mut d = vec![0xFF, 0xFF, 3, 0, 5, 0];
        d.extend_from_slice(b"CFace");
        // body: preamble (00 00 lead + mask 00) + 10B drawbase + 4×f64 plane
        d.extend_from_slice(&[0, 0, 0]);
        d.extend_from_slice(&[0u8; 10]);
        d.extend_from_slice(&[0u8; 32]);
        // loop count = u32::MAX, then the stream ends
        d.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());

        let mut ar = CArchive::new(&d, 0);
        // Must return Err (a stall at EOF), reached without a huge allocation.
        assert!(ar.read_object().is_err());
    }
}
