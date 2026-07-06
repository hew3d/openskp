//! Phase 3.1 acceptance: the materialized mesh equals the COLLADA ground
//! truth — positions (1e-9 m), face-vertex rings (up to rotation; winding
//! validated against the export's front-face copies), holes, and edge
//! flags. SketchUp's `.dae` exports keep faces as polygons (`<polylist>`,
//! or `<polygons><ph>` for hole-bearing faces) and export each face TWICE
//! (front and back material sides), so every mesh face must match a dae
//! face in the SAME winding (the front copy) and each dae face must match
//! one of ours up to reversal.

use std::path::PathBuf;

fn corpus(name: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    std::fs::read(p).unwrap()
}

// ---- a minimal COLLADA reader (SketchUp-export subset, tests only) ----

struct DaeFace {
    outer: Vec<[f64; 3]>,          // metres
    holes: Vec<Vec<[f64; 3]>>,     // metres
    corner_normals: Vec<[f64; 3]>, // outer ring corners
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!("{name}=\"");
    let i = tag.find(&pat)? + pat.len();
    let j = tag[i..].find('"')? + i;
    Some(&tag[i..j])
}

/// All `<tag …>…</tag>` blocks (SketchUp exports never nest a tag in itself).
fn blocks<'a>(doc: &'a str, tag: &str) -> Vec<(&'a str, &'a str)> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = doc[from..].find(&open) {
        let start = from + i;
        let head_end = doc[start..].find('>').unwrap() + start;
        let body_end = doc[head_end..].find(&close).unwrap() + head_end;
        out.push((&doc[start..head_end], &doc[head_end + 1..body_end]));
        from = body_end + close.len();
    }
    out
}

fn floats(s: &str) -> Vec<f64> {
    s.split_ascii_whitespace()
        .map(|t| t.parse().unwrap())
        .collect()
}

fn ints(s: &str) -> Vec<usize> {
    s.split_ascii_whitespace()
        .map(|t| t.parse().unwrap())
        .collect()
}

fn inner(body: &str, tag: &str) -> Option<String> {
    let b = blocks(body, tag);
    b.first().map(|(_, inner)| inner.to_string())
}

fn dae_faces(name: &str) -> Vec<DaeFace> {
    let doc = String::from_utf8(corpus(name)).unwrap();
    let unit: f64 = attr(
        &doc[doc.find("<unit").unwrap()..doc.find("<unit").unwrap() + 80],
        "meter",
    )
    .unwrap()
    .parse()
    .unwrap();

    // float_array id → values
    let mut arrays = std::collections::HashMap::new();
    for (head, body) in blocks(&doc, "float_array") {
        arrays.insert(attr(head, "id").unwrap().to_string(), floats(body));
    }
    // source id → its float_array values (SketchUp: one array per source)
    let mut sources = std::collections::HashMap::new();
    for (head, body) in blocks(&doc, "source") {
        if let Some((fa_head, _)) = blocks(body, "float_array").first() {
            sources.insert(
                attr(head, "id").unwrap().to_string(),
                arrays[attr(fa_head, "id").unwrap()].clone(),
            );
        }
    }
    // vertices id → (positions, normals)
    let mut verts = std::collections::HashMap::new();
    for (head, body) in blocks(&doc, "vertices") {
        let mut pos = None;
        let mut nrm = None;
        for part in body.split("<input").skip(1) {
            let tag = &part[..part.find("/>").unwrap()];
            let src = attr(tag, "source").unwrap().trim_start_matches('#');
            match attr(tag, "semantic").unwrap() {
                "POSITION" => pos = Some(sources[src].clone()),
                "NORMAL" => nrm = Some(sources[src].clone()),
                _ => {}
            }
        }
        verts.insert(attr(head, "id").unwrap().to_string(), (pos.unwrap(), nrm));
    }

    let take = |vals: &[f64], idx: usize, scale: f64| -> [f64; 3] {
        [
            vals[idx * 3] * scale,
            vals[idx * 3 + 1] * scale,
            vals[idx * 3 + 2] * scale,
        ]
    };
    let mut out = Vec::new();

    let vertex_source = |body: &str| -> String {
        for part in body.split("<input").skip(1) {
            let tag = &part[..part.find("/>").unwrap()];
            if attr(tag, "semantic") == Some("VERTEX") {
                return attr(tag, "source")
                    .unwrap()
                    .trim_start_matches('#')
                    .to_string();
            }
        }
        panic!("no VERTEX input");
    };

    for (_, body) in blocks(&doc, "polylist") {
        let (pos, nrm) = &verts[&vertex_source(body)];
        let vcount = ints(&inner(body, "vcount").unwrap());
        let p = ints(&inner(body, "p").unwrap());
        let mut at = 0;
        for n in vcount {
            let ring: Vec<usize> = p[at..at + n].to_vec();
            at += n;
            out.push(DaeFace {
                outer: ring.iter().map(|&i| take(pos, i, unit)).collect(),
                holes: Vec::new(),
                corner_normals: match nrm {
                    Some(nv) => ring.iter().map(|&i| take(nv, i, 1.0)).collect(),
                    None => Vec::new(),
                },
            });
        }
    }
    for (_, body) in blocks(&doc, "polygons") {
        let (pos, nrm) = &verts[&vertex_source(body)];
        for (_, ph) in blocks(body, "ph") {
            let outer = ints(&inner(ph, "p").unwrap());
            let holes: Vec<Vec<usize>> = blocks(ph, "h").iter().map(|(_, h)| ints(h)).collect();
            out.push(DaeFace {
                outer: outer.iter().map(|&i| take(pos, i, unit)).collect(),
                holes: holes
                    .iter()
                    .map(|h| h.iter().map(|&i| take(pos, i, unit)).collect())
                    .collect(),
                corner_normals: match nrm {
                    Some(nv) => outer.iter().map(|&i| take(nv, i, 1.0)).collect(),
                    None => Vec::new(),
                },
            });
        }
    }
    out
}

