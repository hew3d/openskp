//! Phase 3.2 acceptance: the three material oracles link the named/colored
//! material to the correct face SIDE, with ground truth derived from the
//! `.dae` exports (see SKP_FORMAT §4n — back-material's front side was
//! identified by matching exported corner normals to the face's decoded
//! plane normal).

use std::path::PathBuf;

fn model(name: &str) -> openskp::Model {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    openskp::Model::parse(&std::fs::read(p).unwrap()).unwrap()
}

fn rgba(m: &openskp::Material) -> [u8; 4] {
    match m {
        openskp::Material::Solid { rgba, .. } => *rgba,
        other => panic!("expected a solid material, got {other:?}"),
    }
}

/// The only painted face: front side carries "*1" (red).
#[test]
fn paint_one_face_front_is_red() {
    let m = model("paint-one-face.skp");
    let face = m
        .geometry
        .iter()
        .flat_map(|r| r.mesh.faces.iter())
        .find(|f| f.front_material.is_some())
        .unwrap();
    let mat = m.material_of(face.front_material.unwrap()).unwrap();
    assert_eq!(rgba(mat), [255, 0, 0, 255]);
    assert!(face.back_material.is_none());
}

/// One face, BOTH sides painted. The .dae proves the assignment: the side
/// whose exported normals equal the decoded plane normal (the front) is
/// blue "*2"; the back is red "*1".
#[test]
fn back_material_sides_match_dae() {
    let m = model("back-material.skp");
    let face = m
        .geometry
        .iter()
        .flat_map(|r| r.mesh.faces.iter())
        .find(|f| f.back_material.is_some())
        .unwrap();
    let front = m.material_of(face.front_material.unwrap()).unwrap();
    let back = m.material_of(face.back_material.unwrap()).unwrap();
    assert_eq!(rgba(front), [0, 0, 255, 255], "front side is the blue *2");
    assert_eq!(rgba(back), [255, 0, 0, 255], "back side is the red *1");
}

/// Two faces painted with two different materials: the -Y face carries the
/// first (red-ish) material, the +Z face the second (green).
#[test]
fn box_two_materials_faces() {
    let m = model("box-two-materials.skp");
    for f in m.geometry.iter().flat_map(|r| r.mesh.faces.iter()) {
        let Some(slot) = f.front_material else {
            continue;
        };
        let mat = rgba(m.material_of(slot).unwrap());
        if f.normal[2] > 0.9 {
            assert_eq!(mat, [63, 127, 2, 255], "+Z face is green");
        } else if f.normal[1] < -0.9 {
            assert_eq!(mat, [251, 1, 6, 255], "-Y face is red");
        } else {
            panic!("unexpected painted face normal {:?}", f.normal);
        }
    }
}

/// A textured material links exactly like a solid one.
#[test]
fn material_one_face_links_the_texture() {
    let m = model("material-one-face.skp");
    let face = m
        .geometry
        .iter()
        .flat_map(|r| r.mesh.faces.iter())
        .find(|f| f.front_material.is_some())
        .unwrap();
    match m.material_of(face.front_material.unwrap()).unwrap() {
        openskp::Material::Textured { name, .. } => assert_eq!(name, "[Wood Floor Light]"),
        other => panic!("expected the textured material, got {other:?}"),
    }
}

/// The stored opacity f64 applies ONLY when the use-opacity flag byte is
/// set: attributes.skp's *2 and *4 both store 0.5, but only *4 carries the
/// flag — SketchUp's own `.dae` export gives *2 diffuse alpha 1.0 and *4
/// alpha 128/255. Stale slider values with the flag off must read opaque.
#[test]
fn opacity_applies_only_with_the_use_opacity_flag() {
    let model = model("attributes.skp");
    let opacity_of = |want: &str| -> f64 {
        model
            .materials
            .iter()
            .find_map(|m| match m {
                openskp::Material::Solid { name, opacity, .. } if name == want => Some(*opacity),
                _ => None,
            })
            .unwrap_or_else(|| panic!("solid material {want:?}"))
    };
    assert_eq!(opacity_of("*2"), 1.0, "flag off: stored 0.5 reads opaque");
    assert_eq!(opacity_of("*4"), 0.5, "flag on: stored 0.5 applies");
}
