//! Phase 3.5 acceptance: each textured material exposes its embedded image
//! byte-for-byte. Oracle: tools/extract_images.py (frozen) carves the same
//! payloads at the same offsets — its output for these two files is pinned
//! below by length + head/tail bytes (captured from the corpus files).

use std::path::PathBuf;

fn model(name: &str) -> openskp::Model {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    openskp::Model::parse(&std::fs::read(p).unwrap()).unwrap()
}

fn textured_bytes(m: &openskp::Model) -> Vec<u8> {
    m.materials
        .iter()
        .find_map(|mat| match mat {
            openskp::Material::Textured { image_bytes, .. } => image_bytes.clone(),
            _ => None,
        })
        .expect("a textured material with image bytes")
}

#[test]
fn png_texture_material_image() {
    // extract_images.py: [2] png @0x003f0e, 1437 bytes — the material's
    // inline CDib (thumbnails are [0]/[1], the figure texture [3]).
    let b = textured_bytes(&model("png-texture.skp"));
    assert_eq!(b.len(), 1437);
    assert_eq!(
        &b[..16],
        &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52]
    );
    assert_eq!(
        &b[b.len() - 16..],
        &[0x84, 0x58, 0xec, 0xef, 0, 0, 0, 0, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82]
    );
}

#[test]
fn material_one_face_material_image() {
    // extract_images.py: [2] jpg @0x00391c, 17984 bytes.
    let b = textured_bytes(&model("material-one-face.skp"));
    assert_eq!(b.len(), 17984);
    assert_eq!(&b[..4], &[0xff, 0xd8, 0xff, 0xe0], "JPEG SOI + APP0");
    assert_eq!(&b[b.len() - 2..], &[0xff, 0xd9], "JPEG EOI");
}

#[test]
fn solid_materials_have_no_image() {
    let m = model("box-two-materials.skp");
    assert!(m
        .materials
        .iter()
        .all(|mat| matches!(mat, openskp::Material::Solid { .. })));
}
