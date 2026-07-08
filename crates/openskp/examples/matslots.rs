//! Probe: material slot arithmetic inputs for a .skp file.
fn main() {
    let path = std::env::args().nth(1).expect("usage: matslots <file.skp>");
    let d = std::fs::read(&path).expect("read");
    let model = openskp::Model::parse(&d).expect("parse");
    let mut refs: Vec<u16> = model
        .geometry
        .iter()
        .flat_map(|r| r.mesh.faces.iter())
        .flat_map(|f| [f.front_material, f.back_material])
        .flatten()
        .collect();
    let mut irefs: Vec<u16> = model.instances.iter().filter_map(|i| i.material).collect();
    fn walk(n: &openskp::Node, out: &mut Vec<u16>) {
        if n.material != 0 {
            out.push(n.material);
        }
        for c in &n.children {
            walk(c, out);
        }
    }
    let mut nrefs = Vec::new();
    for n in model.scene().iter() {
        walk(n, &mut nrefs);
    }
    refs.sort_unstable();
    refs.dedup();
    irefs.sort_unstable();
    irefs.dedup();
    nrefs.sort_unstable();
    nrefs.dedup();
    println!("materials extracted: {}", model.materials.len());
    let textured = model
        .materials
        .iter()
        .filter(|m| matches!(m, openskp::Material::Textured { .. }))
        .count();
    let with_bytes = model
        .materials
        .iter()
        .filter(|m| {
            matches!(
                m,
                openskp::Material::Textured {
                    image_bytes: Some(_),
                    ..
                }
            )
        })
        .count();
    println!("textured: {textured} (with embedded bytes: {with_bytes})");
    println!("face matref slots ({}): {:?}", refs.len(), refs);
    println!(
        "instance matref slots ({}): {:?}",
        irefs.len(),
        &irefs[..irefs.len().min(30)]
    );
    println!(
        "scene-node matref slots ({}): {:?}",
        nrefs.len(),
        &nrefs[..nrefs.len().min(30)]
    );
    println!(
        "material_links ({}): {:?}",
        model.material_links.len(),
        &model.material_links[..model.material_links.len().min(20)]
    );
}
