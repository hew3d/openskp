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

/// A shared-texture record (§8.1) carries no inline CDib — its u16
/// back-ref names the owning material's dib slot. house.skp's
/// "[Wood Floor Light]1" refs slot 23, the dib of "[Wood Floor Light]"
/// at slot 22; extract_images.py pins that payload as [3] jpg @0x012daa,
/// 17984 bytes. The shared material must expose the owner's exact bytes.
#[test]
fn shared_texture_material_resolves_the_owners_bytes() {
    let m = model("house.skp");
    let bytes_of = |want: &str| {
        m.materials
            .iter()
            .find_map(|mat| match mat {
                openskp::Material::Textured {
                    name, image_bytes, ..
                } if name == want => Some(image_bytes.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("textured material {want:?}"))
    };
    let owner = bytes_of("[Wood Floor Light]").expect("owner carries inline bytes");
    let shared = bytes_of("[Wood Floor Light]1").expect("shared back-ref resolves");
    assert_eq!(owner.len(), 17984);
    assert_eq!(&owner[..4], &[0xff, 0xd8, 0xff, 0xe0], "JPEG SOI + APP0");
    assert_eq!(&owner[owner.len() - 2..], &[0xff, 0xd9], "JPEG EOI");
    assert_eq!(shared, owner, "shared material adopts the owner's payload");
}

#[test]
fn solid_materials_have_no_image() {
    let m = model("box-two-materials.skp");
    assert!(m
        .materials
        .iter()
        .all(|mat| matches!(mat, openskp::Material::Solid { .. })));
}