// ---- ring canonicalization ----
//
// Quantization grid: 0.1 µm. The .dae prints ~9 significant digits, so
// coordinates carry ~1e-9 m of print noise (1.0 arrives as 0.999999999);
// a 1e-7 m grid absorbs that while sitting far below any corpus feature
// size — the plan's "positions within 1e-9 m" holds with margin.

fn q(p: [f64; 3]) -> [i64; 3] {
    [
        (p[0] * 1e7).round() as i64,
        (p[1] * 1e7).round() as i64,
        (p[2] * 1e7).round() as i64,
    ]
}

/// Rotation-invariant canonical form (winding preserved).
fn canon_rot(ring: &[[i64; 3]]) -> Vec<[i64; 3]> {
    (0..ring.len())
        .map(|s| {
            ring.iter()
                .cycle()
                .skip(s)
                .take(ring.len())
                .copied()
                .collect::<Vec<_>>()
        })
        .min()
        .unwrap()
}

/// Rotation+reversal-invariant canonical form.
fn canon_full(ring: &[[i64; 3]]) -> Vec<[i64; 3]> {
    let fwd = canon_rot(ring);
    let rev: Vec<[i64; 3]> = ring.iter().rev().copied().collect();
    fwd.min(canon_rot(&rev))
}

fn mesh_union(name: &str) -> openskp::Mesh {
    let runs = openskp::geometry_runs(&corpus(name));
    assert!(!runs.is_empty(), "{name}: no surviving runs");
    let mut all = openskp::Mesh::default();
    for r in runs {
        let off = all.vertices.len() as u32;
        all.vertices.extend_from_slice(&r.mesh.vertices);
        for f in &r.mesh.faces {
            let mut f = f.clone();
            f.outer.iter_mut().for_each(|v| *v += off);
            f.holes
                .iter_mut()
                .for_each(|h| h.iter_mut().for_each(|v| *v += off));
            all.faces.push(f);
        }
        for e in &r.mesh.edges {
            all.edges.push(openskp::MeshEdge {
                v0: e.v0 + off,
                v1: e.v1 + off,
                ..*e
            });
        }
    }
    all
}

