//! Phase 2.3 acceptance — continue-past-unknown. Originally this pinned
//! mixed-definition's nested instance as the "recorded skip"; Phase 3.3's
//! in-list CComponentInstance reader then decoded it, so mixed-definition
//! now reads FULLY (zero desync, instance surfaced as PlacedInstance).
//! The standing continue-past-unknown witness is attributes.skp's pinned
//! resync histogram in tests/diagnostics.rs (866 recovered stalls across
//! 88 surviving high-confidence runs).

use std::path::PathBuf;

fn corpus(name: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    std::fs::read(p).unwrap()
}

#[test]
fn cube_survives_a_definition_full_of_unknowns() {
    let m = openskp::Model::parse(&corpus("mixed-definition.skp")).unwrap();

    // The outer definition's run (declared map index 89 — SKP_FORMAT §4h).
    let outer = m
        .geometry
        .iter()
        .find(|r| r.def_index == Some(89))
        .expect("the Outer Component's structurally framed run");
    assert_eq!(
        outer.frame,
        Some(21),
        "outer list declares 18 cube elements + instance + dimension + text"
    );
    assert_eq!(
        (
            outer.topology.vertices,
            outer.topology.edges,
            outer.topology.faces,
            outer.topology.loops,
            outer.topology.edge_uses,
        ),
        (8, 12, 6, 6, 24),
        "the FULL cube topology out of a list that also holds unknowns"
    );
    let (sat, con) = outer.resolved;
    assert_eq!(sat, con, "back-ref resolution stays at 100% (>= 75% floor)");
    // (`con` is 0 on the §4s continuous path: against the ONE global map,
    // back-refs resolve to their objects AT READ TIME, so no deferred
    // constraints remain — sat == con == 0 IS full resolution there. The
    // mesh assertions above are the substantive completeness check.)

    // Since 3.3 the nested instance DECODES: zero desync, and the child
    // placement is exposed with the def-ref of the "0.5 box" definition.
    assert_eq!(
        m.diagnostics.iter().filter(|d| d.is_desync()).count(),
        0,
        "mixed-definition reads fully since the in-list instance reader"
    );
    assert_eq!(
        outer.placed,
        vec![openskp::PlacedInstance {
            defref: 22,
            transform: outer.placed[0].transform,
            material: 0,
            hidden: false,
            layer: 0,
        }],
        "the nested instance places the 0.5 box definition (declared idx 22)"
    );
    let t = &outer.placed[0].transform;
    assert!(
        (t[9] / openskp::INCH - 1.0).abs() < 1e-9 && (t[11] / openskp::INCH - 0.5).abs() < 1e-9,
        "nested instance translation is the authored (1 m, 0, 0.5 m): {t:?}"
    );

    // And the nested definition's own cube is independently complete.
    let inner = m
        .geometry
        .iter()
        .find(|r| r.def_index == Some(22))
        .expect("the 0.5 box definition's run");
    assert_eq!(
        (
            inner.topology.vertices,
            inner.topology.edges,
            inner.topology.faces
        ),
        (8, 12, 6)
    );
    assert_eq!(inner.resolved.0, inner.resolved.1);
}
