//! Reader for the post-2017 `.skp` container (SketchUp 2026, `{26.x}`;
//! SKP_FORMAT §16). Derived from the corpus/2026 saves against their
//! decoded 2017 originals: entities keep their persistent ids across the
//! conversion, so every field below is pinned to a known 2017 value.
//!
//! The container is the two UTF-16 header string records, then a ZIP
//! archive. `model.dat` is a tree of `u16 tag | u32 length | payload`
//! records; tags are type-scoped (one tag, one meaning, wherever it
//! appears), so the reader descends only into tags it knows as containers.
//! Materials live beside it as `materials/<name>/material.xml` plus their
//! texture images.

use std::collections::HashMap;

use crate::model::{Error, Model};
use crate::zip::Archive;
use crate::{
    AttrValue, Attribute, Definition, Diagnostic, FaceTexture, GeometryRun, Guide, Image, Instance,
    Layer, Material, Mesh, MeshEdge, MeshFace, PlacedInstance, Topology, INCH,
};

const M_PER_INCH: f64 = 0.0254;

// ---------------------------------------------------------------- records

/// One record: its tag, payload, and the payload's offset in `model.dat`.
#[derive(Clone, Copy)]
pub(crate) struct Rec<'a> {
    pub(crate) tag: u16,
    pub(crate) data: &'a [u8],
    pub(crate) off: usize,
}

/// The child records of a container payload; they must tile it exactly.
pub(crate) fn records(data: &[u8], base: usize) -> Result<Vec<Rec<'_>>, Error> {
    let mut out = Vec::new();
    let mut o = 0usize;
    while o < data.len() {
        let head = data
            .get(o..o + 6)
            .ok_or_else(|| err(base + o, "truncated record header"))?;
        let tag = u16::from_le_bytes([head[0], head[1]]);
        let len = u32::from_le_bytes([head[2], head[3], head[4], head[5]]) as usize;
        let body = (o + 6)
            .checked_add(len)
            .and_then(|end| data.get(o + 6..end))
            .ok_or_else(|| err(base + o, &format!("record {tag:#06x} overruns its parent")))?;
        out.push(Rec {
            tag,
            data: body,
            off: base + o + 6,
        });
        o += 6 + len;
    }
    Ok(out)
}

pub(crate) fn err(at: usize, what: &str) -> Error {
    Error(format!("post-2017 model.dat @{at:#x}: {what}"))
}

/// A container record's children.
pub(crate) struct Node<'a> {
    pub(crate) at: usize,
    pub(crate) len: usize,
    pub(crate) kids: Vec<Rec<'a>>,
}

impl<'a> Node<'a> {
    pub(crate) fn of(r: Rec<'a>) -> Result<Node<'a>, Error> {
        Ok(Node {
            at: r.off,
            len: r.data.len(),
            kids: records(r.data, r.off)?,
        })
    }
    pub(crate) fn get(&self, tag: u16) -> Option<Rec<'a>> {
        self.kids.iter().copied().find(|r| r.tag == tag)
    }
    pub(crate) fn all(&self, tag: u16) -> impl Iterator<Item = Rec<'a>> + '_ {
        self.kids.iter().copied().filter(move |r| r.tag == tag)
    }
    pub(crate) fn node(&self, tag: u16) -> Result<Option<Node<'a>>, Error> {
        self.get(tag).map(Node::of).transpose()
    }
    pub(crate) fn need(&self, tag: u16) -> Result<Node<'a>, Error> {
        self.node(tag)?
            .ok_or_else(|| err(self.at, &format!("missing record {tag:#06x}")))
    }
    pub(crate) fn nodes(&self, tag: u16) -> Result<Vec<Node<'a>>, Error> {
        self.all(tag).map(Node::of).collect()
    }
}

/// Little-endian unsigned integer of the payload's width: ids are 1–3
/// bytes, just wide enough for their value.
pub(crate) fn uint(r: Option<Rec<'_>>) -> Option<u32> {
    let b = r?.data;
    (b.len() <= 4).then(|| b.iter().rev().fold(0u32, |v, &x| (v << 8) | x as u32))
}

pub(crate) fn f64s<const N: usize>(r: Option<Rec<'_>>, at: usize) -> Result<[f64; N], Error> {
    let b = r
        .map(|r| r.data)
        .filter(|b| b.len() == N * 8)
        .ok_or_else(|| err(at, &format!("expected {N} f64 values")))?;
    let mut out = [0.0; N];
    for (i, v) in out.iter_mut().enumerate() {
        *v = f64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
    }
    Ok(out)
}

pub(crate) fn text(r: Option<Rec<'_>>) -> String {
    r.map(|r| String::from_utf8_lossy(r.data).into_owned())
        .unwrap_or_default()
}

// ---------------------------------------------------------------- tags

