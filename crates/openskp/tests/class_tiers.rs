//! Phase 2.1 bookkeeping: the class catalogue in docs/SKP_FORMAT.md must
//! cover EXACTLY the classes the format itself declares (the CVersionMap
//! inventory) — the catalogue cannot silently drift from the 58-class
//! reality.

use std::collections::BTreeSet;
use std::path::PathBuf;

#[test]
fn checklist_matches_the_version_map() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let d = std::fs::read(root.join("corpus/2017/box.skp")).unwrap();
    let inventory: BTreeSet<String> = openskp::inventory(&d)
        .expect("inventory")
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    let doc = std::fs::read_to_string(root.join("docs/SKP_FORMAT.md")).unwrap();
    // Scope the scan to the class-catalogue appendix so no other table in
    // the spec can satisfy (or pollute) the class-name column check.
    let start = doc
        .find("## Appendix A")
        .expect("SKP_FORMAT.md: class-catalogue appendix heading");
    let catalogue = match doc[start + 1..].find("\n## ") {
        Some(end) => &doc[start..start + 1 + end],
        None => &doc[start..],
    };
    let listed: BTreeSet<String> = catalogue
        .lines()
        .filter_map(|l| {
            let mut cols = l.split('|').map(str::trim);
            cols.next()?; // leading empty cell
            let class = cols.next()?;
            (class.starts_with('C') && !class.contains(' ')).then(|| class.to_string())
        })
        .collect();

    let missing: Vec<_> = inventory.difference(&listed).collect();
    let extra: Vec<_> = listed.difference(&inventory).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "SKP_FORMAT.md class catalogue drifted from the version map: missing {missing:?}, extra {extra:?}"
    );
    assert_eq!(inventory.len(), 58, "the 2017 class list is closed at 58");
}
