//! Concrete mesh materialization (Phase 3.1): turn a walked run's object
//! pool into vertices (metres), faces as ordered vertex rings (outer +
//! holes), and edges with their flags — everything a mesh importer consumes.
//!
//! Winding: each loop's ring is recovered by chaining its edge-uses through
//! shared endpoints, then oriented so the ring's Newell normal agrees with
//! the face plane's normal (the plane is decoded data; the ring order is
//! derived, so the plane is the authority).

use crate::carchive::{Child, Slot};
use crate::entity::Entity;

const METERS_PER_INCH: f64 = 0.0254;

/// One materialized geometry run: coordinates in METRES.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub vertices: Vec<[f64; 3]>,
    pub faces: Vec<MeshFace>,
    pub edges: Vec<MeshEdge>,
}

/// A face as ordered vertex-index rings. Ring orientation is
/// counter-clockwise around `normal`.
#[derive(Debug, Clone)]
pub struct MeshFace {
    /// The entity's persistent id (§4i).
    pub pid: u32,
    pub outer: Vec<u32>,
    pub holes: Vec<Vec<u32>>,
    /// Unit normal, from the face's decoded plane.
    pub normal: [f64; 3],
    /// Raw archive material indexes (`0` = none); link via
    /// `Model::material_of`.
    pub front_material: Option<u16>,
    pub back_material: Option<u16>,
    /// Per-entity hidden flag (Phase 3.4, SKP_FORMAT §4q).
    pub hidden: bool,
    /// Layer archive slot (0 = default layer); link via `Model::layer_of`.
    pub layer: u16,
    /// The face's `CFaceTextureCoords`, when it has one (§4u/§4v).
    /// A painted side WITHOUT one uses the identity placement.
    pub texture: Option<FaceTexture>,
}

/// Per-face texture placement (§4v): each side holds a row-major 3×3
/// PROJECTIVE matrix mapping texture space (inches) to the face-local frame
/// (`[t_u, t_v, 1] · K`, row-vector convention), plus the §4u pin lists.
#[derive(Debug, Clone)]
pub struct FaceTexture {
    pub front: [f64; 9],
    pub back: [f64; 9],
    /// Trailing per-side f64 triples — zero on user faces, TBD (§4v).
    pub front_extra: [f64; 3],
    pub back_extra: [f64; 3],
    /// §4u pins `(anchor_u, anchor_v, face_x, face_y)`, inches.
    pub front_pins: Vec<[f64; 4]>,
    pub back_pins: Vec<[f64; 4]>,
    /// Two u32s, semantics TBD ((1,0) affine-era, (0,1) pin-era observed).
    pub flags: [u32; 2],
}

/// Which side of a face a texture query concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Front,
    Back,
}

/// A resolved §4v UV transform for one face side: apply to mesh vertices
/// (metres) to get the exported UV pair.
#[derive(Debug, Clone)]
pub struct UvXform {
    /// Face-local frame axes (unit, world space).
    u_axis: [f64; 3],
    v_axis: [f64; 3],
    /// Inverse of the side's projective map (identity when the side has no
    /// FTC), row-major 3×3: `[x_l, y_l, 1] · K⁻¹ → texture inches`.
    kinv: [f64; 9],
    /// Material applied size, inches.
    w: f64,
    h: f64,
}

impl UvXform {
    /// UV of a world-space point in METRES (§4v formula).
    pub fn apply(&self, p_m: [f64; 3]) -> [f64; 2] {
        let p = [
            p_m[0] / METERS_PER_INCH,
            p_m[1] / METERS_PER_INCH,
            p_m[2] / METERS_PER_INCH,
        ];
        let xl = dot(p, self.u_axis);
        let yl = dot(p, self.v_axis);
        let k = &self.kinv;
        let tu = xl * k[0] + yl * k[3] + k[6];
        let tv = xl * k[1] + yl * k[4] + k[7];
        let tw = xl * k[2] + yl * k[5] + k[8];
        [tu / tw / self.w, tv / tw / self.h]
    }
}

