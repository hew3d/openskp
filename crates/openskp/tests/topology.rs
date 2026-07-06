//! Topology oracle: the Rust walk + back-ref resolution must reproduce the true
//! concrete topology from the COLLADA ground truth / SKP_FORMAT (a box is 8
//! vertices / 12 edges / 6 faces, etc.).

use std::path::PathBuf;

fn corpus(name: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    std::fs::read(p).unwrap()
}

/// (file, [(V, E, F, L, EU, Arc, satisfied, constraints)] per run, in order).
/// Concrete topology is ground truth from each `.dae`; the (satisfied,
/// constraints) back-ref counts are cross-checked against the Python reference.
type Run = (usize, usize, usize, usize, usize, usize, usize, usize);
const CASES: &[(&str, &[Run])] = &[
    ("box.skp", &[(8, 12, 6, 6, 24, 0, 56, 56)]),
    ("triangle-face.skp", &[(3, 3, 1, 1, 3, 0, 9, 9)]),
    ("face-with-hole.skp", &[(8, 8, 1, 2, 8, 0, 16, 16)]),
    ("ngon-face.skp", &[(6, 6, 1, 1, 6, 0, 18, 18)]),
    ("circle.skp", &[(24, 24, 1, 1, 24, 1, 72, 72)]),
    ("single-line.skp", &[(2, 1, 0, 0, 0, 0, 0, 0)]),
    ("two-lines.skp", &[(3, 2, 0, 0, 0, 0, 1, 1)]),
    ("polyline.skp", &[(5, 4, 0, 0, 0, 0, 3, 3)]),
    // arc: 12 segments per arc.dae (<lines count="12">). Before the §4i
    // sparse-pid decode this pinned (13, 11, .., 9, 10): the 12th edge's
    // pid is 4,864 = 0x1300 — a multiple of 256, stored with mask 02 @
    // 0x11abb — so the old `00 00 03`-only scanner never saw it, and its
    // absence left a dangling back-ref constraint (Phase 5.1 fixed both).
    ("arc.skp", &[(13, 12, 0, 0, 0, 1, 11, 11)]),
    ("box-component.skp", &[(8, 12, 6, 6, 24, 0, 35, 35)]),
    ("group.skp", &[(8, 12, 6, 6, 24, 0, 46, 46)]),
    (
        "two-components.skp",
        &[(8, 12, 6, 6, 24, 0, 38, 38), (8, 12, 6, 6, 24, 0, 56, 56)],
    ),
];

/// Phase 5.1 acceptance: the ~70k-entity stress file (2,703 push/pulled 1m
/// boxes, pids to 144,020 — §4i wide/sparse pid forms throughout) parses as
/// ONE clean run whose resolved topology is exact by construction, with
/// every back-ref constraint satisfied. Release-mode parse ≈ 1.7 s
/// (measured on Apple Silicon; the plan's bound is ~2 s).
#[test]
fn pid_stress_counts_by_construction() {
    let d = corpus("pid-stress.skp");
    let runs = openskp::geometry_runs(&d);
    assert_eq!(runs.len(), 1, "one contiguous entity list");
    let t = &runs[0].topology;
    assert_eq!(
        (t.vertices, t.edges, t.faces, t.loops, t.edge_uses),
        (2703 * 8, 2703 * 12, 2703 * 6, 2703 * 6, 2703 * 24),
        "counts must be exactly 2,703 x (8V, 12E, 6F, 6L, 24EU)"
    );
    let (sat, con) = runs[0].resolved;
    assert!(con > 100_000, "the stress run is heavily over-constrained");
    assert_eq!(sat, con, "100% back-ref resolution");
}

#[test]
fn topology_matches_ground_truth() {
    for (file, runs) in CASES {
        let d = corpus(file);
        let got = openskp::geometry_runs(&d);
        assert_eq!(got.len(), runs.len(), "{file}: run count");
        for (r, &(v, e, f, l, eu, arc, sat, con)) in got.iter().zip(runs.iter()) {
            let t = &r.topology;
            assert_eq!(
                (t.vertices, t.edges, t.faces, t.loops, t.edge_uses, t.curves),
                (v, e, f, l, eu, arc),
                "{file}: topology"
            );
            assert_eq!(r.resolved, (sat, con), "{file}: back-ref resolution");
        }
    }
}