// Entity scaffolding.
const ENTITY: u16 = 0x05DC; // entity base: id (+ attributes)
const ENTITY_ATTRS: u16 = 0x05DD;
const ENTITY_ID: u16 = 0x05DE;
const DRAWING: u16 = 0x07D0; // drawing element: entity base + refs + flags
const DRAWING_MATERIAL: u16 = 0x07D1; // a face's FRONT material
const DRAWING_LAYER: u16 = 0x07D2;
const DRAWING_FLAGS: u16 = 0x07D3; // 0x01 hidden, 0x08 soft, 0x10 smooth
                                   // Entity containers (a definition's or the model's entities).
const ENTITIES: u16 = 0x1388;
const VERTICES: u16 = 0x1389;
const EDGES: u16 = 0x138A;
const FACES: u16 = 0x138B;
const COMPONENTS: u16 = 0x138C;
const GROUPS: u16 = 0x138D;
const IMAGES: u16 = 0x1390;
const GUIDE_LINES: u16 = 0x1391;
const GUIDE_POINTS: u16 = 0x1392;
const SECTION_PLANES: u16 = 0x1393;
const PLAIN_CURVES: u16 = 0x1396;
const ARC_CURVES: u16 = 0x1397;
const TEXTS: u16 = 0x1398;
const DIMENSIONS: u16 = 0x1399;
const ORDER: u16 = 0x138E; // packed ids of the top-level entities
                           // Geometry.
const VERTEX: u16 = 0x09C4;
const VERTEX_POINT: u16 = 0x09C5;
const EDGE: u16 = 0x0BB8;
const EDGE_START: u16 = 0x0BB9;
const EDGE_END: u16 = 0x0BBA;
const FACE: u16 = 0x0DAC;
const FACE_PLANE: u16 = 0x0DAD;
const FACE_LOOPS: u16 = 0x0DAE;
const FACE_BACK_MATERIAL: u16 = 0x0DAF;
const LOOP: u16 = 0x1194;
const LOOP_USES: u16 = 0x1195;
const EDGE_USE: u16 = 0x0FA0;
const EDGE_USE_EDGE: u16 = 0x0FA1;
const EDGE_USE_FORWARD: u16 = 0x0FA2;
const CURVE: u16 = 0x4A38;
const CURVE_MEMBERS: u16 = 0x4A39;
const GUIDE_LINE: u16 = 0x4269;
const GUIDE_LINE_GEOMETRY: u16 = 0x426A; // point, direction, bounds
                                         // Instances.
const GROUP: u16 = 0x1D4C;
const INSTANCE: u16 = 0x1964;
const INSTANCE_NAME: u16 = 0x1965;
const INSTANCE_TRANSFORM: u16 = 0x1966;
const INSTANCE_DEFINITION: u16 = 0x1967;
// Definitions.
const DEFINITION: u16 = 0x157C;
const DEFINITION_GUID: u16 = 0x157D;
const DEFINITION_NAME: u16 = 0x157E;
// Attribute dictionaries and typed values.
const ATTR_ROOT: u16 = 0x36B1;
const ATTR_SET: u16 = 0x36B2;
const ATTR_PAIRS: u16 = 0x36B5;
const ATTR_KEY: u16 = 0x36B6;
const VALUE: u16 = 0x38A4;
const VALUE_INT: u16 = 0x38A7;
const VALUE_F64: u16 = 0x38A9;
const VALUE_BOOL: u16 = 0x38AA;
const OPTION_PAIRS: u16 = 0x61AC;
const OPTION_KEY: u16 = 0x61AD;
// Face texture placement (inside the face's attribute set).
const TEXTURE: u16 = 0x2710;
const TEXTURE_FRONT: u16 = 0x2711;
const TEXTURE_BACK: u16 = 0x2712;
const TEXTURE_SIDE: u16 = 0x2713;
const TEXTURE_FLAGS: u16 = 0x2714;
const TEXTURE_MATRIX: u16 = 0x2715;
const TEXTURE_EXTRA: u16 = 0x2716;
const TEXTURE_PINS: u16 = 0x2717;
const TEXTURE_PIN: u16 = 0x2718;
const TEXTURE_PIN_ANCHOR: u16 = 0x2719;
const TEXTURE_PIN_FACE: u16 = 0x271A;
// Model sections.
const MATERIALS: u16 = 0x01F7;
const LAYERS: u16 = 0x01F8;
const DEFINITIONS: u16 = 0x01F9;
const ROOT: u16 = 0x01F6;

pub(crate) fn entity_id(n: &Node<'_>) -> Result<u32, Error> {
    let e = n.need(ENTITY)?;
    uint(e.get(ENTITY_ID)).ok_or_else(|| err(e.at, "entity without an id"))
}

/// The 0x07d0 header of a drawing element.
struct Drawing<'a> {
    id: u32,
    material: Option<u32>,
    layer: Option<u32>,
    flags: u8,
    attrs: Option<Rec<'a>>,
}

