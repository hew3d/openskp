//! Phase 0.4 acceptance: clean corpus files parse with ZERO desync
//! diagnostics (standing regression), and a corrupted stream produces
//! recorded diagnostics — a resync that recovers the walk — instead of a
//! silent short read.

use std::path::{Path, PathBuf};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/2017")
}

fn corpus(name: &str) -> Vec<u8> {
    let p = corpus_dir().join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// THE standing regression: every 2017 corpus file walks without a single
/// desync diagnostic (Resync/Truncated/Skipped). Informational RunFiltered
/// entries are allowed — thumbnail false-positives and dynamic-component
/// fragments are filtered BY DESIGN on clean files.
///
/// The old `attributes.skp` exception (a pinned Resync histogram of
/// 857 x #107 + 8 x #150 + 1 x CCurve, with the note "later class
/// coverage should shrink these to zero") is CLOSED: the §4s continuous
/// walk reads the whole model section — the per-entity attribute pointer
/// (#107 was the CAttributeContainer class), the typed-array 0x0b values,
/// and the CCurve body are all decoded now, so attributes.skp holds the
/// same zero-desync bar as every other clean file.
///
/// Pre-2017 files (`box-v2013..16`) must PARSE, but their body layouts are
/// Phase 5.3 territory: until then, walking them may record degradation
/// diagnostics — which is the plan's 5.3 bar ("graceful, diagnosed
/// degradation"), loud instead of silent. What is NOT acceptable for them
/// is a panic or a parse error.
#[test]
fn clean_corpus_has_zero_desync_diagnostics() {
    let mut checked_2017 = 0;
    for entry in std::fs::read_dir(corpus_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("skp") {
            continue; // .dae ground truth, subdirs (future/ is excluded by read_dir being non-recursive)
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let d = std::fs::read(&path).unwrap();
        let m = openskp::Model::parse(&d).unwrap_or_else(|e| panic!("{name}: {e}"));
        if !m.version.starts_with("{17.") {
            continue; // pre-2017: parse-without-panic is the current bar
        }
        let desync: Vec<openskp::Diagnostic> = m
            .diagnostics
            .iter()
            .filter(|di| di.is_desync())
            .cloned()
            .collect();
        assert!(
            desync.is_empty(),
            "{name}: expected zero desync diagnostics on a clean file, got {desync:?}"
        );
        checked_2017 += 1;
    }
    assert!(
        checked_2017 > 30,
        "corpus glob looks wrong ({checked_2017} 2017 files)"
    );
}

/// Find the user-entity headers (class-ref tag + `00 00 03` + pid), the same
/// shape the walker keys on, so the corruption lands inside the geometry run.
fn user_entity_headers(d: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 7 <= d.len() {
        let v = u16::from_le_bytes([d[off], d[off + 1]]);
        if (v & 0x8000) != 0 && d[off + 2] == 0 && d[off + 3] == 0 && d[off + 4] == 3 {
            let pid = u16::from_le_bytes([d[off + 5], d[off + 6]]);
            if pid > 0x1281 {
                out.push(off);
                off += 7;
                continue;
            }
        }
        off += 1;
    }
    out
}

#[test]
fn corrupted_stream_resyncs_and_records_diagnostics() {
    let original = corpus("box.skp");
    let headers = user_entity_headers(&original);
    assert!(headers.len() > 4, "expected a dense entity cluster");

    // Corrupt the interior of a mid-run CEdge: stamp an FFFF new-class tag
    // over its first vertex pointer so the walker reads a garbage class
    // definition and stalls there (bad/over-long class name). A CEdge with
    // an inline first vertex is identified structurally: its v0 slot at
    // +17 (header 7 + drawbase 10) is ITSELF a header (the inline CVertex's
    // tag + `00 00 03` + pid) — no other record shape has that.
    let header_set: std::collections::HashSet<usize> = headers.iter().copied().collect();
    let mut doctored = original.clone();
    let h = headers
        .iter()
        .copied()
        .skip(1)
        .find(|&h| header_set.contains(&(h + 17)))
        .expect("a CEdge with an inline first vertex inside the run");
    doctored[h + 17..h + 19].copy_from_slice(&[0xFF, 0xFF]);

    let m = openskp::Model::parse(&doctored).expect("corrupted stream must still parse");
    let resyncs: Vec<_> = m
        .diagnostics
        .iter()
        .filter(|di| matches!(di, openskp::Diagnostic::Resync { .. }))
        .collect();
    assert!(
        !resyncs.is_empty(),
        "expected at least one recorded Resync; diagnostics: {:?}",
        m.diagnostics
    );

    // And the recovery is real: the walk continued past the corruption
    // instead of ending the run at the stall (the run still sees most of
    // the box's vertices).
    let total_vertices: usize = m.geometry.iter().map(|r| r.topology.vertices).sum();
    assert!(
        total_vertices >= 4,
        "resync should recover the rest of the run; got {} vertices",
        total_vertices
    );

    // The desync surfaces in the JSON too (the oracle-safe conditional key).
    assert!(
        m.to_json().contains("\"diagnostics\""),
        "desync diagnostics must appear in the JSON"
    );
}
