//! Scan a .skp for entities with the hidden flag set and print their
//! coordinates (inches and meters) plus pids so they can be located in
//! SketchUp. Oracle for SKP_FORMAT §4q/§4s: house.skp holds EXACTLY one
//! hidden edge (pid 0x1458) and one hidden face (pid 0x1459).

const M_PER_IN: f64 = 0.0254;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: find_hidden <file.skp>");
    let m = openskp::Model::read(&path).unwrap();
    let mut found = 0;
    for (ri, run) in m.geometry.iter().enumerate() {
        for (ei, e) in run.mesh.edges.iter().enumerate() {
            if e.hidden {
                found += 1;
                let a = run.mesh.vertices[e.v0 as usize];
                let b = run.mesh.vertices[e.v1 as usize];
                println!(
                    "HIDDEN EDGE  pid 0x{:x} run {ri} (@0x{:x}) edge {ei}: ({:.2}, {:.2}, {:.2})\" -> ({:.2}, {:.2}, {:.2})\"  [{:.4}, {:.4}, {:.4}] m -> [{:.4}, {:.4}, {:.4}] m",
                    e.pid, run.start,
                    a[0] / M_PER_IN, a[1] / M_PER_IN, a[2] / M_PER_IN,
                    b[0] / M_PER_IN, b[1] / M_PER_IN, b[2] / M_PER_IN,
                    a[0], a[1], a[2], b[0], b[1], b[2]
                );
            }
        }
        for (fi, f) in run.mesh.faces.iter().enumerate() {
            if f.hidden {
                found += 1;
                let n = f.outer.len() as f64;
                let c = f.outer.iter().fold([0.0; 3], |acc, &vi| {
                    let v = run.mesh.vertices[vi as usize];
                    [acc[0] + v[0] / n, acc[1] + v[1] / n, acc[2] + v[2] / n]
                });
                println!(
                    "HIDDEN FACE  pid 0x{:x} run {ri} (@0x{:x}) face {fi}: {} verts, centroid ({:.2}, {:.2}, {:.2})\"  [{:.4}, {:.4}, {:.4}] m  normal ({:.2}, {:.2}, {:.2})",
                    f.pid, run.start, f.outer.len(),
                    c[0] / M_PER_IN, c[1] / M_PER_IN, c[2] / M_PER_IN,
                    c[0], c[1], c[2], f.normal[0], f.normal[1], f.normal[2]
                );
            }
        }
    }
    let t = m.geometry.iter().fold((0, 0, 0), |(v, e, f), r| {
        (
            v + r.topology.vertices,
            e + r.topology.edges,
            f + r.topology.faces,
        )
    });
    let groups = m
        .instances
        .iter()
        .filter(|i| i.is_group == Some(true))
        .count();
    let unresolved: usize = m.geometry.iter().map(|r| r.resolved.1 - r.resolved.0).sum();
    println!(
        "totals: {} CEdge, {} CFace, {} CGroup, {} CVertex, {} unresolved refs ({} hidden)",
        t.1, t.2, groups, t.0, unresolved, found
    );
}
