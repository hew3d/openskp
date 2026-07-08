//! Probe: which material SLOT do faces of a named definition carry?
fn main() {
    let path = std::env::args().nth(1).expect("file");
    let d = std::fs::read(&path).expect("read");
    let model = openskp::Model::parse(&d).expect("parse");
    for want in [
        "2x6 Stud",
        "4x8 15/32\" Radiant Barrier OSB",
        "James Hardie Artisan Shiplap Siding",
    ] {
        // def index by name -> its geometry run via definition_links
        let Some(k) = model.definitions.iter().position(|df| df.name == want) else {
            println!("{want:?}: no def");
            continue;
        };
        let Some(&(dr, _)) = model.definition_links.iter().find(|&&(_, kk)| kk == k) else {
            println!("{want:?}: no link");
            continue;
        };
        let Some(run) = model
            .geometry
            .iter()
            .find(|r| r.def_index == Some(dr as usize))
        else {
            println!("{want:?}: no run");
            continue;
        };
        let mut slots: Vec<u16> = run
            .mesh
            .faces
            .iter()
            .flat_map(|f| [f.front_material, f.back_material])
            .flatten()
            .collect();
        slots.sort_unstable();
        slots.dedup();
        println!("{want:?}: face material slots {slots:?}");
    }
}
