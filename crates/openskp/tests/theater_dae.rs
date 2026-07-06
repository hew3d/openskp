//! The third-party stress benchmark: theater-2017.skp (10.7 MB, 752 defs,
//! 72k edges — a real, multi-year production model) vs its COLLADA export.
//!
//! COLLADA exports are WYSIWYG: hidden entities, hidden subtrees and
//! entities on hidden layers are dropped, so our side filters by the §4q
//! hidden flags + layer visibility before comparing. Comparison mirrors
//! tests/equivalence.rs (world vertex/ring sets, per-corner UVs on every
//! textured face, materials) with union-side 1 µm vertex unification.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

fn corpus(name: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    std::fs::read(p).unwrap()
}

// ---- linear algebra (row layout matches openskp::Node::world) ----

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

// ---- COLLADA reading (same subset as tests/equivalence.rs) ----

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

fn blocks(s: &str, tag: &str) -> Vec<(String, String)> {
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
}

#[derive(Default)]
struct DaeGeom {
    pos: Vec<[f64; 3]>,
    rings: Vec<Vec<usize>>,
    textured: Vec<(Vec<usize>, Vec<[f64; 2]>)>,
}

struct Dae {
    unit: f64,
    geoms: HashMap<String, DaeGeom>,
    leaves: Vec<(M4, String)>,
    diffuse: Vec<[u8; 4]>,
}

fn parse_dae(name: &str) -> Dae {
    let doc = String::from_utf8(corpus(name)).unwrap();
    let ui = doc.find("<unit").unwrap();
    let unit: f64 = attr(&doc[ui..ui + 80], "meter").unwrap().parse().unwrap();

    let mut geoms = HashMap::new();
    for (gh, gb) in blocks(&doc, "geometry") {
        let gid = attr(&gh, "id").unwrap().to_string();
        let mut g = DaeGeom::default();
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
        let mut vpos: HashMap<String, String> = HashMap::new();
        for (vh, vb) in blocks(&gb, "vertices") {
            for (ih, _) in blocks(&vb, "input") {
                if attr(&ih, "semantic") == Some("POSITION") {
                    vpos.insert(
                        attr(&vh, "id").unwrap().to_string(),
                        attr(&ih, "source").unwrap().trim_start_matches('#').to_string(),
                    );
                }
            }
        }
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
        // <polygons> (hole-bearing faces) carry offset inputs exactly like
        // <polylist> — the <ph>/<p>/<h> index arrays are STRIDED (theater's
        // textured walls: VERTEX offset 0 + TEXCOORD offset 1). Reading
        // them raw interleaves vertex and uv indices.
        for (_, gb2) in blocks(&gb, "polygons") {
            let mut voff = 0usize;
            let mut toff = None;
            let mut tsrc = None;
            let mut stride = 1usize;
            for (ih, _) in blocks(&gb2, "input") {
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
            let uvs = tsrc.map(|s| sources[&s].clone());
            for (_, ph) in blocks(&gb2, "ph") {
                if let Some((_, pb)) = blocks(&ph, "p").first() {
                    let raw: Vec<usize> = pb
                        .split_ascii_whitespace()
                        .map(|t| t.parse().unwrap())
                        .collect();
                    let n = raw.len() / stride;
                    let ring: Vec<usize> = (0..n).map(|k| raw[k * stride + voff]).collect();
                    if let (Some(toff), Some(uvs)) = (toff, uvs.as_ref()) {
                        let ring_uv: Vec<[f64; 2]> = (0..n)
                            .map(|k| {
                                let ti = raw[k * stride + toff];
                                [uvs[ti * 2], uvs[ti * 2 + 1]]
                            })
                            .collect();
                        g.textured.push((ring.clone(), ring_uv));
                    }
                    g.rings.push(ring);
                }
            }
        }
        geoms.insert(gid, g);
    }

    let defaults: [[u8; 4]; 3] = [
        [255, 255, 255, 255],
        [164, 178, 187, 255],
        [0, 0, 0, 255],
    ];
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
            let url = attr(&own[start..tag_end], "url").unwrap().trim_start_matches('#');
            out.push((world, url.to_string()));
            from = tag_end;
        }
        let mut from = 0;
        while let Some(i) = own[from..].find("<instance_node") {
            let start = from + i;
            let tag_end = own[start..].find("/>").unwrap() + start;
            let url = attr(&own[start..tag_end], "url").unwrap().trim_start_matches('#');
            walk(&lib[url].1, &world, lib, unit, out);
            from = tag_end;
        }
        for (_, b) in child_nodes(body) {
            walk(&b, &world, lib, unit, out);
        }
    }
    for (_, b) in child_nodes(&doc[si..se]) {
        walk(&b, &ID4, &lib, unit, &mut leaves);
    }

    Dae {
        unit,
        geoms,
        leaves,
        diffuse,
    }
}

