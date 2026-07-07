//! Phase 6.1/6.2 — the full-model equivalence harness.
//!
//! 6.1 (house.skp vs house.dae): the whole model, compared in WORLD space —
//! vertex positions, face rings (canonical, orientation-free), materials
//! (every exported diffuse colour is an extracted material), per-corner UVs
//! on every textured face (the §4v formula under composed instance
//! transforms, D≠0 planes and arbitrary wall normals), and the scene
//! hierarchy's named roots.
//!
//! 6.2 (THE goal test, verbatim): house-plus.skp — house.skp with two
//! dimensions, a leader text, two scenes and an imported image added —
//! must decode to the SAME world-space geometry, with the extras
//! surfacing only as counts/diagnostics, never as geometry.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

fn corpus(name: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    std::fs::read(p).unwrap()
}

// ---- shared linear algebra (row layout matches openskp::Node::world) ----

type M4 = [f64; 16];
const ID4: M4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn mul(a: &M4, b: &M4) -> M4 {
    let mut o = [0.0; 16];
    for r in 0..4 {
        for c in 0..4 {
            o[r * 4 + c] = (0..4).map(|k| a[r * 4 + k] * b[k * 4 + c]).sum();
        }
    }
    o
}

fn apply(m: &M4, p: [f64; 3]) -> [f64; 3] {
    [
        m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
        m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
        m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
    ]
}

fn q(p: [f64; 3]) -> [i64; 3] {
    // 1 µm grid: dae prints ~9 significant digits and house spans ~10 m,
    // so composed-transform noise sits well below this.
    [
        (p[0] * 1e6).round() as i64,
        (p[1] * 1e6).round() as i64,
        (p[2] * 1e6).round() as i64,
    ]
}

fn canon_full(ring: &[[i64; 3]]) -> Vec<[i64; 3]> {
    let rot = |r: &[[i64; 3]]| -> Vec<[i64; 3]> {
        (0..r.len())
            .map(|s| r.iter().cycle().skip(s).take(r.len()).copied().collect())
            .min()
            .unwrap()
    };
    let rev: Vec<[i64; 3]> = ring.iter().rev().copied().collect();
    rot(ring).min(rot(&rev))
}

// ---- COLLADA reading (SketchUp-export subset, tests only) ----

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!("{name}=\"");
    let i = tag.find(&pat)? + pat.len();
    let j = tag[i..].find('"')? + i;
    Some(&tag[i..j])
}

fn child_nodes(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = s[from..].find("<node") {
        let start = from + i;
        let head_end = s[start..].find('>').unwrap() + start;
        let mut depth = 1;
        let mut p = head_end;
        while depth > 0 {
            let open = s[p + 1..].find("<node").map(|k| p + 1 + k);
            let close = s[p + 1..].find("</node>").map(|k| p + 1 + k);
            match (open, close) {
                (Some(o), Some(c)) if o < c => {
                    depth += 1;
                    p = s[o..].find('>').unwrap() + o;
                }
                (_, Some(c)) => {
                    depth -= 1;
                    p = c + 6;
                }
                _ => panic!("unbalanced <node>"),
            }
        }
        out.push((
            s[start..head_end].to_string(),
            s[head_end + 1..p - 6].to_string(),
        ));
        from = p;
    }
    out
}

fn own_content(body: &str) -> String {
    let mut own = String::new();
    let mut rest = body;
    while let Some(i) = rest.find("<node") {
        own.push_str(&rest[..i]);
        let mut depth = 1;
        let head_end = rest[i..].find('>').unwrap() + i;
        let mut p = head_end;
        while depth > 0 {
            let open = rest[p + 1..].find("<node").map(|k| p + 1 + k);
            let close = rest[p + 1..].find("</node>").map(|k| p + 1 + k);
            match (open, close) {
                (Some(o), Some(c)) if o < c => {
                    depth += 1;
                    p = rest[o..].find('>').unwrap() + o;
                }
                (_, Some(c)) => {
                    depth -= 1;
                    p = c + 6;
                }
                _ => panic!("unbalanced"),
            }
        }
        rest = &rest[p..];
    }
    own.push_str(rest);
    own
}

/// One geometry's decoded mesh, positions in FILE UNITS (geometry-local).
#[derive(Default)]
struct DaeGeom {
    pos: Vec<[f64; 3]>,
    /// All face rings as position indices (polylist + polygons/ph outers;
    /// holes contribute their corners to the vertex set via `pos` anyway).
    rings: Vec<Vec<usize>>,
    /// Textured faces: (position-index ring, per-corner UV).
    textured: Vec<(Vec<usize>, Vec<[f64; 2]>)>,
}