impl MeshFace {
    /// The §4v UV transform for `side`, given the side's material applied
    /// size in inches (`Model::material_of` on the side's material slot).
    /// `None` only when the projective map is singular (never observed).
    pub fn uv_xform(&self, side: Side, applied_size_in: (f64, f64)) -> Option<UvXform> {
        let (u_axis, v_axis) = face_frame(self.normal);
        let k = match (&self.texture, side) {
            (Some(t), Side::Front) => t.front,
            (Some(t), Side::Back) => t.back,
            (None, _) => [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        };
        Some(UvXform {
            u_axis,
            v_axis,
            kinv: inv3(&k)?,
            w: applied_size_in.0,
            h: applied_size_in.1,
        })
    }
}

/// The §4v face-local texture frame, from the face's FRONT plane normal
/// (both sides share it): horizontal faces pin `v = +Y`; otherwise `v` is
/// the world Z axis projected onto the plane; `u = v × n`.
fn face_frame(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let v = if n[2].abs() > 1.0 - 1e-9 {
        [0.0, 1.0, 0.0]
    } else {
        normalize([-n[2] * n[0], -n[2] * n[1], 1.0 - n[2] * n[2]])
    };
    let u = [
        v[1] * n[2] - v[2] * n[1],
        v[2] * n[0] - v[0] * n[2],
        v[0] * n[1] - v[1] * n[0],
    ];
    (u, v)
}

/// Row-major 3×3 inverse; `None` when singular.
fn inv3(k: &[f64; 9]) -> Option<[f64; 9]> {
    let [a, b, c, d, e, f, g, h, i] = *k;
    let ca = e * i - f * h;
    let cb = -(d * i - f * g);
    let cc = d * h - e * g;
    let det = a * ca + b * cb + c * cc;
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    Some([
        ca / det,
        -(b * i - c * h) / det,
        (b * f - c * e) / det,
        cb / det,
        (a * i - c * g) / det,
        -(a * f - c * d) / det,
        cc / det,
        -(a * h - b * g) / det,
        (a * e - b * d) / det,
    ])
}

#[derive(Debug, Clone, Copy)]
pub struct MeshEdge {
    /// The entity's persistent id (§4i).
    pub pid: u32,
    pub v0: u32,
    pub v1: u32,
    pub soft: bool,
    pub smooth: bool,
    /// Per-entity hidden flag (Phase 3.4, SKP_FORMAT §4q).
    pub hidden: bool,
    /// Layer archive slot (0 = default layer); link via `Model::layer_of`.
    pub layer: u16,
}

/// Build the mesh from a run's object pool. `base` is the resolver's voted
/// back-ref base: a back-ref `r` targets `map[r - base + 1]`.
pub(crate) fn build(map: &[Slot], base: Option<i64>) -> Mesh {
    build_range(map, base, 0, map.len())
}

/// [`build`] restricted to the map slots `lo..hi` — the continuous walk's
/// per-definition materialization (SKP_FORMAT §4s: ONE global map, each
/// definition's subtree occupying a contiguous slot range). Back-refs still
/// dereference against the WHOLE map (with the continuous path's `base = 1`
/// a back-ref `r` IS `map[r]`), but only in-range vertices/edges/faces
/// enter the mesh.
pub(crate) fn build_range(map: &[Slot], base: Option<i64>, lo: usize, hi: usize) -> Mesh {
    let deref = |c: &Child| -> Option<usize> {
        match c {
            Child::Obj(i) => Some(*i),
            Child::Ref(r) => {
                let j = *r as i64 - base?;
                let idx = usize::try_from(j).ok()? + 1;
                (idx < map.len()).then_some(idx)
            }
            Child::Null => None,
        }
    };
    let hi = hi.min(map.len());
    let lo = lo.min(hi);

    // Vertices, in pool order.
    let mut mesh = Mesh::default();
    let mut vert_of_slot: std::collections::HashMap<usize, u32> = Default::default();
    for (i, slot) in map.iter().enumerate().take(hi).skip(lo) {
        if let Slot::Object(Entity::Vertex { x, y, z, .. }) = slot {
            vert_of_slot.insert(i, mesh.vertices.len() as u32);
            mesh.vertices.push([
                x * METERS_PER_INCH,
                y * METERS_PER_INCH,
                z * METERS_PER_INCH,
            ]);
        }
    }
    let vert_of_child = |c: &Child| -> Option<u32> { vert_of_slot.get(&deref(c)?).copied() };

    // Edges, in pool order.
    for slot in map.iter().take(hi).skip(lo) {
        if let Slot::Object(Entity::Edge {
            pid,
            soft,
            smooth,
            hidden,
            layer,
            v0,
            v1,
            ..
        }) = slot
        {
            if let (Some(a), Some(b)) = (vert_of_child(v0), vert_of_child(v1)) {
                mesh.edges.push(MeshEdge {
                    pid: *pid,
                    v0: a,
                    v1: b,
                    soft: *soft,
                    smooth: *smooth,
                    hidden: *hidden,
                    layer: *layer,
                });
            }
        }
    }

    // Faces: chain each loop's edge-uses into a ring.
    let ring_of_loop = |loop_child: &Child| -> Option<Vec<u32>> {
        let Slot::Object(Entity::Loop { edge_uses }) = &map[deref(loop_child)?] else {
            return None;
        };
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        for eu in edge_uses {
            let Slot::Object(Entity::EdgeUse { edge, .. }) = &map[deref(eu)?] else {
                return None;
            };
            let Slot::Object(Entity::Edge { v0, v1, .. }) = &map[deref(edge)?] else {
                return None;
            };
            pairs.push((vert_of_child(v0)?, vert_of_child(v1)?));
        }
        if pairs.len() < 3 {
            return None;
        }
        // Orient the first pair so it chains into the second; then orient
        // each pair to continue from the previous end.
        let (s0, e0) = pairs[0];
        let (s1, e1) = pairs[1];
        let mut ring = Vec::with_capacity(pairs.len());
        let (mut prev_end, first) = if e0 == s1 || e0 == e1 {
            (e0, s0)
        } else if s0 == s1 || s0 == e1 {
            (s0, e0)
        } else {
            return None; // broken chain
        };
        ring.push(first);
        for &(a, b) in &pairs[1..] {
            let next = if a == prev_end {
                b
            } else if b == prev_end {
                a
            } else {
                return None; // broken chain
            };
            ring.push(prev_end);
            prev_end = next;
        }
        (prev_end == first).then_some(ring)
    };

    // The face's FTC hangs off its attribute container (§4s preamble):
    // deref the pointer, then take the container's CFaceTextureCoords child.
    let texture_of = |attrs: &Child| -> Option<FaceTexture> {
        let Slot::Object(Entity::AttrContainer { children }) = &map[deref(attrs)?] else {
            return None;
        };
        children.iter().find_map(|c| {
            let Slot::Object(Entity::FaceTextureCoords {
                front,
                front_extra,
                back,
                back_extra,
                front_pins,
                back_pins,
                flags,
                ..
            }) = &map[deref(c)?]
            else {
                return None;
            };
            Some(FaceTexture {
                front: *front,
                back: *back,
                front_extra: *front_extra,
                back_extra: *back_extra,
                front_pins: front_pins.clone(),
                back_pins: back_pins.clone(),
                flags: *flags,
            })
        })
    };

    for slot in map.iter().take(hi).skip(lo) {
        let Slot::Object(Entity::Face {
            pid,
            front_material,
            back_material,
            hidden,
            layer,
            plane,
            loops,
            attrs,
            ..
        }) = slot
        else {
            continue;
        };
        let mut rings: Vec<Vec<u32>> = loops.iter().filter_map(&ring_of_loop).collect();
        if rings.len() != loops.len() || rings.is_empty() {
            continue; // unresolved loop — leave the face out rather than guess
        }
        let plane_n = normalize([plane[0], plane[1], plane[2]]);
        for ring in &mut rings {
            if dot(newell(ring, &mesh.vertices), plane_n) < 0.0 {
                ring.reverse();
            }
        }
        // The outer ring encloses the largest area; holes are the rest, in
        // serialization order.
        let outer_at = rings
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                area2(a, &mesh.vertices)
                    .partial_cmp(&area2(b, &mesh.vertices))
                    .unwrap()
            })
            .map(|(i, _)| i)
            .unwrap();
        let outer = rings.remove(outer_at);
        mesh.faces.push(MeshFace {
            pid: *pid,
            outer,
            holes: rings,
            normal: plane_n,
            front_material: (*front_material != 0).then_some(*front_material),
            back_material: (*back_material != 0).then_some(*back_material),
            hidden: *hidden,
            layer: *layer,
            texture: texture_of(attrs),
        });
    }
    mesh
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let m = dot(v, v).sqrt();
    if m == 0.0 {
        v
    } else {
        [v[0] / m, v[1] / m, v[2] / m]
    }
}

/// Newell's method: robust polygon normal from an ordered ring.
fn newell(ring: &[u32], verts: &[[f64; 3]]) -> [f64; 3] {
    let mut n = [0.0f64; 3];
    for k in 0..ring.len() {
        let p = verts[ring[k] as usize];
        let q = verts[ring[(k + 1) % ring.len()] as usize];
        n[0] += (p[1] - q[1]) * (p[2] + q[2]);
        n[1] += (p[2] - q[2]) * (p[0] + q[0]);
        n[2] += (p[0] - q[0]) * (p[1] + q[1]);
    }
    n
}

/// Squared magnitude of the Newell normal — proportional to enclosed area,
/// orientation-independent (used to pick the outer ring).
fn area2(ring: &[u32], verts: &[[f64; 3]]) -> f64 {
    let n = newell(ring, verts);
    dot(n, n)
}