fn drawing<'a>(n: &Node<'a>) -> Result<Drawing<'a>, Error> {
    let h = n.need(DRAWING)?;
    let e = h.need(ENTITY)?;
    Ok(Drawing {
        id: uint(e.get(ENTITY_ID)).ok_or_else(|| err(e.at, "entity without an id"))?,
        material: uint(h.get(DRAWING_MATERIAL)),
        layer: uint(h.get(DRAWING_LAYER)),
        flags: uint(h.get(DRAWING_FLAGS)).unwrap_or(0) as u8,
        attrs: e.get(ENTITY_ATTRS),
    })
}

/// A packed id list: repeated `<width:u8> <id: width bytes LE>`.
pub(crate) fn id_list(b: &[u8]) -> usize {
    let (mut n, mut o) = (0, 0);
    while o < b.len() {
        o += 1 + b[o] as usize;
        n += 1;
    }
    n
}

// ---------------------------------------------------------------- materials

/// The attributes of the first `<prefix …>` start tag in `xml`.
pub(crate) fn xml_attrs(xml: &str, prefix: &str) -> Option<HashMap<String, String>> {
    let start = xml.find(prefix)? + prefix.len();
    let tag = &xml[start..start + xml[start..].find('>')?];
    let mut out = HashMap::new();
    let mut rest = tag;
    while let Some(eq) = rest.find("=\"") {
        let key = rest[..eq].trim().to_string();
        let v0 = eq + 2;
        let v1 = v0 + rest[v0..].find('"')?;
        out.insert(key, unescape(&rest[v0..v1]));
        rest = &rest[v1 + 1..];
    }
    Some(out)
}

pub(crate) fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find(';') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let ent = &rest[i + 1..i + j];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&rest[i..=i + j]),
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

/// A material record (0x32c8) plus its `materials/<folder>/material.xml`.
/// The record's name is the archive folder; the XML keeps the original
/// name (2017's unnamed `*N` materials live in folder `_N`).
///
/// A missing XML (the material then reads as unnamed-colour black) or a
/// missing texture image is recorded in `diags`, never silent.
fn material(
    zip: &Archive<'_>,
    rec: &Node<'_>,
    diags: &mut Vec<Diagnostic>,
) -> Result<(u32, Material), Error> {
    let id = entity_id(rec)?;
    let folder = text(rec.get(0x32CC));
    let entry = format!("materials/{folder}/material.xml");
    let xml = match zip.read(&entry).map_err(Error)? {
        Some(b) => String::from_utf8_lossy(&b).into_owned(),
        None => {
            diags.push(Diagnostic::Skipped {
                class: entry,
                at: rec.at,
                bytes: rec.len,
            });
            String::new()
        }
    };
    let a = xml_attrs(&xml, "<mat:material").unwrap_or_default();
    let name = a.get("name").cloned().unwrap_or(folder.clone());
    let num = |k: &str| a.get(k).and_then(|v| v.parse::<f64>().ok());
    let opacity = if a.get("useTrans").map(String::as_str) == Some("1") {
        num("trans").unwrap_or(1.0)
    } else {
        1.0
    };
    let byte = |k: &str| num(k).unwrap_or(0.0) as u8;
    let rgb = [byte("colorRed"), byte("colorGreen"), byte("colorBlue")];
    let has_texture = a.get("hasTexture").map(String::as_str) == Some("1");
    let t = xml_attrs(&xml, "<mat:texture");
    let m = match (has_texture, t) {
        (true, Some(t)) => {
            // The displayed colour: the texture's average (avgColor, packed
            // 0xAABBGGRR), or the tint for a colourised texture (type 2).
            let avg_rgba = if a.get("type").map(String::as_str) == Some("2") {
                Some([rgb[0], rgb[1], rgb[2], 255])
            } else {
                t.get("avgColor")
                    .and_then(|v| v.parse::<u32>().ok())
                    .map(|c| c.to_le_bytes())
            };
            // The image path is folder-relative ("./x.jpg"), or rooted
            // ("materials/<other>/x.jpg") when another material owns it.
            let img = xml_attrs(&xml, "<mat:image ").unwrap_or_default();
            let path = match img.get("path") {
                Some(p) if p.starts_with("./") => format!("materials/{folder}/{}", &p[2..]),
                Some(p) => p.clone(),
                None => format!(
                    "materials/{folder}/{}",
                    t.get("textureFilename").map(String::as_str).unwrap_or("")
                ),
            };
            let size = |k: &str| t.get(k).and_then(|v| v.parse::<f64>().ok());
            let image_bytes = zip.read(&path).map_err(Error)?;
            if image_bytes.is_none() {
                diags.push(Diagnostic::UnresolvedSharedTexture { count: 1 });
            }
            Material::Textured {
                name,
                texture: t.get("textureFilename").cloned(),
                applied_size_in: size("xScale").zip(size("yScale")),
                image_bytes,
                avg_rgba,
                opacity,
            }
        }
        _ => Material::Solid {
            name,
            rgba: [rgb[0], rgb[1], rgb[2], (opacity * 255.0) as u8],
            opacity,
        },
    };
    Ok((id, m))
}

// ---------------------------------------------------------------- entities

