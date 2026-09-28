//! Dump a 2017 file's per-entity appearance data as JSON for
//! `tools/skp26_check.py`: every face's ring (metres), front/back material
//! NAMES, layer name, hidden flag and texture placement; every edge's layer;
//! every placed instance's name, material, layer and hidden flag; and the
//! material and layer tables. The 2026 reader is validated against this.

use serde_json::{json, Value};

fn material_json(m: &openskp::Material) -> Value {
    match m {
        openskp::Material::Solid {
            name,
            rgba,
            opacity,
        } => {
            json!({"name": name, "kind": "solid", "rgba": rgba, "opacity": opacity})
        }
        openskp::Material::Textured {
            name,
            texture,
            applied_size_in,
            image_bytes,
            avg_rgba,
            opacity,
        } => json!({
            "name": name, "kind": "textured", "texture": texture,
            "applied_size_in": applied_size_in, "image_len": image_bytes.as_ref().map(|b| b.len()),
            "avg_rgba": avg_rgba, "opacity": opacity,
        }),
    }
}

fn name_of(m: &openskp::Model, slot: Option<u16>) -> Value {
    match slot.and_then(|s| m.material_of(s)) {
        Some(openskp::Material::Solid { name, .. })
        | Some(openskp::Material::Textured { name, .. }) => {
            json!(name)
        }
        None => Value::Null,
    }
}

fn layer_of(m: &openskp::Model, slot: u16) -> Value {
    m.layer_of(slot)
        .map(|l| json!(l.name))
        .unwrap_or(Value::Null)
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: oracle17 <file.skp>");
    let m = openskp::Model::read(&path).unwrap();
    let runs: Vec<Value> = m
        .geometry
        .iter()
        .map(|r| {
            let v = &r.mesh.vertices;
            let faces: Vec<Value> = r
                .mesh
                .faces
                .iter()
                .map(|f| {
                    json!({
                        "pid": f.pid,
                        "outer": f.outer.iter().map(|&i| v[i as usize]).collect::<Vec<_>>(),
                        "front": name_of(&m, f.front_material),
                        "back": name_of(&m, f.back_material),
                        "layer": layer_of(&m, f.layer),
                        "hidden": f.hidden,
                        "texture": f.texture.as_ref().map(|t| json!({
                            "front": t.front, "back": t.back,
                            "front_extra": t.front_extra, "back_extra": t.back_extra,
                            "front_pins": t.front_pins, "back_pins": t.back_pins,
                            "flags": t.flags,
                        })),
                    })
                })
                .collect();
            let edges: Vec<Value> = r
                .mesh
                .edges
                .iter()
                .map(|e| json!({"a": v[e.v0 as usize], "b": v[e.v1 as usize], "layer": layer_of(&m, e.layer)}))
                .collect();
            json!({"faces": faces, "edges": edges})
        })
        .collect();
    fn walk(m: &openskp::Model, n: &openskp::Node, out: &mut Vec<Value>) {
        out.push(json!({
            "definition": n.definition, "world": n.world,
            "material": name_of(m, Some(n.material)), "layer": layer_of(m, n.layer),
            "hidden": n.hidden,
        }));
        for c in &n.children {
            walk(m, c, out);
        }
    }
    let mut nodes = Vec::new();
    for n in m.scene() {
        walk(&m, &n, &mut nodes);
    }
    let out = json!({
        "materials": m.materials.iter().map(material_json).collect::<Vec<_>>(),
        "layers": m.layers.iter().map(|l| json!({"name": l.name, "visible": l.visible, "rgba": l.rgba})).collect::<Vec<_>>(),
        "runs": runs,
        "nodes": nodes,
    });
    println!("{out}");
}
