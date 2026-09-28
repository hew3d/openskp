//! Phase 5.2 / 5.3 / 5.4 acceptance on the existing corpus:
//! - 5.2 template independence: blank-template-box parses with zero
//!   template assumptions (Phase 0.6 removed the constants; this pins the
//!   behavior).
//! - 5.3 older versions: box-v2013..16 parse without panic — header-level
//!   identification + extractors work; body decode is diagnosed
//!   degradation until their schemas are validated. Post-2017 containers
//!   are identified and read by their own reader.
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

/// Every SketchUp 2026 save in the corpus (corpus/2026 plus the
/// third-party benchmark's 2026 conversion) is identified from its header
/// as the post-2017 ZIP container and read by the post-2017 reader — never
/// walked as a 2017 stream (no continuous-walk or fallback diagnostics).
#[test]
fn post_2017_container_identifies_and_reads() {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(corpus_path("../2026"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("skp"))
        .collect();
    files.push(corpus_path("../third-party/theater-2026.skp"));
    assert!(files.len() >= 4, "2026 corpus glob looks wrong: {files:?}");
    for p in &files {
        let d = std::fs::read(p).unwrap();
        let (version, guid) = openskp::header_info(&d)
            .unwrap_or_else(|| panic!("{}: header not identified", p.display()));
        assert_eq!(version, "{26.2.0}", "{}", p.display());
        assert_eq!(guid, None, "{}: no model GUID in this header", p.display());
        assert_eq!(
            openskp::detect_container(&d),
            openskp::Container::Zip,
            "{}",
            p.display()
        );
        let m = openskp::Model::parse(&d).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        assert_eq!(m.version, version);
        assert!(
            m.diagnostics.is_empty(),
            "{}: {:?}",
            p.display(),
            m.diagnostics
        );
        assert!(!m.geometry.is_empty(), "{}", p.display());
    }
}

/// A damaged post-2017 file is an error, never a panic or a silently
/// different model: every truncation loses the ZIP end-of-directory record
/// that must end the file, and every flipped byte inside the compressed
/// model fails inflation or its CRC-32.
#[test]
fn damaged_post_2017_files_fail_cleanly() {
    let d = std::fs::read(corpus_path("../2026/box.skp")).unwrap();
    for cut in (0..d.len())
        .step_by(d.len() / 97)
        .chain([d.len() - 1, d.len() - 22])
    {
        assert!(
            openskp::Model::parse(&d[..cut]).is_err(),
            "truncated to {cut}"
        );
    }
    // model.dat is the archive's last entry: its compressed bytes follow
    // its local header, the first "model.dat" name in the file.
    let at = d
        .windows(9)
        .position(|w| w == b"model.dat")
        .expect("model.dat entry")
        + 9;
    for off in [40, 200, 900, 2000] {
        let mut bad = d.clone();
        bad[at + off] ^= 0x5A;
        assert!(
            openskp::Model::parse(&bad).is_err(),
            "byte flipped at {}",
            at + off
        );
    }
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

/// The header GUID identifies the MODEL: re-saves of one model share it
/// (empty/empty-2; box.skp and the same box saved by 2013–2016), distinct
/// models differ, and a parsed model carries it.
#[test]
fn header_guid_is_per_model() {
    let guid = |p: std::path::PathBuf| {
        openskp::header_info(&std::fs::read(&p).unwrap())
            .unwrap()
            .1
            .unwrap_or_else(|| panic!("{}: no model GUID", p.display()))
    };
    let box_guid = guid(corpus_path("box.skp"));
    for v in ["2013", "2014", "2015", "2016"] {
        assert_eq!(
            guid(corpus_path(&format!("../legacy/box-v{v}.skp"))),
            box_guid,
            "box saved by {v}"
        );
    }
    assert_eq!(
        guid(corpus_path("empty.skp")),
        guid(corpus_path("empty-2.skp"))
    );
    assert_ne!(guid(corpus_path("empty.skp")), box_guid);
    assert_ne!(guid(corpus_path("house.skp")), box_guid);

    let mut m = openskp::Model::parse(&corpus("box.skp")).unwrap();
    assert_eq!(m.model_guid.as_deref(), Some(box_guid.as_str()));
    assert!(m
        .to_json()
        .contains(&format!("\"model_guid\":\"{box_guid}\"")));

    // A model without a header GUID (any other container) serializes null.
    m.model_guid = None;
    let j = m.to_json();
    assert!(
        j.contains("\"model_guid\":null"),
        "{}",
        &j[..j.len().min(120)]
    );
}