/// Dense u16 slots for 2026 ids (which exceed u16 on large models), so
/// materials and layers link through the same slot tables as 2017 files.
struct Slots {
    material: HashMap<u32, u16>,
    layer: HashMap<u32, u16>,
    definition_name: HashMap<u32, String>,
}

impl Slots {
    fn material(&self, id: Option<u32>) -> Option<u16> {
        id.and_then(|i| self.material.get(&i).copied())
    }
    fn layer(&self, id: Option<u32>) -> u16 {
        id.and_then(|i| self.layer.get(&i).copied()).unwrap_or(0)
    }
}

fn texture(attrs: Option<Rec<'_>>) -> Result<Option<FaceTexture>, Error> {
    let Some(a) = attrs else { return Ok(None) };
    let a = Node::of(a)?;
    let Some(t) = (match a.node(ATTR_ROOT)? {
        Some(r) => match r.node(ATTR_SET)? {
            Some(s) => s.node(TEXTURE)?,
            None => None,
        },
        None => None,
    }) else {
        return Ok(None);
    };
    type Side = ([f64; 9], [f64; 3], Vec<[f64; 4]>, u32);
    let side = |tag: u16| -> Result<Side, Error> {
        let Some(s) = t.node(tag)?.map(|n| n.need(TEXTURE_SIDE)).transpose()? else {
            return Ok(([0.0; 9], [0.0; 3], Vec::new(), 0));
        };
        let mut pins = Vec::new();
        if let Some(pl) = s.node(TEXTURE_PINS)? {
            for p in pl.nodes(TEXTURE_PIN)? {
                let a: [f64; 2] = f64s(p.get(TEXTURE_PIN_ANCHOR), p.at)?;
                let f: [f64; 2] = f64s(p.get(TEXTURE_PIN_FACE), p.at)?;
                pins.push([a[0], a[1], f[0], f[1]]);
            }
        }
        Ok((
            f64s(s.get(TEXTURE_MATRIX), s.at)?,
            f64s(s.get(TEXTURE_EXTRA), s.at)?,
            pins,
            uint(s.get(TEXTURE_FLAGS)).unwrap_or(0),
        ))
    };
    let (front, front_extra, front_pins, ff) = side(TEXTURE_FRONT)?;
    let (back, back_extra, back_pins, bf) = side(TEXTURE_BACK)?;
    Ok(Some(FaceTexture {
        front,
        back,
        front_extra,
        back_extra,
        front_pins,
        back_pins,
        flags: [ff, bf],
    }))
}

/// Everything the model needs from one entity container.
struct Built {
    run: GeometryRun,
    /// (defref, instance record offset, name, placement) per instance.
    instances: Vec<(PlacedInstance, usize, String)>,
    guides: Vec<Guide>,
}

