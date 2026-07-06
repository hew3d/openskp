//! Phase 0.2 acceptance: a known class whose FILE declares an unexpected
//! schema number must downgrade to skip/stall — the 2017-validated reader
//! must never run over a possibly-changed layout — and the walk must degrade
//! safely (no panic, no confidently-wrong topology).

use std::path::Path;

fn corpus(name: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/2017")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Doctor BOTH places a real schema bump would appear (a consistent
/// "future" file): the ASCII `CEdge` new-class record in the entity stream
/// AND the `CVersionMap` entry.
///
/// New-class layout: `FF FF <schema:u16> <namelen:u16> CEdge` — from the
/// name's offset, namelen is at −2, schema at −4, the `FF FF` tag at −6.
/// Version-map layout: UTF-16 string record for "CEdge" (`C\0E\0d\0g\0e\0`)
/// immediately followed by `schema:u32`. (The ASCII search cannot hit the
/// map entry and vice versa — different byte encodings.)
///
/// If the walker learns CEdge from EITHER declaration, it must see the new
/// number; the earlier version of this test doctored only the stream record
/// and the resync's fresh archive fell back to the still-pristine version
/// map — an inconsistent file no real schema change produces.
fn doctor_cedge_schema(d: &mut [u8], new_schema: u16) {
    // 1. Entity-stream new-class record (ASCII name).
    let name_off = d
        .windows(5)
        .position(|w| w == b"CEdge")
        .expect("ASCII CEdge new-class record present");
    assert_eq!(
        &d[name_off - 6..name_off - 4],
        &[0xFF, 0xFF],
        "expected the new-class tag before the schema word"
    );
    assert_eq!(
        u16::from_le_bytes([d[name_off - 2], d[name_off - 1]]),
        5,
        "expected namelen 5 before the name"
    );
    d[name_off - 4..name_off - 2].copy_from_slice(&new_schema.to_le_bytes());

    // 2. CVersionMap entry (UTF-16LE name, schema u32 follows).
    let utf16_name: &[u8] = &[b'C', 0, b'E', 0, b'd', 0, b'g', 0, b'e', 0];
    let map_off = d
        .windows(utf16_name.len())
        .position(|w| w == utf16_name)
        .expect("UTF-16 CEdge version-map entry present");
    let schema_at = map_off + utf16_name.len();
    assert_eq!(
        u32::from_le_bytes([
            d[schema_at],
            d[schema_at + 1],
            d[schema_at + 2],
            d[schema_at + 3]
        ]),
        2,
        "version map declares CEdge schema 2 in 2017 files"
    );
    d[schema_at..schema_at + 4].copy_from_slice(&(new_schema as u32).to_le_bytes());
}

#[test]
fn doctored_schema_downgrades_instead_of_misparsing() {
    let original = corpus("box.skp");

    // Control: the pristine file yields the box.
    let runs = openskp::geometry_runs(&original);
    assert!(
        runs.iter()
            .any(|r| r.topology.vertices == 8 && r.topology.edges == 12),
        "control: pristine box.skp must yield 8V/12E"
    );

    // Doctored: same file, CEdge's declared schema bumped to a number the
    // registry has never validated.
    let mut doctored = original.clone();
    doctor_cedge_schema(&mut doctored, 0x00EE);
    assert_ne!(original, doctored);

    // The walk must not panic, and must NOT reproduce the box as if the
    // 2017 CEdge layout had been confirmed — the reader was refused, so the
    // run degrades (stall/resync; typically filtered as degenerate).
    let runs = openskp::geometry_runs(&doctored);
    assert!(
        !runs
            .iter()
            .any(|r| r.topology.vertices == 8 && r.topology.edges == 12),
        "doctored schema must not walk to a confident 8V/12E box; got {runs:?}"
    );
}

#[test]
fn in_range_schema_still_reads_everything() {
    // Regression guard for the registry refactor itself: the whole corpus
    // suite (differential/topology) covers this too, but assert the flagship
    // file inline so this test file stands alone.
    let runs = openskp::geometry_runs(&corpus("box.skp"));
    assert_eq!(runs.len(), 1, "box.skp is exactly one geometry run");
    assert_eq!(runs[0].topology.faces, 6);
    assert_eq!(runs[0].resolved.0, runs[0].resolved.1, "fully resolved");
}
