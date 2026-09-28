//! Scene records are found by structure — the CViewPage tag and
//! preamble, name, description, properties-saved flags, then the first
//! saved property (SKP_FORMAT §10.10) — not by a fixed flags value.
//! scene-properties.skp holds a scene with a description and its
//! hidden-geometry property cleared (flags 0x0FEF); scene-no-camera.skp
//! a scene that saves no camera (flags 0x7E).

use std::path::PathBuf;

fn model(rel: &str) -> openskp::Model {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus");
    p.push(rel);
    openskp::Model::parse(&std::fs::read(p).unwrap()).unwrap()
}

/// `2017/<name>` and, when the corpus holds it, the file's 2026 twin.
fn twins(name: &str) -> Vec<String> {
    let mut out = vec![format!("2017/{name}")];
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2026");
    p.push(name);
    if p.exists() {
        out.push(format!("2026/{name}"));
    }
    out
}

fn names(m: &openskp::Model) -> Vec<String> {
    m.scenes.iter().map(|s| s.name.clone()).collect()
}

#[test]
fn a_scene_with_a_description_and_cleared_properties_is_found() {
    for rel in twins("scene-properties.skp") {
        assert_eq!(names(&model(&rel)), ["Scene 1"], "{rel}");
    }
}

#[test]
fn a_scene_without_a_saved_camera_is_found() {
    // flags 0x7E: the record goes straight from the flags to the
    // rendering options, with no CCamera (SKP_FORMAT §10.10).
    for rel in twins("scene-no-camera.skp") {
        assert_eq!(names(&model(&rel)), ["Scene 1"], "{rel}");
    }
}

#[test]
fn every_scene_of_a_multi_scene_file_is_found() {
    for name in ["two-scenes.skp", "feature-pack.skp", "house-plus.skp"] {
        for rel in twins(name) {
            assert_eq!(names(&model(&rel)), ["Scene 1", "Scene 2"], "{rel}");
        }
    }
}

#[test]
fn files_without_scenes_report_none() {
    for rel in [
        "2017/box.skp",
        "2026/box.skp",
        "2017/house.skp",
        "2026/house.skp",
        "third-party/theater-2017.skp",
        "third-party/theater-2026.skp",
    ] {
        assert!(model(rel).scenes.is_empty(), "{rel}");
    }
}

#[test]
fn a_scene_behind_an_escalated_class_reference_is_found() {
    // Past slot 0x7FFF a class reference becomes `7F FF` + u32 with the
    // high bit set (§11). Rewrite Scene 2's plain reference that way.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017/two-scenes.skp");
    let d = std::fs::read(p).unwrap();
    let name: Vec<u8> = "Scene 2"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut pat = vec![0, 0, 0, 0xff, 0xfe, 0xff, 7];
    pat.extend(name);
    let at = d.windows(pat.len()).position(|w| w == pat).unwrap();
    let tag = u16::from_le_bytes([d[at - 2], d[at - 1]]);
    assert_ne!(tag & 0x8000, 0);
    let mut big = d[..at - 2].to_vec();
    big.extend([0xff, 0x7f]);
    big.extend((0x8000_0000 | u32::from(tag & 0x7fff)).to_le_bytes());
    big.extend(&d[at..]);
    let m = openskp::Model::parse(&big).unwrap();
    assert_eq!(names(&m), ["Scene 1", "Scene 2"]);
}

#[test]
fn a_section_plane_reads_to_its_exact_extent() {
    // section-plane.skp: the plane is the last root element; with the exact
    // extent (no trailing u32) the root list still closes and the plane is
    // the authored horizontal cut at 0.5 m, in the root's frame.
    for rel in twins("section-plane.skp") {
        let m = model(&rel);
        let root = m.geometry.iter().find(|r| r.def_index.is_none()).unwrap();
        assert_eq!(root.sections.len(), 1, "{rel}");
        let [a, b, c, d] = root.sections[0].plane;
        assert!(a.abs() < 1e-12 && b.abs() < 1e-12 && (c.abs() - 1.0).abs() < 1e-12);
        assert!(
            (d.abs() - 0.5 * openskp::INCH).abs() < 1e-6,
            "{rel}: offset {d}"
        );
        assert!(
            !m.diagnostics.iter().any(|d| d.is_desync()),
            "{rel}: clean read"
        );
    }
}

#[test]
fn house_plus_scenes_name_hidden_entities_and_an_active_section_plane_by_pid() {
    for rel in twins("house-plus.skp") {
        let m = model(&rel);
        assert_eq!(m.scenes.len(), 2);
        let s1 = &m.scenes[0];
        assert_eq!(s1.hidden_entities.len(), 2);
        assert_eq!(
            s1.active_section_planes.len(),
            1,
            "Scene 1 has an active section cut"
        );
        // The active plane is one the geometry walk placed (root list or a
        // definition), found by persistent id, with a unit normal.
        let sp = m
            .geometry
            .iter()
            .flat_map(|r| r.sections.iter())
            .find(|p| p.pid == s1.active_section_planes[0])
            .expect("the scene's section plane is a walked CSectionPlane");
        let [a, b, c, _] = sp.plane;
        assert!(((a * a + b * b + c * c).sqrt() - 1.0).abs() < 1e-9);
        let s2 = &m.scenes[1];
        assert_eq!(s2.hidden_entities.len(), 2);
        assert!(s2.active_section_planes.is_empty());
        // Both scenes hide the same two entities, which are not placed
        // instances (hidden faces or annotations).
        assert_eq!(s1.hidden_entities, s2.hidden_entities);
        assert!(
            !m.instances
                .iter()
                .any(|i| s1.hidden_entities.contains(&i.pid)),
            "the hidden entities are not placed instances"
        );
    }
}
