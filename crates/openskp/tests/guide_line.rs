//! CConstructionLine acceptance (the last 2017 legacy-fallback file):
//! guide.skp parses on the CONTINUOUS path with zero desyncs, and the
//! decoded guide line matches the byte-scan extractor's ground truth —
//! the authored tape-measure guide at y = 1 m, running along +X.

use std::path::PathBuf;

/// A 2017 corpus file and its SketchUp 2026 twin (`corpus/2026/<name>`,
/// the web app's conversion of the same model; the same expectations
/// apply).
fn twin_dirs(name: &str) -> Vec<&'static str> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2026");
    p.push(name);
    if p.exists() {
        vec!["2017", "2026"]
    } else {
        vec!["2017"]
    }
}

fn corpus_at(dir: &str, name: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus");
    p.push(dir);
    p.push(name);
    std::fs::read(p).unwrap()
}

#[test]
fn guide_skp_parses_continuously_with_the_authored_guide() {
    for dir in twin_dirs("guide.skp") {
        let m = openskp::Model::parse(&corpus_at(dir, "guide.skp")).unwrap();
        // The continuous walk is the 2017 path; a 2026 file has no such diagnostic.
        if dir == "2017" {
            assert!(
                m.diagnostics
                    .iter()
                    .any(|d| matches!(d, openskp::Diagnostic::ContinuousWalk { .. })),
                "guide.skp must parse via the continuous walk; diagnostics: {:?}",
                m.diagnostics
            );
        }
        assert_eq!(
            m.diagnostics.iter().filter(|d| d.is_desync()).count(),
            0,
            "zero desyncs"
        );
        assert_eq!(m.guides.len(), 1, "the single authored guide line");
        let g = &m.guides[0];
        assert_eq!(g.point_m[1], 1.0, "authored 1 m offset");
        assert_eq!(g.point_m[2], 0.0);
        assert_eq!(g.direction, [1.0, 0.0, 0.0], "runs along +X");
        // The root run holds exactly the one CConstructionLine element.
        let root = m.geometry.last().expect("root run");
        assert!(root.def_index.is_none());
        assert_eq!(root.top_level, 1, "one root element (the guide)");
        assert_eq!(root.topology.edges, 0, "a guide is not geometry");
    }
}