struct Dae {
    unit: f64,
    geoms: HashMap<String, DaeGeom>,
    /// (world matrix [file units scaled to m], geometry id) leaves.
    leaves: Vec<(M4, String)>,
    /// Non-default effect diffuse RGBAs.
    diffuse: Vec<[u8; 4]>,
    /// name attrs of the visual scene's direct child nodes.
    root_names: Vec<String>,
}

fn parse_dae(name: &str) -> Dae {
    let doc = String::from_utf8(corpus(name)).unwrap();
    let ui = doc.find("<unit").unwrap();
    let unit: f64 = attr(&doc[ui..ui + 80], "meter").unwrap().parse().unwrap();

    let blocks = |s: &str, tag: &str| -> Vec<(String, String)> {
        let open = format!("<{tag}");
        let close = format!("</{tag}>");
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(i) = s[from..].find(&open) {
            let start = from + i;
            let head_end = s[start..].find('>').unwrap() + start;
            if s.as_bytes()[head_end - 1] == b'/' {
                out.push((s[start..head_end].to_string(), String::new()));
                from = head_end;
                continue;
            }
            let body_end = s[head_end..].find(&close).unwrap() + head_end;
            out.push((
                s[start..head_end].to_string(),
                s[head_end + 1..body_end].to_string(),
            ));
            from = body_end + close.len();
        }
        out
    };

    // geometries
    let mut geoms = HashMap::new();
    for (gh, gb) in blocks(&doc, "geometry") {
        let gid = attr(&gh, "id").unwrap().to_string();
        let mut g = DaeGeom::default();
        // sources: id → floats
        let mut sources: HashMap<String, Vec<f64>> = HashMap::new();
        for (sh, sb) in blocks(&gb, "source") {
            if let Some((_, fb)) = blocks(&sb, "float_array").first() {
                sources.insert(
                    attr(&sh, "id").unwrap().to_string(),
                    fb.split_ascii_whitespace()
                        .map(|t| t.parse().unwrap())
                        .collect(),
                );
            }
        }
        // vertices id → POSITION source
        let mut vpos: HashMap<String, String> = HashMap::new();
        for (vh, vb) in blocks(&gb, "vertices") {
            for (ih, _) in blocks(&vb, "input") {
                if attr(&ih, "semantic") == Some("POSITION") {
                    vpos.insert(
                        attr(&vh, "id").unwrap().to_string(),
                        attr(&ih, "source")
                            .unwrap()
                            .trim_start_matches('#')
                            .to_string(),
                    );
                }
            }
        }
        // one position source per SketchUp geometry
        let pos_src = vpos.values().next().expect("geometry without POSITION");
        g.pos = sources[pos_src]
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect();

        for (_, pb) in blocks(&gb, "polylist") {
            let mut voff = 0usize;
            let mut toff = None;
            let mut tsrc = None;
            let mut stride = 0usize;
            for (ih, _) in blocks(&pb, "input") {
                let off: usize = attr(&ih, "offset").unwrap().parse().unwrap();
                stride = stride.max(off + 1);
                match attr(&ih, "semantic").unwrap() {
                    "VERTEX" => voff = off,
                    "TEXCOORD" => {
                        toff = Some(off);
                        tsrc = attr(&ih, "source").map(|s| s.trim_start_matches('#').to_string());
                    }
                    _ => {}
                }
            }
            let vcount: Vec<usize> = blocks(&pb, "vcount")[0]
                .1
                .split_ascii_whitespace()
                .map(|t| t.parse().unwrap())
                .collect();
            let p: Vec<usize> = blocks(&pb, "p")[0]
                .1
                .split_ascii_whitespace()
                .map(|t| t.parse().unwrap())
                .collect();
            let uvs = tsrc.map(|s| sources[&s].clone());
            let mut at = 0;
            for n in vcount {
                let ring: Vec<usize> = (0..n).map(|k| p[(at + k) * stride + voff]).collect();
                if let (Some(toff), Some(uvs)) = (toff, uvs.as_ref()) {
                    let ring_uv: Vec<[f64; 2]> = (0..n)
                        .map(|k| {
                            let ti = p[(at + k) * stride + toff];
                            [uvs[ti * 2], uvs[ti * 2 + 1]]
                        })
                        .collect();
                    g.textured.push((ring.clone(), ring_uv));
                }
                g.rings.push(ring);
                at += n;
            }
        }
        // hole-bearing faces: <polygons><ph><p>outer</p><h>…</h></ph>
        for (_, gb2) in blocks(&gb, "polygons") {
            for (_, ph) in blocks(&gb2, "ph") {
                if let Some((_, pb)) = blocks(&ph, "p").first() {
                    g.rings.push(
                        pb.split_ascii_whitespace()
                            .map(|t| t.parse().unwrap())
                            .collect(),
                    );
                }
            }
        }
        geoms.insert(gid, g);
    }

    // effects: diffuse colours
    let defaults: [[u8; 4]; 3] = [[255, 255, 255, 255], [164, 178, 187, 255], [0, 0, 0, 255]];
    let mut diffuse = Vec::new();
    for (_, eb) in blocks(&doc, "effect") {
        for (_, db) in blocks(&eb, "diffuse") {
            for (_, cb) in blocks(&db, "color") {
                let v: Vec<f64> = cb
                    .split_ascii_whitespace()
                    .map(|t| t.parse().unwrap())
                    .collect();
                let rgba = [
                    (v[0] * 255.0).round() as u8,
                    (v[1] * 255.0).round() as u8,
                    (v[2] * 255.0).round() as u8,
                    (v[3] * 255.0).round() as u8,
                ];
                if !defaults.contains(&rgba) {
                    diffuse.push(rgba);
                }
            }
        }
    }

    // library_nodes + visual scene → leaves
    let mut lib = HashMap::new();
    if let Some(li) = doc.find("<library_nodes>") {
        let le = doc[li..].find("</library_nodes>").unwrap() + li;
        for (h, b) in child_nodes(&doc[li..le]) {
            if let Some(id) = attr(&h, "id") {
                lib.insert(id.to_string(), (h.clone(), b.clone()));
            }
        }
    }
    let si = doc.find("<library_visual_scenes>").unwrap();
    let se = doc[si..].find("</library_visual_scenes>").unwrap() + si;
    let mut leaves = Vec::new();
    let mut root_names = Vec::new();
    fn walk(
        body: &str,
        parent: &M4,
        lib: &HashMap<String, (String, String)>,
        unit: f64,
        out: &mut Vec<(M4, String)>,
    ) {
        let own = own_content(body);
        if own.contains("<instance_camera") {
            return;
        }
        let local: M4 = match own.find("<matrix>") {
            Some(i) => {
                let j = own[i..].find("</matrix>").unwrap() + i;
                let vals: Vec<f64> = own[i + 8..j]
                    .split_ascii_whitespace()
                    .map(|t| t.parse().unwrap())
                    .collect();
                let mut m: M4 = vals.try_into().unwrap();
                for k in [3, 7, 11] {
                    m[k] *= unit;
                }
                m
            }
            None => ID4,
        };
        let world = mul(parent, &local);
        let mut from = 0;
        while let Some(i) = own[from..].find("<instance_geometry") {
            let start = from + i;
            let tag_end = own[start..].find('>').unwrap() + start;
            let url = attr(&own[start..tag_end], "url")
                .unwrap()
                .trim_start_matches('#');
            out.push((world, url.to_string()));
            from = tag_end;
        }
        let mut from = 0;
        while let Some(i) = own[from..].find("<instance_node") {
            let start = from + i;
            let tag_end = own[start..].find("/>").unwrap() + start;
            let url = attr(&own[start..tag_end], "url")
                .unwrap()
                .trim_start_matches('#');
            walk(&lib[url].1, &world, lib, unit, out);
            from = tag_end;
        }
        for (_, b) in child_nodes(body) {
            walk(&b, &world, lib, unit, out);
        }
    }
    // SketchUp wraps the whole scene in one "SketchUp" <node>; the model's
    // top-level entities are ITS children (camera nodes excluded).
    for (h, b) in child_nodes(&doc[si..se]) {
        let inner = if attr(&h, "name") == Some("SketchUp") {
            child_nodes(&b)
        } else {
            vec![(h.clone(), b.clone())]
        };
        for (ih, ib) in &inner {
            if own_content(ib).contains("<instance_camera") {
                continue;
            }
            if let Some(n) = attr(ih, "name") {
                root_names.push(n.to_string());
            }
        }
        walk(&b, &ID4, &lib, unit, &mut leaves);
    }

    Dae {
        unit,
        geoms,
        leaves,
        diffuse,
        root_names,
    }
}

