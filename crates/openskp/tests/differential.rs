//! Differential fidelity: the Rust reader must reproduce the Python reference
//! (`tools/skpwalk.parse_model`) across the whole corpus for every field except
//! the geometry topology (which the Rust SDK reports as resolved concrete counts
//! — covered separately by `topology.rs`).
//!
//! `tests/oracle.json` is the captured Python output; regenerate it from `tools/`
//! when the reference changes.
//!
//! One hand-correction on top of the capture: png-texture.skp's
//! "TextureWithAlpha" applied_size_in is 36×36 (SKP_FORMAT §4v) — the frozen
//! reference reads the JPEG dib shape (payload + u32 + w/h) unconditionally,
//! but PNG (subtype-4) payloads are followed by w/h directly, so it captured
//! 0×0 there.

use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../..");
    p
}

/// Compare two JSON values; numbers within tolerance (rounding-mode agnostic).
fn eq(a: &Value, b: &Value, path: &str) -> Result<(), String> {
    match (a, b) {
        (Value::Object(ma), Value::Object(mb)) => {
            let mut keys: Vec<&String> = ma.keys().chain(mb.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let va = ma.get(k);
                let vb = mb.get(k);
                match (va, vb) {
                    (Some(x), Some(y)) => eq(x, y, &format!("{path}.{k}"))?,
                    (None, _) => return Err(format!("{path}.{k}: missing in rust")),
                    (_, None) => return Err(format!("{path}.{k}: missing in oracle")),
                }
            }
            Ok(())
        }
        (Value::Array(aa), Value::Array(ab)) => {
            if aa.len() != ab.len() {
                return Err(format!(
                    "{path}: array len rust={} oracle={}",
                    aa.len(),
                    ab.len()
                ));
            }
            for (i, (x, y)) in aa.iter().zip(ab.iter()).enumerate() {
                eq(x, y, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        (Value::Number(na), Value::Number(nb)) => {
            let (x, y) = (na.as_f64().unwrap(), nb.as_f64().unwrap());
            let tol = 1e-4 + 1e-6 * x.abs().max(y.abs());
            if (x - y).abs() <= tol {
                Ok(())
            } else {
                Err(format!("{path}: number rust={x} oracle={y}"))
            }
        }
        _ if a == b => Ok(()),
        _ => Err(format!("{path}: rust={a} oracle={b}")),
    }
}

#[test]
fn matches_python_reference() {
    let root = root();
    let oracle: Value = serde_json::from_slice(
        &std::fs::read(root.join("crates/openskp/tests/oracle.json")).unwrap(),
    )
    .unwrap();
    let oracle = oracle.as_object().unwrap();

    let mut failures = Vec::new();
    let mut checked = 0;
    for (file, expected) in oracle {
        let d = std::fs::read(root.join("corpus/2017").join(file)).unwrap();
        let model = openskp::Model::parse(&d).expect("parse");
        let got: Value = serde_json::from_str(&model.to_json()).unwrap();

        // Compare every top-level field except geometry_runs (topology differs by
        // design: the SDK exposes resolved concrete counts, tested in topology.rs),
        // diagnostics (a Rust-side addition the frozen Python oracle predates),
        // and definitions/instances: the §4s CONTINUOUS walk decodes those from
        // the archive itself and legitimately sees MORE than the reference's
        // byte-scan (template/figure definitions whose non-empty descriptions
        // the name-record scan cannot match — box.skp rust=1 vs oracle=0 is the
        // template component, not a regression). Their semantics are pinned
        // where it matters instead: linkage.rs (names + translations per file)
        // and scene_dae.rs (.dae-validated world positions).
        let mut exp = expected.clone();
        let mut act = got.clone();
        for k in ["geometry_runs", "diagnostics", "definitions", "instances"] {
            exp.as_object_mut().unwrap().remove(k);
            act.as_object_mut().unwrap().remove(k);
        }

        checked += 1;
        if let Err(e) = eq(&act, &exp, file) {
            failures.push(e);
        }
    }
    assert!(checked > 0, "no corpus files checked");
    if !failures.is_empty() {
        panic!(
            "{}/{} files diverged:\n{}",
            failures.len(),
            checked,
            failures.join("\n")
        );
    }
}