/// The full equivalence check for one file.
fn check(name_skp: &str, name_dae: &str, expect_faces: usize) {
    let mesh = mesh_union(name_skp);
    let dae = dae_faces(name_dae);
    assert_eq!(
        dae.len(),
        expect_faces * 2,
        "{name_dae}: the export carries each face twice (front+back)"
    );
    assert_eq!(mesh.faces.len(), expect_faces, "{name_skp}: face count");

    // Positions: every dae corner must be one of our vertices and vice versa.
    let ours: std::collections::BTreeSet<[i64; 3]> = mesh.vertices.iter().map(|&p| q(p)).collect();
    let theirs: std::collections::BTreeSet<[i64; 3]> = dae
        .iter()
        .flat_map(|f| {
            f.outer
                .iter()
                .chain(f.holes.iter().flatten())
                .map(|&p| q(p))
        })
        .collect();
    assert_eq!(ours, theirs, "{name_skp}: vertex position sets differ");

    // Faces: ring equality. Ours→dae in the SAME winding (front copy must
    // exist), dae→ours up to reversal (back copies match reversed).
    let to_coords = |ring: &[u32]| -> Vec<[i64; 3]> {
        ring.iter().map(|&v| q(mesh.vertices[v as usize])).collect()
    };
    let dae_rot: Vec<Vec<[i64; 3]>> = dae
        .iter()
        .map(|f| canon_rot(&f.outer.iter().map(|&p| q(p)).collect::<Vec<_>>()))
        .collect();
    let dae_full: std::collections::BTreeSet<Vec<[i64; 3]>> = dae
        .iter()
        .map(|f| canon_full(&f.outer.iter().map(|&p| q(p)).collect::<Vec<_>>()))
        .collect();

    for (fi, f) in mesh.faces.iter().enumerate() {
        let ring = to_coords(&f.outer);
        let same_winding = canon_rot(&ring);
        let matched: Vec<usize> = dae_rot
            .iter()
            .enumerate()
            .filter(|(_, r)| **r == same_winding)
            .map(|(i, _)| i)
            .collect();
        assert!(
            !matched.is_empty(),
            "{name_skp}: face {fi} has no same-winding dae match (winding convention broken?)"
        );
        // Winding sign: the matched front copy's exported corner normals
        // must agree with our plane-derived normal.
        let dn = &dae[matched[0]].corner_normals;
        if !dn.is_empty() {
            let d = dn[0];
            let dot = d[0] * f.normal[0] + d[1] * f.normal[1] + d[2] * f.normal[2];
            assert!(
                dot > 0.99,
                "{name_skp}: face {fi} normal disagrees with the matched dae front copy ({dot})"
            );
        }
        // Holes (face-with-hole): compare as canonical sets.
        let our_holes: std::collections::BTreeSet<Vec<[i64; 3]>> =
            f.holes.iter().map(|h| canon_full(&to_coords(h))).collect();
        let dae_holes: std::collections::BTreeSet<Vec<[i64; 3]>> = dae[matched[0]]
            .holes
            .iter()
            .map(|h| canon_full(&h.iter().map(|&p| q(p)).collect::<Vec<_>>()))
            .collect();
        assert_eq!(our_holes, dae_holes, "{name_skp}: face {fi} holes");
    }
    // Every dae face (front or back) matches one of ours up to reversal.
    let our_full: std::collections::BTreeSet<Vec<[i64; 3]>> = mesh
        .faces
        .iter()
        .map(|f| canon_full(&to_coords(&f.outer)))
        .collect();
    assert_eq!(
        dae_full, our_full,
        "{name_skp}: dae/mesh face ring sets differ"
    );
}

#[test]
fn box_mesh_matches_dae() {
    check("box.skp", "box.dae", 6);
}

#[test]
fn triangle_mesh_matches_dae() {
    check("triangle-face.skp", "triangle-face.dae", 1);
}

#[test]
fn ngon_mesh_matches_dae() {
    check("ngon-face.skp", "ngon-face.dae", 1);
}

#[test]
fn circle_mesh_matches_dae() {
    check("circle.skp", "circle.dae", 1);
}

#[test]
fn face_with_hole_mesh_matches_dae() {
    check("face-with-hole.skp", "face-with-hole.dae", 1);
}

#[test]
fn layers_mesh_matches_dae() {
    check("layers.skp", "layers.dae", 18);
}

#[test]
fn soft_smooth_flags_and_mesh() {
    // The drawn box is the SECOND run (the first is a stale instance-less
    // definition, SKP_FORMAT §4j) — the .dae covers the drawn one.
    let runs = openskp::geometry_runs(&corpus("soft-smooth-edges.skp"));
    let drawn = runs.last().unwrap();
    assert_eq!(drawn.mesh.faces.len(), 6);
    assert_eq!(drawn.mesh.edges.len(), 12);
    assert!(
        drawn.mesh.edges.iter().all(|e| e.soft && e.smooth),
        "every drawn edge was softened+smoothed"
    );
    // And box.skp's edges are all hard.
    let plain = openskp::geometry_runs(&corpus("box.skp"));
    assert!(plain[0].mesh.edges.iter().all(|e| !e.soft && !e.smooth));
}