// ---- our side: world-space leaves ----

/// (world matrix, run index, inherited material slot) for every
/// geometry-bearing placement: the scene tree's nodes plus the root run.
fn our_leaves(m: &openskp::Model) -> Vec<(M4, usize, u16)> {
    let mut out = Vec::new();
    fn rec(n: &openskp::Node, out: &mut Vec<(M4, usize, u16)>) {
        if let Some(ri) = n.run {
            out.push((n.world, ri, n.material));
        }
        for c in &n.children {
            rec(c, out);
        }
    }
    for n in m.scene() {
        rec(&n, &mut out);
    }
    // the root run: def_index None, file span empty (continuous path)
    let root = m
        .geometry
        .iter()
        .position(|r| r.def_index.is_none())
        .expect("root run");
    out.push((ID4, root, 0));
    out
}

/// World-space vertex + canonical face-ring sets (both sides reduce to this).
#[derive(PartialEq)]
struct WorldMesh {
    verts: BTreeSet<[i64; 3]>,
    rings: BTreeSet<Vec<[i64; 3]>>,
}

/// Canonical vertex unification: house coordinates sit on a 1/64-inch
/// lattice whose µm values end in .5, so the same world point can round
/// to keys 1 µm apart — and the dae itself prints BOTH variants (one per
/// adjacent geometry). Built over the UNION of both sides' quantized
/// vertices: each key maps to the representative of its ≤1 µm
/// (Chebyshev) neighborhood, assigned in ascending key order.
struct Snap(HashMap<[i64; 3], [i64; 3]>);

