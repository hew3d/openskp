//! Phase 4 close (SKP_FORMAT §4v): per-face UVs from the CFaceTextureCoords
//! projective blocks must reproduce the COLLADA exports' TEXCOORD sets.
//!
//! Oracles, per pair:
//! - every all-affine placement (identity / offset / rotated / rot45 /
//!   scaled / scale2x / no-FTC paints) matches the `.dae` TEXCOORDs
//!   per-corner;
//! - every pin list is reproduced EXACTLY by its side's projective matrix
//!   (`[anchor,1]·K → face`), including the two NON-affine pin files
//!   (pin-one, pin-fixed-distort) whose textures the exporter BAKES into
//!   face-bbox 0..1 UVs — for those the `.dae` oracles the frame rule but
//!   not K, so they are excluded from the TEXCOORD comparison.

use std::path::PathBuf;

fn corpus(name: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus/2017");
    p.push(name);
    std::fs::read(p).unwrap()
}

// ---- minimal COLLADA TEXCOORD reader (SketchUp-export subset) ----

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!("{name}=\"");
    let i = tag.find(&pat)? + pat.len();
    let j = tag[i..].find('"')? + i;
    Some(&tag[i..j])
}

fn blocks<'a>(doc: &'a str, tag: &str) -> Vec<(&'a str, &'a str)> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = doc[from..].find(&open) {
        let start = from + i;
        let head_end = doc[start..].find('>').unwrap() + start;
        let body_end = doc[head_end..].find(&close).unwrap() + head_end;
        out.push((&doc[start..head_end], &doc[head_end + 1..body_end]));
        from = body_end + close.len();
    }
    out
}

fn floats(s: &str) -> Vec<f64> {
    s.split_ascii_whitespace()
        .map(|t| t.parse().unwrap())
        .collect()
}

/// Textured polylist faces: `(corner positions [m], corner UVs)`.
fn dae_textured(name: &str) -> Vec<(Vec<[f64; 3]>, Vec<[f64; 2]>)> {
    let doc = String::from_utf8(corpus(name)).unwrap();
    let unit: f64 = attr(
        &doc[doc.find("<unit").unwrap()..doc.find("<unit").unwrap() + 80],
        "meter",
    )
    .unwrap()
    .parse()
    .unwrap();
    let mut arrays = std::collections::HashMap::new();
    for (head, body) in blocks(&doc, "float_array") {
        arrays.insert(attr(head, "id").unwrap().to_string(), floats(body));
    }
    let mut sources = std::collections::HashMap::new();
    for (head, body) in blocks(&doc, "source") {
        if let Some((fa_head, _)) = blocks(body, "float_array").first() {
            sources.insert(
                attr(head, "id").unwrap().to_string(),
                arrays[attr(fa_head, "id").unwrap()].clone(),
            );
        }
    }
    let mut verts = std::collections::HashMap::new();
    for (head, body) in blocks(&doc, "vertices") {
        for part in body.split("<input").skip(1) {
            let tag = &part[..part.find("/>").unwrap()];
            if attr(tag, "semantic") == Some("POSITION") {
                let src = attr(tag, "source").unwrap().trim_start_matches('#');
                verts.insert(attr(head, "id").unwrap().to_string(), sources[src].clone());
            }
        }
    }
    let mut out = Vec::new();
    for (_, body) in blocks(&doc, "polylist") {
        // offset-aware inputs; textured polylists carry VERTEX + TEXCOORD
        let mut vsrc = None;
        let mut tsrc = None;
        let (mut voff, mut toff, mut stride) = (0usize, 0usize, 0usize);
        for part in body.split("<input").skip(1) {
            let tag = &part[..part.find("/>").unwrap()];
            let off: usize = attr(tag, "offset").unwrap().parse().unwrap();
            stride = stride.max(off + 1);
            match attr(tag, "semantic").unwrap() {
                "VERTEX" => {
                    vsrc = Some(attr(tag, "source").unwrap().trim_start_matches('#'));
                    voff = off;
                }
                "TEXCOORD" => {
                    tsrc = Some(attr(tag, "source").unwrap().trim_start_matches('#'));
                    toff = off;
                }
                _ => {}
            }
        }
        let (Some(vs), Some(ts)) = (vsrc, tsrc) else {
            continue;
        };
        let (pos, uvs) = (&verts[vs], &sources[ts]);
        let vcount: Vec<usize> = blocks(body, "vcount")[0]
            .1
            .split_ascii_whitespace()
            .map(|t| t.parse().unwrap())
            .collect();
        let p: Vec<usize> = blocks(body, "p")[0]
            .1
            .split_ascii_whitespace()
            .map(|t| t.parse().unwrap())
            .collect();
        let mut at = 0;
        for n in vcount {
            let mut ring = Vec::with_capacity(n);
            let mut ring_uv = Vec::with_capacity(n);
            for k in 0..n {
                let vi = p[(at + k) * stride + voff];
                let ti = p[(at + k) * stride + toff];
                ring.push([pos[vi * 3] * unit, pos[vi * 3 + 1] * unit, pos[vi * 3 + 2] * unit]);
                ring_uv.push([uvs[ti * 2], uvs[ti * 2 + 1]]);
            }
            at += n;
            out.push((ring, ring_uv));
        }
    }
    out
}

