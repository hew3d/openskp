//! Phase 1.2 acceptance: instance→definition linkage is STRUCTURAL — the
//! file declares each definition's archive map index in the pre-list
//! prelude (`GeometryRun::def_index`), instances carry that index as their
//! def-ref, and the definition's name record follows its list (bracketed by
//! the next framed run). The positional zip survives only as a RECORDED
//! fallback (`Diagnostic::HeuristicLinkage`).

use std::path::PathBuf;

fn corpus(name: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    std::fs::read(p).unwrap()
}

fn linkage(m: &openskp::Model) -> Vec<(Option<String>, [f64; 3])> {
    m.instances
        .iter()
        .map(|i| (i.definition.clone(), i.translation_m))
        .collect()
}

fn fell_back(m: &openskp::Model) -> bool {
    m.diagnostics
        .iter()
        .any(|d| matches!(d, openskp::Diagnostic::HeuristicLinkage { .. }))
}

type NamedTranslation = (&'static str, [f64; 3]);

/// The previously validated (zip-era) linkage, now reproduced structurally:
/// no HeuristicLinkage diagnostic may fire on these files.
#[test]
fn component_files_link_structurally() {
    let cases: &[(&str, &[NamedTranslation])] = &[
        ("box-component.skp", &[("Box Component", [0.0, 0.0, 0.0])]),
        ("group.skp", &[("Group#1", [0.0, 0.0, 0.0])]),
        ("box-group.skp", &[("Group#1", [0.0, 0.0, 0.0])]),
        (
            "two-components.skp",
            &[("Box 1", [0.0, 0.0, 0.0]), ("Box 2", [1.0, 0.0, 0.0])],
        ),
        (
            "nested-component.skp",
            &[("Box 2", [1.0, 0.0, 0.0]), ("Box 1", [0.0, 0.0, 0.0])],
        ),
        (
            "box-component-two-instances.skp",
            &[
                ("Box Component", [0.0, 0.0, 0.0]),
                ("Box Component", [1.24301, 0.0, 0.0]),
            ],
        ),
        (
            "component-move.skp",
            &[
                ("Box Component", [0.0, 0.0, 0.0]),
                ("Box Component", [2.0, 0.0, 0.0]),
            ],
        ),
        (
            "component-rotate.skp",
            &[
                ("Box Component", [0.0, 0.0, 0.0]),
                ("Box Component", [2.0, 0.0, 0.0]),
            ],
        ),
    ];
    for (file, expected) in cases {
        let m = openskp::Model::parse(&corpus(file)).unwrap();
        let want: Vec<(Option<String>, [f64; 3])> = expected
            .iter()
            .map(|(n, t)| (Some(n.to_string()), *t))
            .collect();
        assert_eq!(linkage(&m), want, "{file}: linkage");
        assert!(
            !fell_back(&m),
            "{file}: expected the STRUCTURAL link, got the zip fallback"
        );
    }
}

/// The declared map indexes are exposed per run and equal the instances'
/// def-refs (the SKP_FORMAT §4h harvest, pinned).
#[test]
fn declared_def_indexes_are_exposed() {
    type StartAndIndex = (usize, Option<usize>);
    let cases: &[(&str, &[StartAndIndex])] = &[
        ("box-component.skp", &[(0x2795, Some(23))]),
        ("group.skp", &[(0x265e, Some(21))]),
        (
            "two-components.skp",
            &[(0x2870, Some(21)), (0x2ead, Some(87))],
        ),
        (
            "nested-component.skp",
            &[(0x2870, Some(21)), (0x2ead, Some(87))],
        ),
    ];
    for (file, expected) in cases {
        let runs = openskp::geometry_runs(&corpus(file));
        let got: Vec<(usize, Option<usize>)> =
            runs.iter().map(|r| (r.start, r.def_index)).collect();
        assert_eq!(&got, expected, "{file}: (start, def_index)");
    }
}

/// mixed-definition: Phase 1.3's structural candidate discovery surfaces
/// the "0.5 box" definition's own list (@0x2f01, declared index 22 — the
/// exact def-ref its nested instance carries), so the pairing is complete
/// and the linkage is structural. Its full cube topology is recovered from
/// a definition that also contains unknown-class entities — Phase 2.3's
/// acceptance shape, early.
#[test]
fn mixed_definition_links_structurally_with_the_inner_cube() {
    let m = openskp::Model::parse(&corpus("mixed-definition.skp")).unwrap();
    assert!(
        !fell_back(&m),
        "mixed-definition: expected the STRUCTURAL link; diagnostics: {:?}",
        m.diagnostics
    );
    assert_eq!(
        linkage(&m),
        vec![
            (Some("0.5 box".to_string()), [1.0, 0.0, 0.5]),
            (Some("Outer Component".to_string()), [0.0, 0.0, 0.0]),
        ],
        "mixed-definition: linked names/translations"
    );
    let cube = m
        .geometry
        .iter()
        .find(|r| r.def_index == Some(22))
        .expect("the 0.5 box definition's own run (global slot 22)");
    // (Formerly pinned `cube.start == 0x2f01`, the entity LIST offset the
    // legacy cluster heuristic used to miss; on the §4s continuous path a
    // run's span starts at the definition's own record tag instead.)
    assert_eq!(
        (
            cube.topology.vertices,
            cube.topology.edges,
            cube.topology.faces
        ),
        (8, 12, 6),
        "full cube topology from a definition that also holds unknowns"
    );
    assert_eq!(cube.resolved.0, cube.resolved.1, "100% back-refs");
}

/// long-name was the zip-fallback poster child on the legacy path (TWO
/// cube serializations, ONE name record — an incomplete byte-scan
/// pairing). The §4s CONTINUOUS walk supersedes that: def-refs are GLOBAL
/// map slots read straight off the archive, so the instance links exactly
/// with no positional zip and no HeuristicLinkage — the ContinuousWalk
/// marker records which path ran. (The recorded zip fallback remains the
/// legacy path's behavior, still exercised whenever the continuous walk
/// dies — diagnostics.rs's corrupted-stream test covers that route.)
#[test]
fn long_name_links_exactly_on_the_continuous_path() {
    let m = openskp::Model::parse(&corpus("long-name.skp")).unwrap();
    assert!(
        m.diagnostics
            .iter()
            .any(|d| matches!(d, openskp::Diagnostic::ContinuousWalk { .. })),
        "long-name: expected the continuous walk; diagnostics: {:?}",
        m.diagnostics
    );
    assert!(
        !fell_back(&m),
        "long-name: continuous def-refs are exact — no zip fallback; diagnostics: {:?}",
        m.diagnostics
    );
    // The exact link lands on the definition the instance ACTUALLY
    // references (@0x13687-era serialization): the one carrying the
    // authored >255-char name — the escalated-length string record this
    // file exists to exercise. (The legacy zip used to link the stale
    // "Box Component" copy instead, because its byte-scan could only see
    // that one name record.)
    let links = linkage(&m);
    assert_eq!(links.len(), 1, "one placed instance");
    let (name, t) = &links[0];
    let name = name.as_deref().expect("linked definition name");
    assert!(
        name.chars().count() > 255,
        "the authored long name (>255 chars), got {} chars",
        name.chars().count()
    );
    assert!(name.starts_with("This is a component of a simple 1m square unit box"));
    assert_eq!(*t, [0.0, 0.0, 0.0]);
}