impl Snap {
    fn of(union: &BTreeSet<[i64; 3]>) -> Snap {
        let mut rep: HashMap<[i64; 3], [i64; 3]> = HashMap::new();
        for &k in union {
            let mut r = k;
            'search: for dx in -1..=1i64 {
                for dy in -1..=1i64 {
                    for dz in -1..=1i64 {
                        let n = [k[0] + dx, k[1] + dy, k[2] + dz];
                        if let Some(&nr) = rep.get(&n) {
                            r = nr;
                            break 'search;
                        }
                    }
                }
            }
            rep.insert(k, r);
        }
        Snap(rep)
    }

    fn snap(&self, k: [i64; 3]) -> [i64; 3] {
        self.0.get(&k).copied().unwrap_or(k)
    }
}

/// Raw (unsnapped) quantized world vertex set — input to [`Snap::of`].
fn our_raw_verts(m: &openskp::Model) -> BTreeSet<[i64; 3]> {
    let mut verts = BTreeSet::new();
    for (world, ri, _) in our_leaves(m) {
        for &p in &m.geometry[ri].mesh.vertices {
            verts.insert(q(apply(&world, p)));
        }
    }
    verts
}

fn our_world_mesh(m: &openskp::Model, snap: &Snap) -> WorldMesh {
    let mut verts = BTreeSet::new();
    let mut rings = BTreeSet::new();
    for (world, ri, _) in our_leaves(m) {
        let mesh = &m.geometry[ri].mesh;
        for &p in &mesh.vertices {
            verts.insert(snap.snap(q(apply(&world, p))));
        }
        for f in &mesh.faces {
            let ring: Vec<[i64; 3]> = f
                .outer
                .iter()
                .map(|&vi| snap.snap(q(apply(&world, mesh.vertices[vi as usize]))))
                .collect();
            rings.insert(canon_full(&ring));
        }
    }
    WorldMesh { verts, rings }
}

fn dae_raw_verts(d: &Dae) -> BTreeSet<[i64; 3]> {
    let mut verts = BTreeSet::new();
    for (world, gid) in &d.leaves {
        for &p in &d.geoms[gid].pos {
            verts.insert(q(apply(
                world,
                [p[0] * d.unit, p[1] * d.unit, p[2] * d.unit],
            )));
        }
    }
    verts
}

fn dae_world_mesh(d: &Dae, snap: &Snap) -> WorldMesh {
    let mut verts = BTreeSet::new();
    let mut rings = BTreeSet::new();
    for (world, gid) in &d.leaves {
        let g = &d.geoms[gid];
        let wp = |i: usize| -> [i64; 3] {
            let p = g.pos[i];
            snap.snap(q(apply(
                world,
                [p[0] * d.unit, p[1] * d.unit, p[2] * d.unit],
            )))
        };
        for i in 0..g.pos.len() {
            verts.insert(wp(i));
        }
        for ring in &g.rings {
            rings.insert(canon_full(&ring.iter().map(|&i| wp(i)).collect::<Vec<_>>()));
        }
    }
    WorldMesh { verts, rings }
}