fn container(
    n: &Node<'_>,
    start: usize,
    end: usize,
    def: Option<u32>,
    s: &Slots,
    diags: &mut Vec<Diagnostic>,
) -> Result<Built, Error> {
    // Vertices, keyed by persistent id.
    let mut vindex: HashMap<u32, u32> = HashMap::new();
    let mut vertices = Vec::new();
    if let Some(pool) = n.node(VERTICES)? {
        for v in pool.nodes(VERTEX)? {
            let p: [f64; 3] = f64s(v.get(VERTEX_POINT), v.at)?;
            vindex.insert(entity_id(&v)?, vertices.len() as u32);
            // Same conversion as the 2017 mesh (× 0.0254), so both readers
            // produce bit-identical coordinates.
            vertices.push(p.map(|x| x * M_PER_INCH));
        }
    }
    let (mut con, mut sat) = (0usize, 0usize);
    let mut edges = Vec::new();
    let mut ends: HashMap<u32, (u32, u32)> = HashMap::new();
    if let Some(pool) = n.node(EDGES)? {
        for e in pool.nodes(EDGE)? {
            let h = drawing(&e)?;
            let (a, b) = (uint(e.get(EDGE_START)), uint(e.get(EDGE_END)));
            con += 2;
            let (Some(a), Some(b)) = (
                a.and_then(|a| vindex.get(&a).copied()),
                b.and_then(|b| vindex.get(&b).copied()),
            ) else {
                // An edge whose ends do not resolve is dropped, recorded.
                diags.push(Diagnostic::Skipped {
                    class: "edge".into(),
                    at: e.at,
                    bytes: e.len,
                });
                continue;
            };
            sat += 2;
            ends.insert(h.id, (a, b));
            edges.push(MeshEdge {
                pid: h.id,
                v0: a,
                v1: b,
                soft: h.flags & 0x08 != 0,
                smooth: h.flags & 0x10 != 0,
                hidden: h.flags & 0x01 != 0,
                layer: s.layer(h.layer),
            });
        }
    }
    let (mut loops, mut uses) = (0usize, 0usize);
    let mut faces = Vec::new();
    if let Some(pool) = n.node(FACES)? {
        for f in pool.nodes(FACE)? {
            let h = drawing(&f)?;
            let plane: [f64; 4] = f64s(f.get(FACE_PLANE), f.at)?;
            let mut rings: Vec<Vec<u32>> = Vec::new();
            let mut whole = true;
            for lp in f.need(FACE_LOOPS)?.nodes(LOOP)? {
                loops += 1;
                let mut ring = Vec::new();
                for u in lp.need(LOOP_USES)?.nodes(EDGE_USE)? {
                    uses += 1;
                    con += 1;
                    let Some(&(a, b)) = uint(u.get(EDGE_USE_EDGE)).and_then(|e| ends.get(&e))
                    else {
                        whole = false;
                        continue;
                    };
                    sat += 1;
                    // Each use contributes its start vertex.
                    ring.push(if uint(u.get(EDGE_USE_FORWARD)).unwrap_or(0) != 0 {
                        a
                    } else {
                        b
                    });
                }
                whole &= ring.len() >= 3;
                rings.push(ring);
            }
            // A face with an unresolved edge use (or a degenerate loop) is
            // dropped whole, recorded — never emitted with a shortened ring.
            if !whole || rings.is_empty() {
                diags.push(Diagnostic::Skipped {
                    class: "face".into(),
                    at: f.at,
                    bytes: f.len,
                });
                continue;
            }
            let outer = rings.remove(0);
            // Inner loops run opposite to the reader's hole convention
            // (which follows the COLLADA export, like the 2017 path).
            let holes = rings.into_iter().map(|mut r| {
                r.reverse();
                r
            });
            faces.push(MeshFace {
                pid: h.id,
                outer,
                holes: holes.collect(),
                normal: [plane[0], plane[1], plane[2]],
                front_material: s.material(h.material),
                back_material: s.material(uint(f.get(FACE_BACK_MATERIAL))),
                hidden: h.flags & 0x01 != 0,
                layer: s.layer(h.layer),
                texture: texture(h.attrs)?,
            });
        }
    }
    let mut instances = Vec::new();
    let mut placed = Vec::new();
    let mut place = |x: &Node<'_>, is_group: bool| -> Result<(), Error> {
        let h = drawing(x)?;
        let defref = uint(x.get(INSTANCE_DEFINITION))
            .ok_or_else(|| err(x.at, "instance without a definition"))?;
        con += 1;
        sat += usize::from(s.definition_name.contains_key(&defref));
        let t: [f64; 13] = f64s(x.get(INSTANCE_TRANSFORM), x.at)?;
        let p = PlacedInstance {
            defref,
            transform: t,
            is_group,
            material: s.material(h.material).unwrap_or(0),
            hidden: h.flags & 0x01 != 0,
            layer: s.layer(h.layer),
        };
        placed.push(p.clone());
        instances.push((p, x.at, text(x.get(INSTANCE_NAME))));
        Ok(())
    };
    if let Some(pool) = n.node(COMPONENTS)? {
        for x in pool.nodes(INSTANCE)? {
            place(&x, false)?;
        }
    }
    if let Some(pool) = n.node(GROUPS)? {
        for g in pool.nodes(GROUP)? {
            place(&g.need(INSTANCE)?, true)?;
        }
    }
    let mut guides = Vec::new();
    if let Some(pool) = n.node(GUIDE_LINES)? {
        for g in pool.nodes(GUIDE_LINE)? {
            let v: [f64; 8] = f64s(g.get(GUIDE_LINE_GEOMETRY), g.at)?;
            // Rounded like the 2017 path's guides.
            let r5 = |x: f64| (x * 1e5).round() / 1e5;
            guides.push(Guide {
                point_m: [r5(v[0] / INCH), r5(v[1] / INCH), r5(v[2] / INCH)],
                direction: [r5(v[3]), r5(v[4]), r5(v[5])],
            });
        }
    }
    let count =
        |tag: u16| -> Result<usize, Error> { Ok(n.node(tag)?.map(|p| p.kids.len()).unwrap_or(0)) };
    let mut curve_members = Vec::new();
    if let Some(pool) = n.node(PLAIN_CURVES)? {
        for c in pool.nodes(CURVE)? {
            curve_members.push(uint(c.get(CURVE_MEMBERS)).unwrap_or(0));
        }
    }
    let top = n.get(ORDER).map(|r| id_list(r.data)).unwrap_or(0);
    Ok(Built {
        run: GeometryRun {
            start,
            end,
            top_level: top,
            frame: Some(top),
            def_index: def.map(|d| d as usize),
            topology: Topology {
                vertices: vertices.len(),
                edges: edges.len(),
                faces: faces.len(),
                loops,
                edge_uses: uses,
                curves: count(ARC_CURVES)?,
                dimensions: count(DIMENSIONS)?,
                texts: count(TEXTS)?,
                section_planes: count(SECTION_PLANES)?,
                images_placed: count(IMAGES)?,
                construction_points: count(GUIDE_POINTS)?,
            },
            resolved: (sat, con),
            mesh: Mesh {
                vertices,
                faces,
                edges,
            },
            placed,
            curve_members,
        },
        instances,
        guides,
    })
}

