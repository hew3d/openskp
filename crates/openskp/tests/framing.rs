//! Phase 1.1 acceptance: entity lists are structurally framed by the count
//! u32 written immediately before them — run boundaries are exact, and the
//! gap/stall heuristics only carry runs that lack a frame (recorded as the
//! informational HeuristicFraming diagnostic).

use std::path::PathBuf;

fn corpus(name: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    std::fs::read(p).unwrap()
}

/// (file, per-run (declared frame, top-level elements read)). Frames are
/// by-construction ground truth: a cube's list = 12 CEdge + 6 CFace = 18
/// top-level elements (inline vertices/loops/edge-uses are pool objects, not
/// list elements); nested-component's inner definition adds 1 child
/// instance (19); layers.skp = 3 loose boxes = 54; pid-stress = 2,703 boxes
/// x 18 = 48,654; dimension.skp = 18 + the TWO authored linear dimensions =
/// 20 — independent confirmation of the authored ground truth.
///
/// read == frame (EXACT consumption) holds on purely-drawn geometry
/// (triangle/single-line/circle). Push/pulled boxes under-read: the walker
/// still HOPS the push/pull faces' trailing back-ref edge-lists as filler,
/// absorbing list elements — full-fidelity CFace body reading (Phase 2)
/// closes that gap; dimension's bodies await their Phase 2.2 reader. The
/// topology recovered is nonetheless complete and .dae-validated (see
/// topology.rs) — the frame today is an exact CAP and a truth anchor, not
/// yet always exactly met.
///
/// pid-stress (Phase 5.1): ONE run since the §4i pid-bitmask decode — the
/// walk crosses the whole 3.4 MB entity list (the pre-fix walk died at 172
/// elements and a junk second run appeared after it). 36,407 < 48,654 is
/// the same push/pull filler-hop under-read as box.skp's 14 < 18; the
/// resolved topology is exact by construction (topology.rs asserts
/// 2,703 × 8/12/6).
const CASES: &[(&str, &[(usize, usize)])] = &[
    ("box.skp", &[(18, 14)]),
    ("blank-template-box.skp", &[(18, 14)]),
    ("group.skp", &[(18, 14)]),
    ("box-component.skp", &[(18, 9)]),
    ("two-components.skp", &[(18, 12), (18, 14)]),
    ("nested-component.skp", &[(18, 12), (19, 15)]),
    ("layers.skp", &[(54, 42)]),
    ("triangle-face.skp", &[(4, 4)]),
    ("single-line.skp", &[(1, 1)]),
    ("circle.skp", &[(25, 25)]),
    ("pid-stress.skp", &[(48_654, 36_407)]),
    // Phase 2.2: both CDimensionLinear/CText elements now read — the old
    // phantom second run (the second annotation misparsed as a list) is
    // consumed into the real one.
    ("dimension.skp", &[(20, 16)]),
    ("text.skp", &[(20, 16)]),
    ("section-plane.skp", &[(19, 15)]),
    // mixed-definition: both definitions frame structurally; the outer
    // under-reads only via the known face filler-hops + the one nested
    // instance awaiting 3.3 (continue_past_unknown.rs pins the topology).
    ("mixed-definition.skp", &[(18, 7), (21, 15)]),
    // image.skp's quad definition consumes its frame EXACTLY once the
    // 5th element (CImage-family) reads.
    ("image.skp", &[(5, 5)]),
];

#[test]
fn entity_lists_are_structurally_framed() {
    for (file, expected) in CASES {
        let d = corpus(file);
        let runs = openskp::geometry_runs(&d);
        assert_eq!(runs.len(), expected.len(), "{file}: run count");
        for (r, &(frame, read)) in runs.iter().zip(expected.iter()) {
            assert_eq!(r.frame, Some(frame), "{file}: declared frame");
            assert_eq!(r.top_level, read, "{file}: top-level elements read");
            assert!(
                r.top_level <= frame,
                "{file}: the frame is a hard cap ({} > {})",
                r.top_level,
                frame
            );
            assert!(r.end > r.start, "{file}: end offset recorded");
        }
    }
}

/// The framing contract: a surviving run without a structural frame is
/// allowed ONLY if the fallback was RECORDED (HeuristicFraming diagnostic) —
/// the gap heuristic never carries a run silently.
#[test]
fn unframed_surviving_runs_are_always_recorded() {
    for entry in
        std::fs::read_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/2017"))
            .unwrap()
    {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("skp") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let d = std::fs::read(&path).unwrap();
        let (runs, diags) = openskp::geometry_runs_with_diagnostics(&d);
        for r in runs {
            if r.frame.is_none() {
                assert!(
                    diags
                        .iter()
                        .any(|di| matches!(di, openskp::Diagnostic::HeuristicFraming { at } if *at == r.start)),
                    "{name}: unframed surviving run @{:#x} has no HeuristicFraming record",
                    r.start
                );
            }
        }
    }
}