// ---- 6.1: house.skp vs house.dae ----

#[test]
fn house_world_mesh_matches_dae() {
    let m = openskp::Model::parse(&corpus("house.skp")).unwrap();
    assert!(
        m.diagnostics
            .iter()
            .any(|d| matches!(d, openskp::Diagnostic::ContinuousWalk { .. })),
        "house must parse on the continuous path"
    );
    let dae = parse_dae("house.dae");
    let mut union = our_raw_verts(&m);
    union.extend(dae_raw_verts(&dae));
    let snap = Snap::of(&union);
    let theirs = dae_world_mesh(&dae, &snap);
    let ours = our_world_mesh(&m, &snap);
    let only_ours: Vec<_> = ours.verts.difference(&theirs.verts).collect();
    let only_dae: Vec<_> = theirs.verts.difference(&ours.verts).collect();
    assert!(
        only_ours.is_empty() && only_dae.is_empty(),
        "world vertex sets differ: only-ours {only_ours:?} only-dae {only_dae:?}"
    );
    let ring_only_ours: Vec<_> = ours.rings.difference(&theirs.rings).collect();
    let ring_only_dae: Vec<_> = theirs.rings.difference(&ours.rings).collect();
    assert!(
        ring_only_ours.is_empty() && ring_only_dae.is_empty(),
        "world face-ring sets differ: only-ours {ring_only_ours:?} only-dae {ring_only_dae:?}"
    );
}

#[test]
fn house_materials_cover_dae_diffuse() {
    let m = openskp::Model::parse(&corpus("house.skp")).unwrap();
    let dae = parse_dae("house.dae");
    let ours: BTreeSet<[u8; 3]> = m
        .materials
        .iter()
        .filter_map(|mat| match mat {
            openskp::Material::Solid { rgba, .. } => Some([rgba[0], rgba[1], rgba[2]]),
            openskp::Material::Textured {
                avg_rgba: Some(c), ..
            } => Some([c[0], c[1], c[2]]),
            _ => None,
        })
        .collect();
    for c in &dae.diffuse {
        assert!(
            ours.contains(&[c[0], c[1], c[2]]),
            "dae diffuse {c:?} not among extracted materials {ours:?}"
        );
    }
}

#[test]
fn house_scene_roots_match_dae_nodes() {
    let m = openskp::Model::parse(&corpus("house.skp")).unwrap();
    let dae = parse_dae("house.dae");
    let scene = m.scene();
    assert_eq!(scene.len(), dae.root_names.len(), "scene root count");
    // named roots: the exporter replaces spaces with underscores and
    // auto-names anonymous instances (instance_N) — compare the named ones.
    let ours: BTreeSet<String> = m
        .instances
        .iter()
        .filter_map(|i| i.name.clone())
        .filter(|n| !n.is_empty())
        .map(|n| n.replace(' ', "_"))
        .collect();
    for n in dae
        .root_names
        .iter()
        .filter(|n| !n.starts_with("instance_"))
    {
        assert!(ours.contains(n.as_str()), "dae root {n:?} missing from skp");
    }
}

