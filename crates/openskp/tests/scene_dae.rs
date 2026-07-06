//! Phase 3.3 acceptance: composed world transforms — every leaf mesh's
//! world-space vertex positions match the `.dae` scene graph. The dae
//! side composes `<node><matrix>` chains through `<instance_node>`
//! indirection (SketchUp puts definitions in `<library_nodes>`); our side
//! composes `Model::scene()` worlds over run meshes.
//!
//! component-rotate.skp is the discriminating oracle for the 13-f64
//! transform's 3×3 layout (translation-only files can't tell).

use std::collections::BTreeSet;
use std::path::PathBuf;

fn corpus(name: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    std::fs::read(p).unwrap()
}

// ---- minimal COLLADA scene reader (tests only) ----

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!("{name}=\"");
    let i = tag.find(&pat)? + pat.len();
    let j = tag[i..].find('"')? + i;
    Some(&tag[i..j])
}

/// Recursive <node> splitter: returns (head, body) of every DIRECT child
/// <node> of `s`, handling nesting by depth counting.
fn child_nodes(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = s[from..].find("<node") {
        let start = from + i;
        let head_end = s[start..].find('>').unwrap() + start;
        // find matching </node> at depth 0
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
        let body_end = p - 6;
        out.push((
            s[start..head_end].to_string(),
            s[head_end + 1..body_end].to_string(),
        ));
        from = p;
    }
    out
}

/// Only this node's own tags (not nested nodes'): strip child node bodies.
fn own_content(body: &str) -> String {
    let mut own = String::new();
    let mut rest = body;
    while let Some(i) = rest.find("<node") {
        own.push_str(&rest[..i]);
        // skip the whole child node block
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

/// Collect the world-space vertex set of every instance_geometry leaf.
fn walk_dae_node(
    head: &str,
    body: &str,
    parent: &M4,
    lib: &std::collections::HashMap<String, (String, String)>,
    geoms: &std::collections::HashMap<String, Vec<[f64; 3]>>,
    unit: f64,
    out: &mut Vec<BTreeSet<[i64; 3]>>,
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
            // translations are in file units → metres
            for k in [3, 7, 11] {
                m[k] *= unit;
            }
            m
        }
        None => ID4,
    };
    let world = mul(parent, &local);
    let _ = head;
    // geometry leaves
    let mut from = 0;
    while let Some(i) = own[from..].find("<instance_geometry") {
        let start = from + i;
        let tag_end = own[start..].find('>').unwrap() + start;
        let url = attr(&own[start..tag_end], "url")
            .unwrap()
            .trim_start_matches('#');
        let verts = &geoms[url];
        out.push(
            verts
                .iter()
                .map(|&p| {
                    let w = apply(&world, [p[0] * unit, p[1] * unit, p[2] * unit]);
                    [
                        (w[0] * 1e7).round() as i64,
                        (w[1] * 1e7).round() as i64,
                        (w[2] * 1e7).round() as i64,
                    ]
                })
                .collect(),
        );
        from = tag_end;
    }
    // instance_node indirection
    let mut from = 0;
    while let Some(i) = own[from..].find("<instance_node") {
        let start = from + i;
        let tag_end = own[start..].find("/>").unwrap() + start;
        let url = attr(&own[start..tag_end], "url")
            .unwrap()
            .trim_start_matches('#');
        let (h, b) = &lib[url];
        // the referenced library node's own matrix (if any) applies below world
        walk_dae_node(h, b, &world, lib, geoms, unit, out);
        from = tag_end;
    }
    // direct child nodes
    for (h, b) in child_nodes(body) {
        walk_dae_node(&h, &b, &world, lib, geoms, unit, out);
    }
}

