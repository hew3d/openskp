//! §4t acceptance: welded/freehand `CCurve` records decode on the
//! continuous path, and their declared member-edge counts are the exact
//! polyline structure the COLLADA export carries. Oracle: curve.skp
//! (blank template, two authored freehand curves) vs corpus/curve.dae's
//! two `<lines>` geometries.

use std::path::PathBuf;

fn corpus(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    p
}

/// The `count` attribute of every `<lines …>` geometry in a `.dae`.
fn dae_line_counts(name: &str) -> Vec<u32> {
    let s = std::fs::read_to_string(corpus(name)).unwrap();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = s[from..].find("<lines ") {
        let start = from + i;
        let head_end = s[start..].find('>').unwrap() + start;
        let head = &s[start..head_end];
        if let Some(j) = head.find("count=\"") {
            let v = &head[j + 7..];
            let k = v.find('"').unwrap();
            out.push(v[..k].parse().unwrap());
        }
        from = head_end;
    }
    out
}

#[test]
fn curve_skp_ccurves_match_the_dae_polylines() {
    let m = openskp::Model::parse(&std::fs::read(corpus("curve.skp")).unwrap()).unwrap();
    assert!(
        m.diagnostics
            .iter()
            .any(|d| matches!(d, openskp::Diagnostic::ContinuousWalk { .. })),
        "curve.skp must parse via the continuous walk; diagnostics: {:?}",
        m.diagnostics
    );
    assert_eq!(
        m.diagnostics.iter().filter(|d| d.is_desync()).count(),
        0,
        "zero desyncs"
    );

    // Blank template: no definitions, ONE run — the root entity list.
    let root = m.geometry.last().expect("the root run");
    assert!(root.def_index.is_none(), "the root run");

    // Exactly the two authored freehand curves, each carrying its
    // member-edge count; together they own every edge in the drawing.
    let mut members = root.curve_members.clone();
    assert_eq!(members.len(), 2, "exactly two CCurve records");
    assert_eq!(
        members.iter().sum::<u32>() as usize,
        root.topology.edges,
        "the curves' member counts partition the root edge list"
    );

    // COLLADA ground truth: two <lines> geometries, one per curve, whose
    // segment counts are the member counts exactly.
    let mut dae = dae_line_counts("curve.dae");
    members.sort_unstable();
    dae.sort_unstable();
    assert_eq!(
        members, dae,
        "CCurve member-edge counts == the .dae <lines> segment counts"
    );
}