#[test]
fn house_uvs_match_dae_texcoords() {
    let m = openskp::Model::parse(&corpus("house.skp")).unwrap();
    let dae = parse_dae("house.dae");

    let mut union = our_raw_verts(&m);
    union.extend(dae_raw_verts(&dae));
    let snap = Snap::of(&union);

    // our faces, world-keyed: canon ring → (leaf world, run, face, inherited)
    let mut ours: HashMap<Vec<[i64; 3]>, (M4, usize, usize, u16)> = HashMap::new();
    for (world, ri, inherited) in our_leaves(&m) {
        let mesh = &m.geometry[ri].mesh;
        for (fi, f) in mesh.faces.iter().enumerate() {
            let ring: Vec<[i64; 3]> = f
                .outer
                .iter()
                .map(|&vi| snap.snap(q(apply(&world, mesh.vertices[vi as usize]))))
                .collect();
            ours.insert(canon_full(&ring), (world, ri, fi, inherited));
        }
    }

    let mut checked = 0;
    for (world, gid) in &dae.leaves {
        let g = &dae.geoms[gid];
        for (ring, ring_uv) in &g.textured {
            let wring: Vec<([i64; 3], [f64; 2])> = ring
                .iter()
                .zip(ring_uv)
                .map(|(&i, &uv)| {
                    let p = g.pos[i];
                    (
                        snap.snap(q(apply(
                            world,
                            [p[0] * dae.unit, p[1] * dae.unit, p[2] * dae.unit],
                        ))),
                        uv,
                    )
                })
                .collect();
            let key = canon_full(&wring.iter().map(|(p, _)| *p).collect::<Vec<_>>());
            let Some(&(oworld, ri, fi, inherited)) = ours.get(&key) else {
                panic!("dae textured face has no skp match ({gid})");
            };
            let mesh = &m.geometry[ri].mesh;
            let f = &mesh.faces[fi];
            // world corner → our LOCAL corner (UVs are defined in def space)
            let local_of: HashMap<[i64; 3], [f64; 3]> = f
                .outer
                .iter()
                .map(|&vi| {
                    let p = mesh.vertices[vi as usize];
                    (snap.snap(q(apply(&oworld, p))), p)
                })
                .collect();
            // pick the side whose predicted UVs fit best; require < tol.
            // A default-material side renders the INHERITED instance
            // material (§4q drawbase matref down the node path).
            let mut best = f64::INFINITY;
            for (mat, side) in [
                (f.front_material, openskp::Side::Front),
                (f.back_material, openskp::Side::Back),
            ] {
                let slot = mat.unwrap_or(inherited);
                let Some(size) = (slot != 0)
                    .then_some(slot)
                    .and_then(|slot| m.applied_size_of(slot))
                else {
                    continue;
                };
                let x = f.uv_xform(side, size).unwrap();
                let mut err = 0.0f64;
                for (wp, uv) in &wring {
                    let local = local_of[wp];
                    let got = x.apply(local);
                    err = err.max((got[0] - uv[0]).abs()).max((got[1] - uv[1]).abs());
                }
                best = best.min(err);
            }
            assert!(
                best < 1e-4,
                "textured face pid {} ({gid}): best side uv err {best}",
                f.pid
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 68, "house.dae carries 68 textured polylist faces");
}

// ---- 6.2: THE goal test ----

#[test]
fn house_plus_geometry_equals_house() {
    let plus = openskp::Model::parse(&corpus("house-plus.skp")).unwrap();
    let base = openskp::Model::parse(&corpus("house.skp")).unwrap();
    for (m, name) in [(&plus, "house-plus"), (&base, "house")] {
        assert!(
            m.diagnostics
                .iter()
                .any(|d| matches!(d, openskp::Diagnostic::ContinuousWalk { .. })),
            "{name} must parse on the continuous path"
        );
    }

    // Verbatim: the DRAWING is unchanged — identical world-space geometry.
    let mut union = our_raw_verts(&base);
    union.extend(our_raw_verts(&plus));
    let snap = Snap::of(&union);
    let b = our_world_mesh(&base, &snap);
    let a = our_world_mesh(&plus, &snap);
    assert_eq!(
        a.verts,
        b.verts,
        "house-plus world vertices differ from house ({} vs {})",
        a.verts.len(),
        b.verts.len()
    );
    assert_eq!(
        a.rings,
        b.rings,
        "house-plus world face rings differ from house ({} vs {})",
        a.rings.len(),
        b.rings.len()
    );

    // The extras are all VISIBLE, and only as non-geometry surfaces.
    let count = |m: &openskp::Model, f: fn(&openskp::Topology) -> usize| -> usize {
        m.geometry.iter().map(|r| f(&r.topology)).sum()
    };
    assert_eq!(count(&base, |t| t.dimensions), 0);
    assert_eq!(count(&base, |t| t.texts), 0);
    assert_eq!(count(&base, |t| t.images_placed), 0);
    assert_eq!(count(&plus, |t| t.dimensions), 2, "the two dimensions");
    assert_eq!(count(&plus, |t| t.texts), 1, "the leader text");
    assert_eq!(count(&plus, |t| t.images_placed), 1, "the imported image");
    assert_eq!(base.scenes.len(), 0);
    assert_eq!(plus.scenes.len(), 2, "the two authored scenes");
    assert_eq!(base.materials.len(), 9, "house materials (§4s addendum 2)");
    assert_eq!(
        plus.materials.len(),
        11,
        "house-plus adds the dimension auto-material and the image material"
    );
    assert_eq!(
        plus.images.len(),
        base.images.len() + 1,
        "the imported picture's CDib"
    );
}