/// Parse: geometry id → unique positions; node library; scene roots.
fn dae_leaf_sets(name: &str) -> Vec<BTreeSet<[i64; 3]>> {
    let doc = String::from_utf8(corpus(name)).unwrap();
    let ui = doc.find("<unit").unwrap();
    let unit: f64 = attr(&doc[ui..ui + 80], "meter").unwrap().parse().unwrap();

    // geometry id → positions (via mesh/vertices/POSITION indirection —
    // SketchUp exports one POSITION float_array per geometry, so take the
    // FIRST float_array inside each <geometry> block).
    let mut geoms = std::collections::HashMap::new();
    let mut from = 0;
    while let Some(i) = doc[from..].find("<geometry") {
        let start = from + i;
        let head_end = doc[start..].find('>').unwrap() + start;
        let gid = attr(&doc[start..head_end], "id").unwrap().to_string();
        let gend = doc[start..].find("</geometry>").unwrap() + start;
        let block = &doc[head_end..gend];
        let fi = block.find("<float_array").unwrap();
        let fh = block[fi..].find('>').unwrap() + fi;
        let fe = block[fh..].find("</float_array>").unwrap() + fh;
        let vals: Vec<f64> = block[fh + 1..fe]
            .split_ascii_whitespace()
            .map(|t| t.parse().unwrap())
            .collect();
        geoms.insert(
            gid,
            vals.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
        );
        from = gend;
    }

    // library_nodes: id → (head, body)
    let mut lib = std::collections::HashMap::new();
    if let Some(li) = doc.find("<library_nodes>") {
        let le = doc[li..].find("</library_nodes>").unwrap() + li;
        for (h, b) in child_nodes(&doc[li..le]) {
            if let Some(id) = attr(&h, "id") {
                lib.insert(id.to_string(), (h.clone(), b.clone()));
            }
        }
    }

    // visual scene roots
    let si = doc.find("<library_visual_scenes>").unwrap();
    let se = doc[si..].find("</library_visual_scenes>").unwrap() + si;
    let mut out = Vec::new();
    for (h, b) in child_nodes(&doc[si..se]) {
        walk_dae_node(&h, &b, &ID4, &lib, &geoms, unit, &mut out);
    }
    out
}

/// Our side: every scene leaf's world-transformed mesh vertex set.
fn our_leaf_sets(name: &str) -> Vec<BTreeSet<[i64; 3]>> {
    let m = openskp::Model::parse(&corpus(name)).unwrap();
    let mut out = Vec::new();
    fn rec(m: &openskp::Model, n: &openskp::Node, out: &mut Vec<BTreeSet<[i64; 3]>>) {
        if let Some(ri) = n.run {
            let mesh = &m.geometry[ri].mesh;
            if !mesh.vertices.is_empty() {
                out.push(
                    mesh.vertices
                        .iter()
                        .map(|&p| {
                            let w = [
                                n.world[0] * p[0]
                                    + n.world[1] * p[1]
                                    + n.world[2] * p[2]
                                    + n.world[3],
                                n.world[4] * p[0]
                                    + n.world[5] * p[1]
                                    + n.world[6] * p[2]
                                    + n.world[7],
                                n.world[8] * p[0]
                                    + n.world[9] * p[1]
                                    + n.world[10] * p[2]
                                    + n.world[11],
                            ];
                            [
                                (w[0] * 1e7).round() as i64,
                                (w[1] * 1e7).round() as i64,
                                (w[2] * 1e7).round() as i64,
                            ]
                        })
                        .collect(),
                );
            }
        }
        for c in &n.children {
            rec(m, c, out);
        }
    }
    for n in m.scene() {
        rec(&m, &n, &mut out);
    }
    out
}

fn check(name_skp: &str, name_dae: &str, expect_leaves: usize) {
    let mut ours = our_leaf_sets(name_skp);
    let mut theirs = dae_leaf_sets(name_dae);
    assert_eq!(ours.len(), expect_leaves, "{name_skp}: our leaf count");
    assert_eq!(theirs.len(), expect_leaves, "{name_dae}: dae leaf count");
    ours.sort();
    theirs.sort();
    assert_eq!(
        ours, theirs,
        "{name_skp}: world-space leaf vertex sets differ"
    );
}

#[test]
fn nested_component_world_positions() {
    // Outer instance at origin; its definition holds the cube + a child
    // instance at (1 m, 0, 0) — two leaves, composed two deep.
    check("nested-component.skp", "nested-component.dae", 2);
}

#[test]
fn two_components_world_positions() {
    check("two-components.skp", "two-components.dae", 2);
}

#[test]
fn component_move_world_positions() {
    check("component-move.skp", "component-move.dae", 2);
}

#[test]
fn component_rotate_world_positions() {
    // The rotated second instance discriminates the 3×3 layout convention.
    check("component-rotate.skp", "component-rotate.dae", 2);
}

#[test]
fn box_component_two_instances_world_positions() {
    check(
        "box-component-two-instances.skp",
        "box-component-two-instances.dae",
        2,
    );
}

#[test]
fn nested_3_deep_world_positions() {
    // Tier C2: C ⊃ 2×B ⊃ 2×A, typed offsets 1 m (X)
    // and 2 m (Y) at the B and C levels — four leaf cubes composed THREE
    // instances deep. Validates world = parent × local across 3 levels.
    check("nested-3-deep.skp", "nested-3-deep.dae", 4);
}

#[test]
fn instance_scaled_world_positions() {
    // Tier C2: three instances of one box definition — identity, a
    // non-uniform 2×1×1 scale, and an X mirror (negative determinant).
    // Proves scale/mirror live in the 13-f64 transform's 3×3.
    check("instance-scaled.skp", "instance-scaled.dae", 3);
}
