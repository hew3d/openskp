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

fn names(m: &openskp::Model) -> Vec<String> {
    m.scenes.iter().map(|s| s.name.clone()).collect()
}

#[test]
fn a_scene_with_a_description_and_cleared_properties_is_found() {
    assert_eq!(names(&model("2017/scene-properties.skp")), ["Scene 1"]);
}

#[test]
fn a_scene_without_a_saved_camera_is_found() {
    // flags 0x7E: the record goes straight from the flags to the
    // rendering options, with no CCamera (SKP_FORMAT §10.10).
    assert_eq!(names(&model("2017/scene-no-camera.skp")), ["Scene 1"]);
}

#[test]
fn every_scene_of_a_multi_scene_file_is_found() {
    for rel in [
        "2017/two-scenes.skp",
        "2017/feature-pack.skp",
        "2017/house-plus.skp",
    ] {
        assert_eq!(names(&model(rel)), ["Scene 1", "Scene 2"], "{rel}");
    }
}

#[test]
fn files_without_scenes_report_none() {
    for rel in [
        "2017/box.skp",
        "2017/house.skp",
        "third-party/theater-2017.skp",
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