// ---------------------------------------------------------------- attributes

/// A typed value holds exactly one child record naming its type.
fn typed_value(r: Rec<'_>) -> Result<Option<AttrValue>, Error> {
    let Some(v) = records(r.data, r.off)?.first().copied() else {
        return Ok(None);
    };
    let b = v.data;
    Ok(match v.tag {
        VALUE_INT if b.len() == 4 => {
            Some(AttrValue::Int(i32::from_le_bytes(b.try_into().unwrap())))
        }
        VALUE_F64 if b.len() == 8 => {
            Some(AttrValue::F64(f64::from_le_bytes(b.try_into().unwrap())))
        }
        VALUE_BOOL if b.len() == 1 => Some(AttrValue::Bool(b[0] != 0)),
        _ => None,
    })
}

/// Alternating `key_tag` key / 0x38a4 typed-value records; the int, f64 and
/// bool values surface (the same value kinds as the 2017 path).
fn key_values(r: Rec<'_>, key_tag: u16, out: &mut Vec<Attribute>) -> Result<(), Error> {
    let mut key: Option<String> = None;
    for k in records(r.data, r.off)? {
        if k.tag == key_tag {
            key = Some(String::from_utf8_lossy(k.data).into_owned());
        } else if k.tag == VALUE {
            if let (Some(key), Some(value)) = (key.take(), typed_value(k)?) {
                out.push(Attribute { key, value });
            }
        }
    }
    Ok(())
}

