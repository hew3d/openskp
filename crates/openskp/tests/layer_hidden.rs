//! Phase 3.4 acceptance: per-entity hidden flag and entity→layer binding
//! (drawbase byte[2] and bytes[8..10], SKP_FORMAT §4q). Oracles: the hidden
//! geometry is ABSENT from hidden-entities.dae (by-construction), and
//! layers.skp's three boxes have distinguishable authored sizes.
//!
//! §4s update: these files parse through the CONTINUOUS walk, whose model
//! includes the TEMPLATE definition runs (the default-template figure —
//! its own hidden/soft construction geometry included) ahead of the USER
//! drawing. The user drawing is the ROOT entity list = the LAST run, so
//! the assertions target that run instead of `geometry[0]`.

use std::path::PathBuf;

fn model(name: &str) -> openskp::Model {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    openskp::Model::parse(&std::fs::read(p).unwrap()).unwrap()
}

/// The user drawing's run: the model-root entity list (last on the
/// continuous path; single-run legacy files degenerate to the same).
fn user_run(m: &openskp::Model) -> &openskp::GeometryRun {
    m.geometry.last().expect("at least one geometry run")
}

#[test]
fn hidden_entities_flags() {
    let m = model("hidden-entities.skp");
    let mesh = &user_run(&m).mesh;
    assert_eq!(
        mesh.edges.iter().filter(|e| e.hidden).count(),
        1,
        "exactly the one authored hidden edge"
    );
    assert_eq!(
        mesh.faces.iter().filter(|f| f.hidden).count(),
        1,
        "exactly the one authored hidden face"
    );
    // NOTE: hidden-entities.dae exports all 12 face copies — this export
    // INCLUDED hidden geometry, so .dae-absence is NOT usable as the
    // oracle here. The evidence is the minimal-pair byte diff instead:
    // hidden-entities.skp vs box.skp differ in drawbase byte[2] on exactly
    // the two authored entities (SKP_FORMAT §4q).

    // And plain box.skp's drawing has nothing hidden.
    let plain = model("box.skp");
    let pm = &user_run(&plain).mesh;
    assert!(pm.edges.iter().all(|e| !e.hidden));
    assert!(pm.faces.iter().all(|f| !f.hidden));
}

#[test]
fn layers_entity_binding() {
    let m = model("layers.skp");
    assert_eq!(
        m.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
        ["Layer0", "BigLayer", "MediumLayer", "LittleLayer"]
    );
    assert!(!m.layers[2].visible, "MediumLayer was hidden by the author");

    // The three boxes are distinguishable by their authored sizes; every
    // edge and face of a box must sit on that box's layer.
    let run = user_run(&m);
    assert_eq!(run.mesh.edges.len(), 36, "the three drawn boxes");
    let expect = |x_min: f64| -> &str {
        // box1 spans [0,1] m, box2 [1.6,2.1] m, box3 [2.61,2.86] m —
        // classify by the entity's minimum x with box-gap thresholds.
        if x_min < 1.5 {
            "BigLayer"
        } else if x_min < 2.5 {
            "MediumLayer"
        } else {
            "LittleLayer"
        }
    };
    for e in &run.mesh.edges {
        let x = run.mesh.vertices[e.v0 as usize][0].min(run.mesh.vertices[e.v1 as usize][0]);
        let layer = m.layer_of(e.layer).expect("edge layer resolves");
        assert_eq!(layer.name, expect(x), "edge at x={x}");
    }
    for f in &run.mesh.faces {
        let x = f
            .outer
            .iter()
            .map(|&v| run.mesh.vertices[v as usize][0])
            .fold(f64::INFINITY, f64::min);
        let layer = m.layer_of(f.layer).expect("face layer resolves");
        assert_eq!(layer.name, expect(x), "face at x={x}");
    }
}

#[test]
fn default_layer_binding() {
    let m = model("box.skp");
    let run = user_run(&m);
    assert_eq!(run.mesh.edges.len(), 12, "the drawn cube");
    for e in &run.mesh.edges {
        assert_eq!(e.layer, 0);
        assert_eq!(m.layer_of(e.layer).unwrap().name, "Layer0");
    }
}
