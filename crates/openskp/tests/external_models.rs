//! Production-scale models that cannot ship in the corpus (third-party
//! files without redistribution rights). Point `OPENSKP_EXTERNAL_MODELS`
//! at a directory of `.skp` files to run; without it the test passes
//! vacuously. Every 2017 file there must read on the continuous walk
//! without a desync, and every `<name> (2026).skp` next to a `<name>.skp`
//! must read equivalent to it (geometry counts, definitions, materials,
//! layers, scenes with their hidden layers, section planes).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use openskp::{Container, Model};

fn models() -> Vec<PathBuf> {
    let Some(dir) = std::env::var_os("OPENSKP_EXTERNAL_MODELS") else {
        eprintln!("OPENSKP_EXTERNAL_MODELS unset: external models skipped");
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("OPENSKP_EXTERNAL_MODELS is a readable directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "skp"))
        .filter(|p| !p.to_string_lossy().ends_with("~.skp"))
        .collect();
    v.sort();
    v
}

fn parse(p: &Path) -> Model {
    let data = std::fs::read(p).unwrap();
    Model::parse(&data).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn every_external_2017_model_reads_on_the_continuous_walk() {
    for p in models() {
        let m = parse(&p);
        if m.container != Container::Carchive2017 {
            continue;
        }
        let bad: Vec<_> = m.diagnostics.iter().filter(|d| d.is_desync()).collect();
        assert!(
            bad.is_empty(),
            "{}: {} desync diagnostic(s), first {:?}",
            p.display(),
            bad.len(),
            bad.first()
        );
        assert!(!m.definitions.is_empty(), "{}: no definitions", p.display());
        assert!(m.camera.is_some(), "{}: no camera", p.display());
    }
}

#[test]
fn every_external_2026_conversion_reads_equivalent_to_its_2017_original() {
    let all = models();
    for new in &all {
        let name = new.file_name().unwrap().to_string_lossy().into_owned();
        let Some(base) = name.strip_suffix(" (2026).skp") else {
            continue;
        };
        let old = new.with_file_name(format!("{base}.skp"));
        if !old.exists() {
            continue;
        }
        let (a, b) = (parse(&old), parse(new));
        assert_eq!(a.container, Container::Carchive2017, "{}", old.display());
        assert_eq!(b.container, Container::Zip, "{}", new.display());
        let what = new.display();

        let names =
            |m: &Model| -> Vec<String> { m.definitions.iter().map(|d| d.name.clone()).collect() };
        assert_eq!(names(&a), names(&b), "{what}: definitions");
        assert_eq!(a.instances.len(), b.instances.len(), "{what}: instances");
        let pids = |m: &Model| -> BTreeSet<u32> { m.instances.iter().map(|i| i.pid).collect() };
        assert_eq!(pids(&a), pids(&b), "{what}: instance pids");
        let counts = |m: &Model| -> (usize, usize, usize, usize) {
            m.geometry.iter().fold((0, 0, 0, 0), |acc, r| {
                (
                    acc.0 + r.topology.vertices,
                    acc.1 + r.topology.edges,
                    acc.2 + r.topology.faces,
                    acc.3 + r.mesh.faces.iter().filter(|f| f.texture.is_some()).count(),
                )
            })
        };
        assert_eq!(
            counts(&a),
            counts(&b),
            "{what}: vertices/edges/faces/textured"
        );
        assert_eq!(a.materials.len(), b.materials.len(), "{what}: materials");
        let layers =
            |m: &Model| -> Vec<String> { m.layers.iter().map(|l| l.name.clone()).collect() };
        assert_eq!(layers(&a), layers(&b), "{what}: layers");

        assert_eq!(a.scenes.len(), b.scenes.len(), "{what}: scenes");
        for (sa, sb) in a.scenes.iter().zip(&b.scenes) {
            assert_eq!(sa.name, sb.name, "{what}: scene name");
            assert_eq!(
                sa.saved.0 & 0x7F,
                sb.saved.0 & 0x7F,
                "{what}: {}: saved",
                sa.name
            );
            let hl = |m: &Model, idx: &[usize]| -> Vec<String> {
                idx.iter().map(|&i| m.layers[i].name.clone()).collect()
            };
            assert_eq!(
                hl(&a, &sa.hidden_layers),
                hl(&b, &sb.hidden_layers),
                "{what}: {}: hidden layers",
                sa.name
            );
            assert_eq!(
                sa.active_section_planes.len(),
                sb.active_section_planes.len(),
                "{what}: {}: active section planes",
                sa.name
            );
        }
        let planes = |m: &Model| -> Vec<[i64; 4]> {
            let mut v: Vec<[i64; 4]> = m
                .geometry
                .iter()
                .flat_map(|r| r.sections.iter())
                .map(|s| s.plane.map(|x| (x * 1e6).round() as i64))
                .collect();
            v.sort();
            v
        };
        assert_eq!(planes(&a), planes(&b), "{what}: section planes");

        // Guides name their owning definition on both paths; compare them
        // by (owner name, direction), since owners are identified by
        // path-specific indices.
        let guides = |m: &Model| -> Vec<(Option<String>, [i64; 3])> {
            let owner = |i: Option<usize>| {
                i.map(|i| {
                    m.definitions
                        .iter()
                        .find(|d| d.map_index == Some(i))
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| format!("unresolved owner {i}"))
                })
            };
            let mut v: Vec<_> = m
                .guides
                .iter()
                .map(|g| {
                    (
                        owner(g.def_index),
                        g.direction.map(|x| (x * 1e4).round() as i64),
                    )
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(guides(&a), guides(&b), "{what}: guides");
    }
}