/// Collect attribute dictionaries (0x36b1 > 0x36b2 > 0x36b3 > 0x36b5) and
/// model option sets (0x61ac) from `r`'s subtree, in document order.
/// Descends only through container records: payloads that do not tile as
/// records (numeric leaves) are skipped. Depth-limited: observed nesting
/// is under 20, and a crafted file must not exhaust the stack.
fn attributes(r: Rec<'_>, out: &mut Vec<Attribute>, depth: u32) -> Result<(), Error> {
    if depth > 64 {
        return Ok(());
    }
    match r.tag {
        ATTR_PAIRS => return key_values(r, ATTR_KEY, out),
        OPTION_PAIRS => return key_values(r, OPTION_KEY, out),
        VERTEX_POINT | FACE_PLANE | INSTANCE_TRANSFORM | TEXTURE_MATRIX | TEXTURE_EXTRA
        | GUIDE_LINE_GEOMETRY | DEFINITION_GUID => return Ok(()),
        _ => {}
    }
    if r.data.len() < 6 {
        return Ok(());
    }
    let Ok(kids) = records(r.data, r.off) else {
        return Ok(());
    };
    if kids.iter().any(|k| k.tag == 0) {
        return Ok(()); // zero bytes that merely tile; not a container
    }
    for k in kids {
        attributes(k, out, depth + 1)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- model

/// Parse a post-2017 file. `version` is the header's version string.
pub(crate) fn parse(d: &[u8], version: String) -> Result<Model, Error> {
    let start =
        crate::ctx::zip_start(d).ok_or_else(|| Error("no ZIP archive after the header".into()))?;
    let zip = Archive::open(d, start).map_err(Error)?;
    let dat = zip
        .read("model.dat")
        .map_err(Error)?
        .ok_or_else(|| Error("post-2017 container without model.dat".into()))?;
    let top_rec = *records(&dat, 0)?
        .first()
        .ok_or_else(|| err(0, "empty model.dat"))?;
    let top = Node::of(top_rec)?;
    let mut diagnostics = Vec::new();

    // Materials (dense slots 1..), layers (slot 0 = the default layer, the
    // list's first entry, as on the 2017 path).
    let mut materials = Vec::new();
    let mut slots = Slots {
        material: HashMap::new(),
        layer: HashMap::new(),
        definition_name: HashMap::new(),
    };
    if let Some(sec) = top.node(MATERIALS)? {
        for rec in sec.need(0x30D4)?.need(0x30D5)?.nodes(0x32C8)? {
            let (id, m) = material(&zip, &rec, &mut diagnostics)?;
            slots.material.insert(id, materials.len() as u16 + 1);
            materials.push(m);
        }
    }
    let material_links = (0..materials.len()).map(|i| (i as u16 + 1, i)).collect();
    let mut layers = Vec::new();
    if let Some(sec) = top.node(LAYERS)? {
        for l in sec.need(0x3A98)?.need(0x3A99)?.nodes(0x3C8C)? {
            let id = entity_id(&l)?;
            // 0x3c8f embeds the layer's colour material, Layer_<name>.
            let rgba = match l.get(0x3C8F) {
                Some(r) => match Node::of(r)?.node(0x32C8)? {
                    Some(m) => match material(&zip, &m, &mut diagnostics)?.1 {
                        Material::Solid { rgba, .. } => [rgba[0], rgba[1], rgba[2], 255],
                        Material::Textured { .. } => [0, 0, 0, 255],
                    },
                    None => [0, 0, 0, 255],
                },
                None => [0, 0, 0, 255],
            };
            slots.layer.insert(id, layers.len() as u16 + 1);
            layers.push(Layer {
                name: text(l.get(0x3C8D)),
                visible: uint(l.get(0x3C8E)).unwrap_or(0) == 0,
                rgba,
            });
        }
    }
    let mut layer_links = Vec::new();
    if !layers.is_empty() {
        layer_links.push((0u16, 0usize));
        layer_links.extend((0..layers.len()).map(|i| (i as u16 + 1, i)));
    }

    // Definitions: id -> name first, so instance links resolve anywhere.
    let mut defs = Vec::new();
    if let Some(sec) = top.node(DEFINITIONS)? {
        for list in sec.nodes(0x1770)? {
            for d in list.need(0x1771)?.nodes(DEFINITION)? {
                let ents = d.need(ENTITIES)?;
                let id = drawing(&ents)?.id;
                let guid: String = d
                    .get(DEFINITION_GUID)
                    .map(|g| g.data.iter().map(|b| format!("{b:02x}")).collect())
                    .unwrap_or_default();
                let rec = d.get(ENTITIES).unwrap();
                slots
                    .definition_name
                    .insert(id, text(d.get(DEFINITION_NAME)));
                let behaviour = crate::settings26::behaviour(&d)?;
                let timestamp = uint(d.get(0x1581)).unwrap_or(0);
                defs.push((
                    id,
                    text(d.get(DEFINITION_NAME)),
                    guid,
                    ents,
                    rec,
                    behaviour,
                    timestamp,
                ));
            }
        }
    }
    let mut definitions = Vec::with_capacity(defs.len());
    let mut definition_links = Vec::with_capacity(defs.len());
    let mut geometry = Vec::with_capacity(defs.len() + 1);
    let mut instances = Vec::new();
    let mut guides = Vec::new();
    let round5 = |x: f64| (x * 1e5).round() / 1e5;
    let mut take = |b: Built, geometry: &mut Vec<GeometryRun>| {
        for (p, at, name) in b.instances {
            instances.push(Instance {
                definition: slots.definition_name.get(&p.defref).cloned(),
                defref: p.defref,
                offset: at,
                translation_m: [
                    round5(p.transform[9] / INCH),
                    round5(p.transform[10] / INCH),
                    round5(p.transform[11] / INCH),
                ],
                transform: p.transform,
                name: Some(name),
                is_group: Some(p.is_group),
                material: Some(p.material),
                hidden: Some(p.hidden),
                layer: Some(p.layer),
            });
        }
        guides.extend(b.guides);
        geometry.push(b.run);
    };
    let (fonts, font_ids) = crate::settings26::fonts(&top)?;
    let mut texts = Vec::new();
    let mut dimensions = Vec::new();
    for (id, name, guid, ents, rec, behaviour, timestamp) in &defs {
        definition_links.push((*id, definitions.len()));
        definitions.push(Definition {
            name: name.clone(),
            guid: guid.clone(),
            behaviour: behaviour.clone(),
            timestamp: *timestamp,
            map_index: Some(*id as usize),
        });
        texts.extend(crate::settings26::texts(ents, &font_ids)?);
        dimensions.extend(crate::settings26::dimensions(ents, &font_ids)?);
        // A definition's run spans its entity container in model.dat.
        let b = container(
            ents,
            rec.off,
            rec.off + rec.data.len(),
            Some(*id),
            &slots,
            &mut diagnostics,
        )?;
        take(b, &mut geometry);
    }
    definition_links.sort_unstable();
    // The root run's span is empty: `Model::scene` treats instances lying
    // outside every run as scene roots, exactly the root's instances.
    let root = top.need(ROOT)?.need(ENTITIES)?;
    let b = container(&root, root.at, root.at, None, &slots, &mut diagnostics)?;
    take(b, &mut geometry);

    texts.extend(crate::settings26::texts(&root, &font_ids)?);
    dimensions.extend(crate::settings26::dimensions(&root, &font_ids)?);

    let camera = match top.node(0x01FA)? {
        Some(c) => Some(crate::settings26::camera(&c.need(0x34BC)?)?),
        None => None,
    };
    let rendering = match top.node(0x01FB)? {
        Some(r) => Some(crate::settings26::rendering(&r.need(0x733C)?)),
        None => None,
    };
    let shadows = match top.node(0x0204)? {
        Some(s) => Some(crate::settings26::shadow(&s.need(0x6590)?)?),
        None => None,
    };
    let st = crate::settings26::styles(&top, &zip)?;
    let scenes = crate::settings26::scenes(&top, &st.names)?;
    let axes = match top.node(0x01FC)? {
        Some(a) => Some(crate::settings26::axes(&a.need(0x4650)?)?),
        None => None,
    };
    let text_defaults = crate::settings26::text_defaults(&top, &font_ids)?;
    let dimension_defaults = crate::settings26::dimension_defaults(&top, &font_ids)?;
    let anti_aliased_textures = uint(top.get(0x020C)).map(|v| v != 0);

    let mut attrs = Vec::new();
    for k in &top.kids {
        attributes(*k, &mut attrs, 0)?;
    }

    // Embedded images: every PNG/JPEG in the archive (thumbnails, textures).
    let mut images = Vec::new();
    for e in &zip.entries {
        let kind = match e
            .name
            .rsplit('.')
            .next()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") => "png",
            Some("jpg" | "jpeg") => "jpg",
            _ => continue,
        };
        images.push(Image {
            kind: kind.into(),
            bytes: e.size,
        });
    }

    let units = crate::settings::Units::from_attributes(&attrs);
    let animation = crate::settings::Animation::from_attributes(&attrs);
    let geo_located = attrs.iter().any(|a| {
        a.key == "UsesGeoReferencing"
            && matches!(
                a.value,
                crate::AttrValue::Bool(true) | crate::AttrValue::Int(1..)
            )
    });
    Ok(Model {
        container: crate::ctx::Container::Zip,
        version,
        model_guid: None,
        definitions,
        instances,
        geometry,
        materials,
        layers,
        scenes,
        guides,
        attributes: attrs,
        images,
        camera,
        rendering,
        shadows,
        units,
        styles: st.styles,
        active_style: st.active,
        watermarks: st.current_watermarks,
        fonts,
        texts,
        dimensions,
        axes,
        text_defaults,
        dimension_defaults,
        anti_aliased_textures,
        animation,
        geo_located,
        definition_links,
        layer_links,
        material_links,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(tag: u16, payload: &[u8]) -> Vec<u8> {
        let mut v = tag.to_le_bytes().to_vec();
        v.extend((payload.len() as u32).to_le_bytes());
        v.extend(payload);
        v
    }
    fn cat(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.concat()
    }
    fn header(id: u8) -> Vec<u8> {
        rec(
            DRAWING,
            &cat(&[
                rec(ENTITY, &rec(ENTITY_ID, &[id])),
                rec(DRAWING_FLAGS, &[6]),
            ]),
        )
    }
    fn vertex(id: u8, x: f64) -> Vec<u8> {
        let p: Vec<u8> = [x, 0.0, 0.0].iter().flat_map(|c| c.to_le_bytes()).collect();
        rec(
            VERTEX,
            &cat(&[rec(ENTITY, &rec(ENTITY_ID, &[id])), rec(VERTEX_POINT, &p)]),
        )
    }
    fn edge(id: u8, a: u8, b: u8) -> Vec<u8> {
        rec(
            EDGE,
            &cat(&[header(id), rec(EDGE_START, &[a]), rec(EDGE_END, &[b])]),
        )
    }
    fn face(id: u8, edges: &[u8]) -> Vec<u8> {
        let uses: Vec<Vec<u8>> = edges
            .iter()
            .map(|&e| {
                rec(
                    EDGE_USE,
                    &cat(&[rec(EDGE_USE_EDGE, &[e]), rec(EDGE_USE_FORWARD, &[1])]),
                )
            })
            .collect();
        let lp = rec(LOOP, &rec(LOOP_USES, &cat(&uses)));
        rec(
            FACE,
            &cat(&[
                header(id),
                rec(FACE_PLANE, &[0u8; 32]),
                rec(FACE_LOOPS, &lp),
            ]),
        )
    }

    /// A triangle whose loop names a missing edge is dropped whole and
    /// recorded; its intact twin is kept with its full ring.
    #[test]
    fn unresolved_edge_use_drops_the_face() {
        let body = cat(&[
            header(1),
            rec(
                VERTICES,
                &cat(&[vertex(2, 0.0), vertex(3, 1.0), vertex(4, 2.0)]),
            ),
            rec(
                EDGES,
                &cat(&[edge(5, 2, 3), edge(6, 3, 4), edge(7, 4, 2), edge(8, 2, 9)]),
            ),
            rec(FACES, &cat(&[face(10, &[5, 6, 7]), face(11, &[5, 6, 99])])),
        ]);
        let tree = rec(ENTITIES, &body);
        let n = Node::of(records(&tree, 0).unwrap()[0]).unwrap();
        let slots = Slots {
            material: HashMap::new(),
            layer: HashMap::new(),
            definition_name: HashMap::new(),
        };
        let mut diags = Vec::new();
        let b = container(&n, 0, 0, None, &slots, &mut diags).unwrap();
        let mesh = &b.run.mesh;
        assert_eq!(mesh.edges.len(), 3, "edge 8 has an unresolvable end");
        assert_eq!(mesh.faces.len(), 1);
        assert_eq!(mesh.faces[0].pid, 10);
        assert_eq!(mesh.faces[0].outer.len(), 3);
        let skipped: Vec<_> = diags
            .iter()
            .map(|d| match d {
                Diagnostic::Skipped { class, .. } => class.as_str(),
                _ => "other",
            })
            .collect();
        assert_eq!(skipped, ["edge", "face"]);
    }
}