// ---- the check ----

fn q(p: [f64; 3]) -> [i64; 3] {
    [
        (p[0] * 1e7).round() as i64,
        (p[1] * 1e7).round() as i64,
        (p[2] * 1e7).round() as i64,
    ]
}

/// Validate one pair. Returns `(faces checked, worst pin residual [in])`.
/// `check_dae_uv = false` for the projective-pin files (baked exports).
fn check(stem: &str, check_dae_uv: bool, tol: f64) -> (usize, f64) {
    let skp = corpus(&format!("{stem}.skp"));
    let model = openskp::Model::parse(&skp).expect("parse");
    let dae = dae_textured(&format!("{stem}.dae"));
    assert!(!dae.is_empty(), "{stem}: no textured .dae faces");

    let mut checked = 0;
    let mut worst_pin = 0.0f64;
    for run in &model.geometry {
        for f in &run.mesh.faces {
            // pins must be reproduced by their side's matrix regardless of
            // what the exporter did with the texture
            if let Some(t) = &f.texture {
                for (k, pins) in [(&t.front, &t.front_pins), (&t.back, &t.back_pins)] {
                    for p in pins {
                        let w = p[0] * k[2] + p[1] * k[5] + k[8];
                        let x = (p[0] * k[0] + p[1] * k[3] + k[6]) / w;
                        let y = (p[0] * k[1] + p[1] * k[4] + k[7]) / w;
                        worst_pin = worst_pin.max((x - p[2]).abs()).max((y - p[3]).abs());
                    }
                }
            }
            let sides = [
                (f.front_material, openskp::Side::Front),
                (f.back_material, openskp::Side::Back),
            ];
            for (mat, side) in sides {
                let Some(size) = mat.and_then(|slot| model.applied_size_of(slot)) else {
                    continue;
                };
                // match a dae textured face by corner positions
                let ours: std::collections::BTreeSet<[i64; 3]> = f
                    .outer
                    .iter()
                    .map(|&vi| q(run.mesh.vertices[vi as usize]))
                    .collect();
                let matches = dae.iter().filter(|(ring, _)| {
                    ring.iter().map(|&p| q(p)).collect::<std::collections::BTreeSet<_>>() == ours
                });
                for (ring, ring_uv) in matches {
                    checked += 1;
                    if !check_dae_uv {
                        continue;
                    }
                    let x = f.uv_xform(side, size).expect("singular FTC matrix");
                    for (p, uv) in ring.iter().zip(ring_uv) {
                        let got = x.apply(*p);
                        assert!(
                            (got[0] - uv[0]).abs() < tol && (got[1] - uv[1]).abs() < tol,
                            "{stem} pid {}: uv {:?} != dae {:?} (side {side:?})",
                            f.pid,
                            got,
                            uv
                        );
                    }
                }
            }
        }
    }
    assert!(checked > 0, "{stem}: no skp face matched a textured dae face");
    (checked, worst_pin)
}

#[test]
fn affine_placements_match_dae_texcoords() {
    // (pair, uv tolerance): rot45's stored k carries ~2e-7 write noise
    // (SKP_FORMAT §4r), everything else is float-exact vs the export print.
    for (stem, tol) in [
        ("uv-quad", 1e-6),
        ("texture-offset", 1e-6),
        ("texture-rotated", 1e-6),
        ("texture-rot45", 1e-5),
        ("texture-scale2x", 1e-6),
        ("texture-scaled", 1e-6),
        ("png-texture", 1e-6),
        ("pin-identity", 1e-6),
        ("pin-four", 1e-6),
    ] {
        let (n, pin) = check(stem, true, tol);
        assert!(n >= 1, "{stem}");
        assert!(pin < 1e-9, "{stem}: pin residual {pin}");
    }
}

#[test]
fn projective_pin_files_reproduce_pins_exactly() {
    // Non-affine pin arrangements: the exporter bakes the texture (face-bbox
    // UVs), so K's oracle is the pin list itself (§4v).
    for stem in ["pin-one", "pin-fixed-distort"] {
        let (n, pin) = check(stem, false, 0.0);
        assert!(n >= 1, "{stem}");
        assert!(pin < 1e-9, "{stem}: pin residual {pin}");
    }
}

#[test]
fn png_texture_applied_size_is_36in() {
    // §4v: subtype-4 (PNG) dib payloads are followed by w/h directly —
    // the JPEG-shaped read used to report 0×0 here.
    let model = openskp::Model::parse(&corpus("png-texture.skp")).unwrap();
    let m = model
        .materials
        .iter()
        .find_map(|m| match m {
            openskp::Material::Textured {
                name,
                applied_size_in,
                ..
            } if name == "TextureWithAlpha" => Some(*applied_size_in),
            _ => None,
        })
        .expect("TextureWithAlpha present");
    assert_eq!(m, Some((36.0, 36.0)));
}
