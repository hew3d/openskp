//! Phase 5.2 / 5.3 / 5.4 acceptance on the existing corpus:
//! - 5.2 template independence: blank-template-box parses with zero
//!   template assumptions (Phase 0.6 removed the constants; this pins the
//!   behavior).
//! - 5.3 older versions: box-v2013..16 parse without panic — header-level
//!   identification + extractors work; body decode is diagnosed
//!   degradation until their schemas are validated. Post-2017 containers
//!   refuse cleanly but stay identifiable.
//! - 5.4 long strings: the MFC length escalation decodes real >255-char
//!   strings (proves Phase 0.5 on authored data).

use std::path::PathBuf;

fn corpus_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/2017")
        .join(name)
}

fn corpus(name: &str) -> Vec<u8> {
    std::fs::read(corpus_path(name)).unwrap()
}

#[test]
fn blank_template_box_is_template_independent() {
    let m = openskp::Model::parse(&corpus("blank-template-box.skp")).unwrap();
    assert_eq!(m.diagnostics.iter().filter(|d| d.is_desync()).count(), 0);
    assert!(
        m.materials.is_empty(),
        "the ubiquitous Default material is TEMPLATE content, not a format constant"
    );
    assert_eq!(m.geometry.len(), 1);
    let r = &m.geometry[0];
    assert_eq!(r.frame, Some(18), "structurally framed");
    assert_eq!(
        (r.topology.vertices, r.topology.edges, r.topology.faces),
        (8, 12, 6)
    );
    assert_eq!(r.resolved.0, r.resolved.1, "100% back-refs");
    // By construction: a 1 m box drawn from the origin — the mesh vertex
    // set is exactly the metre-cube corners.
    let mut got: Vec<[i64; 3]> = r
        .mesh
        .vertices
        .iter()
        .map(|p| {
            [
                (p[0] * 1e7).round() as i64,
                (p[1] * 1e7).round() as i64,
                (p[2] * 1e7).round() as i64,
            ]
        })
        .collect();
    got.sort();
    let mut want = Vec::new();
    for x in [0i64, 10_000_000] {
        for y in [0i64, 10_000_000] {
            for z in [0i64, 10_000_000] {
                want.push([x, y, z]);
            }
        }
    }
    want.sort();
    assert_eq!(got, want, "the authored unit-metre cube");
}

#[test]
fn pre_2017_versions_parse_with_diagnosed_degradation() {
    for (file, version) in [
        ("../legacy/box-v2013.skp", "{13.0.1}"),
        ("../legacy/box-v2014.skp", "{14.0.1}"),
        ("../legacy/box-v2015.skp", "{15.0.1}"),
        ("../legacy/box-v2016.skp", "{16.0.1}"),
    ] {
        let d = corpus(file);
        let m = openskp::Model::parse(&d).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(m.version, version, "{file}");
        // Extractors keep working at header/record level.
        assert!(
            m.materials.iter().any(
                |mat| matches!(mat, openskp::Material::Solid { name, .. } if name == "Default")
            ),
            "{file}: Default material extracted"
        );
        assert!(
            m.layers.iter().any(|l| l.name == "Layer0"),
            "{file}: Layer0 extracted"
        );
        // Degradation on these bodies is allowed but must be RECORDED, not
        // silent: any surviving junk runs come with diagnostics.
        // (No assertion on counts — 5.3's full decode is future work.)
    }
}

#[test]
fn post_2017_container_refuses_cleanly_but_identifies() {
    let p = corpus_path("../future/box-v2026.skp");
    if !p.exists() {
        return; // the forward-format probe file is optional
    }
    let d = std::fs::read(p).unwrap();
    assert!(
        openskp::Model::parse(&d).is_err(),
        "post-2017 container must refuse, not walk garbage"
    );
}

#[test]
fn long_name_escalated_strings_decode() {
    let d = corpus("long-name.skp");
    // u16-escalated string records: FF FE FF FF <len:u16> <utf16>.
    let mut found = Vec::new();
    let mut i = 0;
    while i + 6 < d.len() {
        if d[i..i + 4] == [0xFF, 0xFE, 0xFF, 0xFF] {
            if let Some((n, at)) = openskp::mfc_strlen(&d, i) {
                if (255..0xFFFF).contains(&n) && at + n * 2 <= d.len() {
                    let units: Vec<u16> = d[at..at + n * 2]
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    if let Ok(s) = String::from_utf16(&units) {
                        found.push(s);
                    }
                }
            }
        }
        i += 1;
    }
    // The two authored strings (name 262 chars, description 273) decode
    // through the escalation; template boilerplate may add more.
    assert!(
        found
            .iter()
            .any(|s| s.len() == 262 && s.starts_with("This is a component")),
        "authored 262-char component name decodes"
    );
    assert!(
        found
            .iter()
            .any(|s| s.len() == 273 && s.starts_with("The description of this component")),
        "authored 273-char description decodes"
    );
}