// ---- our side: world leaves ----

/// (world, run, inherited material) for every placement. NO visibility
/// filtering: this export was made with "Export Hidden Geometry" ON
/// (unfiltered vertex counts match the dae; the WYSIWYG-filtered set is
/// ~3k vertices short), so hidden entities/layers are compared too.
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
    let root = m
        .geometry
        .iter()
        .position(|r| r.def_index.is_none())
        .expect("root run");
    out.push((ID4, root, 0));
    out
}

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

#[test]
fn theater_2017_world_equivalence() {
    let data = corpus("../third-party/theater-2017.skp");
    let m = openskp::Model::parse(&data).unwrap();
    assert!(
        m.diagnostics
            .iter()
            .any(|d| matches!(d, openskp::Diagnostic::ContinuousWalk { .. })),
        "theater-2017 must parse on the continuous path"
    );
    assert_eq!(
        m.diagnostics.iter().filter(|d| d.is_desync()).count(),
        0,
        "zero desyncs"
    );
    let dae = parse_dae("../third-party/theater-2017.dae");
    let leaves = our_leaves(&m);

    // -- vertex-set containment + ring coverage --
    let mut dae_raw: BTreeSet<[i64; 3]> = BTreeSet::new();
    for (world, gid) in &dae.leaves {
        for &p in &dae.geoms[gid].pos {
            dae_raw.insert(q(apply(
                world,
                [p[0] * dae.unit, p[1] * dae.unit, p[2] * dae.unit],
            )));
        }
    }
    let mut our_raw: BTreeSet<[i64; 3]> = BTreeSet::new();
    for (world, ri, _) in &leaves {
        for &p in &m.geometry[*ri].mesh.vertices {
            our_raw.insert(q(apply(world, p)));
        }
    }
    let mut union = our_raw.clone();
    union.extend(dae_raw.iter().copied());
    let snap = Snap::of(&union);
    let ours: BTreeSet<[i64; 3]> = our_raw.iter().map(|&k| snap.snap(k)).collect();
    let theirs: BTreeSet<[i64; 3]> = dae_raw.iter().map(|&k| snap.snap(k)).collect();
    let only_dae = theirs.difference(&ours).count();
    let only_ours = ours.difference(&theirs).count();
    println!(
        "verts: ours {} dae {} | only-dae {} only-ours {}",
        ours.len(),
        theirs.len(),
        only_dae,
        only_ours
    );
    // Every exported vertex must exist in our decode. (We may hold MORE:
    // hidden-geometry filtering is instance/face-level here, while the
    // exporter also drops e.g. edge-only vertices of hidden edges.)
    assert_eq!(only_dae, 0, "dae has vertices we failed to decode");

    // -- face-ring sets: every exported face must be one of ours --
    let mut our_rings: BTreeSet<Vec<[i64; 3]>> = BTreeSet::new();
    for (world, ri, _) in &leaves {
        let mesh = &m.geometry[*ri].mesh;
        for f in &mesh.faces {
            let ring: Vec<[i64; 3]> = f
                .outer
                .iter()
                .map(|&vi| snap.snap(q(apply(world, mesh.vertices[vi as usize]))))
                .collect();
            our_rings.insert(canon_full(&ring));
        }
    }
    let mut dae_rings: BTreeSet<Vec<[i64; 3]>> = BTreeSet::new();
    let mut ring_src: HashMap<Vec<[i64; 3]>, (String, Vec<usize>)> = HashMap::new();
    for (world, gid) in &dae.leaves {
        let g = &dae.geoms[gid];
        for ring in &g.rings {
            let r: Vec<[i64; 3]> = ring
                .iter()
                .map(|&i| {
                    let p = g.pos[i];
                    snap.snap(q(apply(
                        world,
                        [p[0] * dae.unit, p[1] * dae.unit, p[2] * dae.unit],
                    )))
                })
                .collect();
            let c = canon_full(&r);
            ring_src.entry(c.clone()).or_insert_with(|| (gid.clone(), ring.clone()));
            dae_rings.insert(c);
        }
    }
    let ring_only_dae = dae_rings.difference(&our_rings).count();
    let ring_only_ours = our_rings.difference(&dae_rings).count();
    assert_eq!(ring_only_ours, 0, "we hold face rings the export lacks");
    println!(
        "rings: ours {} dae {} | only-dae {} only-ours {}",
        our_rings.len(),
        dae_rings.len(),
        ring_only_dae,
        ring_only_ours
    );
    // classify: same unordered corner SET on the other side = a ring-path
    // difference (outer-choice / traversal), not missing geometry
    let vset = |r: &Vec<[i64; 3]>| -> BTreeSet<[i64; 3]> { r.iter().copied().collect() };
    let our_vsets: BTreeSet<BTreeSet<[i64; 3]>> = our_rings.iter().map(&vset).collect();
    // per-face outer ∪ holes corner unions: a face whose hole TOUCHES the
    // outer boundary exports as ONE self-touching ring (pinch vertices
    // repeat) — same face, merged representation
    let mut face_unions: BTreeSet<BTreeSet<[i64; 3]>> = BTreeSet::new();
    for (world, ri, _) in &leaves {
        let mesh = &m.geometry[*ri].mesh;
        for f in &mesh.faces {
            if f.holes.is_empty() {
                continue;
            }
            let mut u: BTreeSet<[i64; 3]> = BTreeSet::new();
            for &vi in f.outer.iter().chain(f.holes.iter().flatten()) {
                u.insert(snap.snap(q(apply(world, mesh.vertices[vi as usize]))));
            }
            face_unions.insert(u);
        }
    }
    let (mut path_diff, mut merged_hole, mut unexplained) = (0, 0, 0);
    let all_face_sets: Vec<(BTreeSet<[i64; 3]>, usize)> = our_rings
        .iter()
        .map(|r| (vset(r), r.len()))
        .collect();
    for r in dae_rings.difference(&our_rings) {
        let s = vset(r);
        if our_vsets.contains(&s) {
            path_diff += 1; // same corners, different traversal
        } else if face_unions.contains(&s) {
            merged_hole += 1; // our outer+holes, exporter's pinched ring
        } else {
            unexplained += 1;
            if unexplained <= 4 {
                let (gid, idx) = &ring_src[r];
                println!(
                    "  UNEXPLAINED dae ring ({} corners) gid={gid} indices={idx:?}",
                    r.len()
                );
            }
        }
    }
    for r in our_rings.difference(&dae_rings).take(8) {
        println!("  only-ours ring: {} corners ({} distinct)", r.len(), vset(r).len());
    }
    println!(
        "  only-dae classified: path-diff {path_diff}, merged-hole {merged_hole}, unexplained {unexplained}"
    );
    assert_eq!(unexplained, 0, "dae rings with no matching skp face");

    // -- materials: every exported diffuse is an extracted material --
    let our_colors: BTreeSet<[u8; 3]> = m
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
    let missing: Vec<_> = dae
        .diffuse
        .iter()
        .filter(|c| !our_colors.contains(&[c[0], c[1], c[2]]))
        .collect();
    println!(
        "materials: skp {} | dae diffuse {} | missing {:?}",
        m.materials.len(),
        dae.diffuse.len(),
        missing
    );
    assert!(missing.is_empty(), "dae diffuse colours missing from skp");

    // -- UVs: every textured exported face matches the §4v formula --
    // A real model holds COINCIDENT duplicate faces (studs boxed by
    // panels, etc.) whose world rings collide; keep every candidate and
    // accept if ANY matches (the dae exports each copy with its own
    // material).
    let mut face_of: HashMap<Vec<[i64; 3]>, Vec<(M4, usize, usize, u16)>> = HashMap::new();
    for (world, ri, inherited) in &leaves {
        let mesh = &m.geometry[*ri].mesh;
        for (fi, f) in mesh.faces.iter().enumerate() {
            let ring: Vec<[i64; 3]> = f
                .outer
                .iter()
                .map(|&vi| snap.snap(q(apply(world, mesh.vertices[vi as usize]))))
                .collect();
            face_of
                .entry(canon_full(&ring))
                .or_default()
                .push((*world, *ri, fi, *inherited));
        }
    }
    let (mut checked, mut unmatched, mut failed) = (0usize, 0usize, 0usize);
    let mut census: std::collections::BTreeMap<(&str, bool, i8), usize> = Default::default();
    let mut worst = 0.0f64;
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
            let Some(cands) = face_of.get(&key) else {
                unmatched += 1;
                continue;
            };
            // Best error over candidates × sides × {plain, u-mirrored}.
            // The u-mirror is NOT format semantics: the export writes each
            // face twice, and the REVERSE-VIEW copy (the unpainted side)
            // shows the painted side's texture mirrored in u — the census
            // confirms every mirrored winner is a front-side map (its back
            // copy), while painted sides always match §4v plain.
            let mut best = f64::INFINITY;
            let mut best_kind: (&str, openskp::Side, i8) = ("?", openskp::Side::Front, 0);
            for &(oworld, ri, fi, inherited) in cands {
                let mesh = &m.geometry[ri].mesh;
                let f = &mesh.faces[fi];
                let local_of: HashMap<[i64; 3], [f64; 3]> = f
                    .outer
                    .iter()
                    .map(|&vi| {
                        let p = mesh.vertices[vi as usize];
                        (snap.snap(q(apply(&oworld, p))), p)
                    })
                    .collect();
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
                    let Some(x) = f.uv_xform(side, size) else {
                        continue;
                    };
                    let (mut err_plain, mut err_mirror) = (0.0f64, 0.0f64);
                    for (wp, uv) in &wring {
                        let Some(local) = local_of.get(wp) else {
                            err_plain = f64::INFINITY;
                            err_mirror = f64::INFINITY;
                            break;
                        };
                        let got = x.apply(*local);
                        err_plain = err_plain
                            .max((got[0] - uv[0]).abs())
                            .max((got[1] - uv[1]).abs());
                        err_mirror = err_mirror
                            .max((-got[0] - uv[0]).abs())
                            .max((got[1] - uv[1]).abs());
                    }
                    let nz = f.normal[2];
                    for (err, mir) in [(err_plain, 0i8), (err_mirror, 1i8)] {
                        if err < best {
                            best = err;
                            let bucket = if nz.abs() > 1.0 - 1e-9 {
                                if nz > 0.0 { "horiz+Z" } else { "horiz-Z" }
                            } else if nz.abs() < 1e-9 {
                                "vertical"
                            } else {
                                "sloped"
                            };
                            best_kind = (bucket, side, mir);
                        }
                    }
                }
            }
            checked += 1;
            if best < 1e-3 {
                *census
                    .entry((best_kind.0, best_kind.1 == openskp::Side::Front, best_kind.2))
                    .or_default() += 1;
            } else {
                failed += 1;
                if best.is_finite() && best > worst {
                    worst = best;
                }
                if failed <= 10 {
                    println!(
                        "  uv fail: ({gid}) err {best:.4} kind {best_kind:?} cands {}",
                        cands.len()
                    );
                }
            }
        }
    }
    println!("  census (bucket, is_front, mirrored) -> passes: {census:?}");
    println!(
        "uvs: textured dae faces {} | checked {checked} unmatched {unmatched} failed {failed} (worst finite {worst:e})",
        checked + unmatched
    );
    assert_eq!(unmatched, 0, "textured dae faces with no skp face match");
    assert_eq!(failed, 0, "textured faces whose UVs miss the §4v formula");
}
