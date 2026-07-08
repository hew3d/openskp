//! Probe: list textured materials without carved image bytes.
fn main() {
    let path = std::env::args().nth(1).expect("usage: matdump <file.skp>");
    let d = std::fs::read(&path).expect("read");
    let model = openskp::Model::parse(&d).expect("parse");
    for (i, m) in model.materials.iter().enumerate() {
        if let openskp::Material::Textured {
            name,
            texture,
            image_bytes,
            applied_size_in,
            ..
        } = m
        {
            if image_bytes.is_none() {
                println!("[{i}] {name:?} texture={texture:?} size={applied_size_in:?} NO BYTES");
            }
        }
    }
}
// (appended) full ordered dump used by the slot solver
