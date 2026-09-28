//! feature-pack.skp acceptance: one root list holding every annotation
//! kind back to back — guide line, guide point, section plane, leader +
//! screen text, linear dimension, placed image — after a box and two
//! imported components. The single-feature files each held their
//! annotation as the LAST root entity, where a body reader that
//! over-consumes eats zeros of the root tail unnoticed; here each
//! annotation is followed by the next one's new-class record, so every
//! body length is pinned exactly (a u32 misread as the CConstructionPoint
//! u8 tail once silently dropped the five root entities after it).

use std::path::PathBuf;

/// corpus-relative paths to check: the 2017 file, plus its 2026 twin when
/// the corpus carries one (corpus/2026/<name> is the SketchUp web app's
/// conversion of corpus/2017/<name>, so the same expectations apply).
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

fn model(rel: &str) -> openskp::Model {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus");
    p.push(rel);
    openskp::Model::parse(&std::fs::read(p).unwrap()).unwrap()
}

#[test]
fn every_root_annotation_is_read() {
    for rel in twins("feature-pack.skp") {
        let m = model(&rel);
        let is_2026 = rel.starts_with("2026/");
        if !is_2026 {
            // 2017-only diagnostic: which walk path served the file.
            assert!(
                m.diagnostics
                    .iter()
                    .any(|d| matches!(d, openskp::Diagnostic::ContinuousWalk { .. })),
                "{rel}: continuous path; diagnostics: {:?}",
                m.diagnostics
            );
        }
        assert_eq!(
            m.diagnostics.iter().filter(|d| d.is_desync()).count(),
            0,
            "{rel}"
        );

        let root = m.geometry.last().expect("root run");
        assert!(root.def_index.is_none(), "{rel}");
        let t = &root.topology;
        assert_eq!(
            (t.vertices, t.edges, t.faces),
            (8, 12, 6),
            "{rel}: Box A is the only loose root geometry"
        );
        assert_eq!(t.construction_points, 1, "{rel}: the guide point");
        assert_eq!(t.section_planes, 1, "{rel}: the section plane");
        assert_eq!(t.texts, 2, "{rel}: leader text + screen text");
        assert_eq!(t.dimensions, 1, "{rel}: the linear dimension");
        assert_eq!(t.images_placed, 1, "{rel}: the placed image");

        // Annotations live in the root list only.
        let elsewhere: usize = m.geometry[..m.geometry.len() - 1]
            .iter()
            .map(|r| {
                let t = &r.topology;
                t.construction_points + t.section_planes + t.texts + t.dimensions + t.images_placed
            })
            .sum();
        assert_eq!(elsewhere, 0, "{rel}");

        assert_eq!(m.guides.len(), 1, "{rel}: the tape-measure guide line");
        let g = &m.guides[0];
        assert_eq!(g.point_m[1], 1.0, "{rel}: 1 m off the red axis");
        assert_eq!(g.point_m[2], 0.0, "{rel}");
        assert_eq!(g.direction, [1.0, 0.0, 0.0], "{rel}: runs along +X");

        let names: Vec<&str> = m.scenes.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Scene 1", "Scene 2"], "{rel}");
        for name in ["pin-fixed-distort", "attributes", "DynamicHorizontalBlind"] {
            assert!(
                m.definitions.iter().any(|d| d.name == name),
                "{rel}: imported definition {name:?}"
            );
        }
    }
}

#[test]
fn box_a_edge_and_face_flags() {
    for rel in twins("feature-pack.skp") {
        let m = model(&rel);
        let mesh = &m.geometry.last().unwrap().mesh;
        let at = |i: u32| mesh.vertices[i as usize];
        let vertical_at = |x: f64, y: f64| {
            mesh.edges
                .iter()
                .find(|e| {
                    let (a, b) = (at(e.v0), at(e.v1));
                    a[0] == x && a[1] == y && b[0] == x && b[1] == y
                })
                .unwrap_or_else(|| panic!("{rel}: vertical edge at ({x}, {y})"))
        };
        let soft = vertical_at(0.0, 0.0);
        assert!(
            soft.soft && soft.smooth && !soft.hidden,
            "{rel}: origin edge: soft + smooth"
        );
        let hidden = vertical_at(1.0, 1.0);
        assert!(
            hidden.hidden && !hidden.soft && !hidden.smooth,
            "{rel}: (1 m, 1 m) edge: hidden"
        );
        assert_eq!(
            mesh.edges.iter().filter(|e| e.soft || e.smooth).count(),
            1,
            "{rel}"
        );
        assert_eq!(mesh.edges.iter().filter(|e| e.hidden).count(), 1, "{rel}");

        let hidden_faces: Vec<_> = mesh.faces.iter().filter(|f| f.hidden).collect();
        assert_eq!(hidden_faces.len(), 1, "{rel}: only the top face is hidden");
        assert_eq!(hidden_faces[0].normal, [0.0, 0.0, 1.0], "{rel}");
        assert!(
            hidden_faces[0].outer.iter().all(|&i| at(i)[2] == 1.0),
            "{rel}: at z = 1 m"
        );
    }
}
