//! `is_group` acceptance: the placing entity's class (`CGroup` vs
//! `CComponentInstance`, §4s) must reach BOTH public surfaces that carry
//! per-instance identity — `Model::instances` (pre-existing) and
//! `Model::scene()` nodes (added alongside `PlacedInstance::is_group`).
//! Neither had a test pinning the actual true/false values against known
//! source classes before this file; `linkage.rs` only exercises the
//! def-ref/name side of these same fixtures.

use std::path::PathBuf;

fn corpus(name: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    std::fs::read(p).unwrap()
}

#[test]
fn instance_is_group_matches_source_class() {
    for (file, want) in [
        ("group.skp", Some(true)),
        ("box-group.skp", Some(true)),
        ("box-component.skp", Some(false)),
    ] {
        let m = openskp::Model::parse(&corpus(file)).unwrap();
        assert_eq!(m.instances.len(), 1, "{file}: one placed instance");
        assert_eq!(
            m.instances[0].is_group, want,
            "{file}: Instance::is_group"
        );
    }
}

#[test]
fn scene_node_is_group_matches_source_class() {
    for (file, want) in [
        ("group.skp", Some(true)),
        ("box-group.skp", Some(true)),
        ("box-component.skp", Some(false)),
    ] {
        let m = openskp::Model::parse(&corpus(file)).unwrap();
        let scene = m.scene();
        assert_eq!(scene.len(), 1, "{file}: one scene root");
        assert_eq!(scene[0].is_group, want, "{file}: Node::is_group");
    }
}
