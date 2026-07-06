use std::path::PathBuf;

fn corpus(name: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    std::fs::read(p).unwrap()
}

#[test]
fn inventory_matches_reference() {
    // Counts captured from the Python reference (tools/skpwalk.inventory).
    let (name, count) = ("box.skp", 58usize);
    let inv = openskp::inventory(&corpus(name)).expect("inventory");
    assert_eq!(inv.len(), count, "class count for {name}");
    assert_eq!(inv[0], ("CArcCurve".to_string(), 3));
    assert_eq!(inv[count - 1], ("CWatermarkManager".to_string(), 2));
}
