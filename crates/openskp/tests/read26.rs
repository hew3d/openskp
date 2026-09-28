//! Post-2017 acceptance: every SketchUp 2026 save in the corpus reads to a
//! model EQUIVALENT to its decoded 2017 original, through the public API
//! alone. corpus/2026/X.skp is the SketchUp web app's conversion of
//! corpus/2017/X.skp (theater likewise), so the 2017 reader — itself
//! validated against COLLADA — is the oracle for every value.
//!
//! Persistent ids mostly survive the conversion but are renumbered where a
//! 2017 file held duplicates, and a few definition GUIDs are regenerated, so
//! comparisons are by content: vertex rings, endpoints, names, matrices.

use std::collections::BTreeMap;
use std::path::PathBuf;

use openskp::{Material, Model};

fn model(rel: &str) -> Model {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus")
        .join(rel);
    Model::read(&p).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// Every 2026 corpus file next to its 2017 original, plus the third-party
/// benchmark pair.
fn pairs() -> Vec<(String, String)> {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.push("../../corpus");
    let mut names: Vec<String> = std::fs::read_dir(dir.join("2026"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".skp") && dir.join("2017").join(n).exists())
        .collect();
    names.sort();
    let mut out: Vec<(String, String)> = names
        .into_iter()
        .map(|n| (format!("2017/{n}"), format!("2026/{n}")))
        .collect();
    out.push((
        "third-party/theater-2017.skp".into(),
        "third-party/theater-2026.skp".into(),
    ));
    out
}

type P = [i64; 3];

fn key(p: [f64; 3]) -> P {
    p.map(|c| (c * 1e6).round() as i64)
}

fn k6(x: f64) -> i64 {
    (x * 1e6).round() as i64
}

/// Rotation-invariant ring key (winding kept).
fn ring(pts: Vec<P>) -> Vec<P> {
    let i = (0..pts.len()).min_by_key(|&i| pts[i]).unwrap_or(0);
    pts[i..].iter().chain(&pts[..i]).copied().collect()
}

fn multiset<T: Ord>(it: impl IntoIterator<Item = T>) -> BTreeMap<T, usize> {
    let mut m = BTreeMap::new();
    for x in it {
        *m.entry(x).or_insert(0) += 1;
    }
    m
}

fn mat_name(m: &Model, slot: Option<u16>) -> Option<String> {
    slot.and_then(|s| m.material_of(s)).map(|x| match x {
        Material::Solid { name, .. } | Material::Textured { name, .. } => name.clone(),
    })
}

fn layer_name(m: &Model, slot: u16) -> String {
    m.layer_of(slot).map(|l| l.name.clone()).unwrap_or_default()
}

/// Per-definition content: faces (outer ring, holes, hidden) and edges
/// (endpoints, soft, smooth, hidden).
type RunSig = (
    Vec<(Vec<P>, Vec<Vec<P>>, bool)>,
    Vec<([P; 2], bool, bool, bool)>,
);

fn run_sig(r: &openskp::GeometryRun) -> RunSig {
    let v = |i: u32| key(r.mesh.vertices[i as usize]);
    let mut faces: Vec<_> = r
        .mesh
        .faces
        .iter()
        .map(|f| {
            let mut holes: Vec<_> = f
                .holes
                .iter()
                .map(|h| ring(h.iter().map(|&i| v(i)).collect()))
                .collect();
            holes.sort();
            (
                ring(f.outer.iter().map(|&i| v(i)).collect()),
                holes,
                f.hidden,
            )
        })
        .collect();
    faces.sort();
    let mut edges: Vec<_> = r
        .mesh
        .edges
        .iter()
        .map(|e| {
            let mut ends = [v(e.v0), v(e.v1)];
            ends.sort();
            (ends, e.soft, e.smooth, e.hidden)
        })
        .collect();
    edges.sort();
    (faces, edges)
}

#[test]
fn every_2026_file_reads_equivalent_to_its_2017_original() {
    for (old, new) in pairs() {
        let (old, new) = (old.as_str(), new.as_str());
        let a = model(old);
        let b = model(new);
        assert_eq!(b.version, "{26.2.0}", "{new}");
        // The conversion writes a fresh model GUID into meta/meta.dat.
        let guid = b.model_guid.as_deref().expect("2026 model GUID");
        assert!(
            guid.len() == 32 && guid.chars().all(|c| c.is_ascii_hexdigit()),
            "{new}"
        );
        assert_ne!(b.model_guid, a.model_guid, "{new}");
        assert_eq!(b.diagnostics, Vec::new(), "{new}");

        // 1. Every definition's (and the root's) geometry, by content.
        let sigs = |m: &Model| {
            multiset(
                m.geometry
                    .iter()
                    .filter(|r| !r.mesh.faces.is_empty() || !r.mesh.edges.is_empty())
                    .map(run_sig),
            )
        };
        assert!(sigs(&a) == sigs(&b), "{new}: definition geometry differs");

        // 2. Topology counts per run, and curve member counts.
        let topo = |m: &Model| multiset(m.geometry.iter().map(|r| format!("{:?}", r.topology)));
        assert_eq!(topo(&a), topo(&b), "{new}: topology counts");
        let curves = |m: &Model| {
            multiset(
                m.geometry
                    .iter()
                    .flat_map(|r| r.curve_members.iter().copied()),
            )
        };
        assert_eq!(curves(&a), curves(&b), "{new}: curve member counts");

        // 3. The composed scene: every placed leaf's definition, world
        //    matrix, inherited material, layer and hidden flag.
        fn leaves(m: &Model, n: &openskp::Node, out: &mut Vec<String>) {
            out.push(format!(
                "{:?} {:?} {:?} {} {} {:?}",
                n.definition,
                n.world.map(k6),
                mat_name(m, Some(n.material)),
                layer_name(m, n.layer),
                n.hidden,
                n.is_group
            ));
            for c in &n.children {
                leaves(m, c, out);
            }
        }
        let scene = |m: &Model| {
            let mut out = Vec::new();
            for n in m.scene() {
                leaves(m, &n, &mut out);
            }
            multiset(out)
        };
        let (sa, sb) = (scene(&a), scene(&b));
        assert!(
            sa == sb,
            "{new}: scene differs ({} vs {} nodes)",
            sa.values().sum::<usize>(),
            sb.values().sum::<usize>()
        );

        // 4. Face and edge appearance: materials by name, layers by name,
        //    and the texture placement (matrices, pins, flags).
        let appearance = |m: &Model| {
            let mut out = Vec::new();
            for r in &m.geometry {
                let v = |i: u32| key(r.mesh.vertices[i as usize]);
                for f in &r.mesh.faces {
                    out.push(format!(
                        "F {:?} {:?} {:?} {} {:?}",
                        ring(f.outer.iter().map(|&i| v(i)).collect()),
                        mat_name(m, f.front_material),
                        mat_name(m, f.back_material),
                        layer_name(m, f.layer),
                        f.texture.as_ref().map(|t| (
                            t.front.map(k6),
                            t.back.map(k6),
                            t.front_extra.map(k6),
                            t.back_extra.map(k6),
                            t.front_pins.iter().map(|p| p.map(k6)).collect::<Vec<_>>(),
                            t.back_pins.iter().map(|p| p.map(k6)).collect::<Vec<_>>(),
                            t.flags,
                        )),
                    ));
                }
                for e in &r.mesh.edges {
                    let mut ends = [v(e.v0), v(e.v1)];
                    ends.sort();
                    out.push(format!("E {:?} {}", ends, layer_name(m, e.layer)));
                }
            }
            multiset(out)
        };
        assert!(
            appearance(&a) == appearance(&b),
            "{new}: face/edge appearance differs"
        );

        // 5. Material and layer tables. A solid's colour is compared as RGB
        //    plus opacity: the 2026 container keeps no separate colour alpha
        //    (2017 files can store a stale one — feature-pack's "*4" holds
        //    255 at opacity 0.5, house-plus's "*14" 127), so the reader
        //    derives alpha from opacity, as the COLLADA export does.
        let mats = |m: &Model| {
            multiset(m.materials.iter().map(|x| {
                match x {
                    Material::Solid {
                        name,
                        rgba,
                        opacity,
                    } => format!("S {name} {:?} {}", &rgba[..3], k6(*opacity)),
                    Material::Textured {
                        name,
                        applied_size_in,
                        image_bytes,
                        avg_rgba,
                        opacity,
                        ..
                    } => format!(
                        "T {name} {:?} {:?} {avg_rgba:?} {}",
                        // The 2017 path rounds applied sizes to 3 decimals.
                        applied_size_in
                            .map(|(w, h)| ((w * 1e3).round() as i64, (h * 1e3).round() as i64)),
                        image_bytes.as_ref().map(|b| b.len()),
                        k6(*opacity) / 1000,
                    ),
                }
            }))
        };
        assert_eq!(mats(&a), mats(&b), "{new}: materials");
        let layers = |m: &Model| {
            multiset(
                m.layers
                    .iter()
                    .map(|l| format!("{} {} {:?}", l.name, l.visible, l.rgba)),
            )
        };
        assert_eq!(layers(&a), layers(&b), "{new}: layers");

        // 6. Document-level lists.
        let scene_names = |m: &openskp::Model| -> Vec<String> {
            m.scenes.iter().map(|s| s.name.clone()).collect()
        };
        assert_eq!(scene_names(&a), scene_names(&b), "{new}: scenes");
        let guides = |m: &Model| {
            multiset(m.guides.iter().map(|g| {
                (
                    g.point_m.map(|x| (x * 1e4).round() as i64),
                    g.direction.map(k6),
                )
            }))
        };
        assert_eq!(guides(&a), guides(&b), "{new}: guides");
        let defs = |m: &Model| multiset(m.definitions.iter().map(|d| d.name.clone()));
        assert_eq!(defs(&a), defs(&b), "{new}: definition names");
        let insts = |m: &Model| {
            multiset(m.instances.iter().map(|i| {
                (
                    i.definition.clone(),
                    i.name.clone(),
                    i.is_group,
                    i.translation_m.map(k6),
                )
            }))
        };
        assert_eq!(
            insts(&a),
            insts(&b),
            "{new}: instances (definition, name, group, translation)"
        );
        assert!(b.instances.iter().all(|i| i.is_group.is_some()), "{new}");
    }
}

/// A textured material's image bytes are the 2017 file's embedded bytes,
/// byte for byte (the conversion stores them as ordinary archive files).
#[test]
fn texture_images_are_byte_identical() {
    for (old, new) in pairs() {
        let (old, new) = (old.as_str(), new.as_str());
        let (a, b) = (model(old), model(new));
        let imgs = |m: &Model| -> BTreeMap<String, Vec<u8>> {
            m.materials
                .iter()
                .filter_map(|x| match x {
                    Material::Textured {
                        name,
                        image_bytes: Some(b),
                        ..
                    } => Some((name.clone(), b.clone())),
                    _ => None,
                })
                .collect()
        };
        let (ia, ib) = (imgs(&a), imgs(&b));
        assert_eq!(
            ia.keys().collect::<Vec<_>>(),
            ib.keys().collect::<Vec<_>>(),
            "{new}"
        );
        for (k, v) in &ia {
            assert!(ib[k] == *v, "{new}: {k} image bytes differ");
        }
    }
}

/// The feature-pack's annotations surface as counts in the root run, like
/// the 2017 reader's.
#[test]
fn feature_pack_annotations() {
    let m = model("2026/feature-pack.skp");
    let t = &m.geometry.last().unwrap().topology;
    assert_eq!(
        (
            t.construction_points,
            t.section_planes,
            t.texts,
            t.dimensions,
            t.images_placed
        ),
        (1, 1, 2, 1, 1)
    );
    assert_eq!(m.guides.len(), 1);
    assert_eq!(m.guides[0].direction, [1.0, 0.0, 0.0]);
}

fn font_of(m: &Model, i: Option<usize>) -> Option<openskp::Font> {
    i.map(|i| m.fonts[i].clone())
}

/// Anchored-entity ids match when the file's ids survived the conversion;
/// otherwise only their presence can be compared.
fn same_anchor_entity(x: Option<u32>, y: Option<u32>, same_ids: bool, what: &str) {
    if same_ids {
        assert_eq!(x, y, "{what}: anchor entity");
    } else {
        assert_eq!(x.is_some(), y.is_some(), "{what}: anchor entity");
    }
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn close3(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
    (0..3).all(|i| close(a[i], b[i], tol))
}

fn same_camera(a: &openskp::Camera, b: &openskp::Camera, what: &str) {
    assert!(
        close3(a.eye_m, b.eye_m, 1e-6),
        "{what}: eye {:?} vs {:?}",
        a.eye_m,
        b.eye_m
    );
    assert!(close3(a.target_m, b.target_m, 1e-6), "{what}: target");
    assert!(close3(a.up, b.up, 1e-9), "{what}: up");
    assert_eq!(a.perspective, b.perspective, "{what}: perspective");
    assert!(close(a.fov_deg, b.fov_deg, 1e-9), "{what}: fov");
    assert!(
        close(a.parallel_height_m, b.parallel_height_m, 1e-6),
        "{what}: height"
    );
    assert_eq!(
        a.two_point_perspective, b.two_point_perspective,
        "{what}: two-point"
    );
}

/// Every rendering-option field the 2017 block carries.
fn same_rendering(a: &openskp::RenderingOptions, b: &openskp::RenderingOptions, what: &str) {
    let mut a = a.clone();
    let mut b = b.clone();
    // post-2017-only fields
    a.section_fill = None;
    b.section_fill = None;
    a.section_fill_color = None;
    b.section_fill_color = None;
    assert_eq!(a, b, "{what}: rendering options");
}

fn same_shadows(a: &openskp::ShadowInfo, b: &openskp::ShadowInfo, what: &str) {
    assert_eq!(a, b, "{what}: shadow info");
}

/// The web app adds its own "SketchUp" watermark to every file it saves.
fn own_watermarks(w: &[openskp::Watermark]) -> Vec<openskp::Watermark> {
    w.iter().filter(|w| w.name != "SketchUp").cloned().collect()
}

#[test]
fn every_2026_file_reads_the_same_settings_as_its_2017_original() {
    for (old, new) in pairs() {
        let (old, new) = (old.as_str(), new.as_str());
        let a = model(old);
        let b = model(new);
        assert_eq!(a.container, openskp::Container::Carchive2017, "{old}");
        assert_eq!(b.container, openskp::Container::Zip, "{new}");

        same_camera(
            a.camera.as_ref().expect("2017 camera"),
            b.camera.as_ref().expect("2026 camera"),
            new,
        );
        same_rendering(
            a.rendering.as_ref().expect("2017 rendering"),
            b.rendering.as_ref().expect("2026 rendering"),
            new,
        );
        same_shadows(
            a.shadows.as_ref().expect("2017 shadows"),
            b.shadows.as_ref().expect("2026 shadows"),
            new,
        );

        // Units: the post-2017 file adds area and volume keys, and the 2017
        // engineering format (2) converts to decimal feet (§16.12).
        let mut ua = a.units.clone();
        let mut ub = b.units.clone();
        ub.area_unit = None;
        ub.volume_unit = None;
        ub.area_precision = a.units.area_precision;
        ub.volume_precision = a.units.volume_precision;
        if ua.length_format == 2 {
            ua.length_format = 0;
            ua.length_unit = 1;
        }
        // The conversion recomputes the snap length in the display unit
        // (0.003937 m stored by 2017 versus 1/254 exactly).
        assert!(
            close(ua.length_snap_length, ub.length_snap_length, 1e-6),
            "{new}: snap length {} vs {}",
            ua.length_snap_length,
            ub.length_snap_length
        );
        ub.length_snap_length = ua.length_snap_length;
        assert_eq!(ua, ub, "{new}: units");

        // Document tail (§10.11): axes, annotation defaults, and the
        // settings kept in option sets. A 2017 file without texts holds
        // null text-default font references; the conversion assigns a font.
        assert_eq!(a.axes, b.axes, "{new}: axes");
        assert_eq!(
            a.anti_aliased_textures, b.anti_aliased_textures,
            "{new}: anti-aliased textures"
        );
        assert_eq!(a.animation, b.animation, "{new}: animation");
        assert_eq!(a.geo_located, b.geo_located, "{new}: geo-located");
        let (ta, tb) = (
            a.text_defaults.as_ref().expect("2017 text defaults"),
            b.text_defaults.as_ref().expect("2026 text defaults"),
        );
        assert_eq!(ta.arrow, tb.arrow, "{new}: text default arrow");
        assert_eq!(
            ta.leader_text_color, tb.leader_text_color,
            "{new}: leader text colour"
        );
        assert_eq!(
            ta.screen_text_color, tb.screen_text_color,
            "{new}: screen text colour"
        );
        for (x, y) in [(ta.font, tb.font), (ta.screen_font, tb.screen_font)] {
            if x.is_some() {
                assert_eq!(font_of(&a, x), font_of(&b, y), "{new}: text default font");
            }
        }
        let (da, db) = (
            a.dimension_defaults
                .as_ref()
                .expect("2017 dimension defaults"),
            b.dimension_defaults
                .as_ref()
                .expect("2026 dimension defaults"),
        );
        assert_eq!(da.aligned, db.aligned, "{new}: dimension default aligned");
        assert_eq!(da.arrow, db.arrow, "{new}: dimension default arrow");
        assert_eq!(
            da.text_position, db.text_position,
            "{new}: dimension default text position"
        );
        assert_eq!(da.color, db.color, "{new}: dimension default colour");
        assert_eq!(
            font_of(&a, da.font),
            font_of(&b, db.font),
            "{new}: dimension default font"
        );

        // Styles: same names; the conversion writes the current settings
        // into the active style, so that style reads as the 2017 document
        // settings; the current watermarks match apart from the added one.
        let names =
            |m: &Model| -> Vec<String> { m.styles.iter().map(|s| s.name.clone()).collect() };
        assert_eq!(names(&a), names(&b), "{new}: style names");
        assert_eq!(a.active_style, b.active_style, "{new}: active style");
        let active = b.active_style.expect("active style");
        same_rendering(
            &a.rendering.as_ref().unwrap().style_view(),
            &b.styles[active].settings.style_view(),
            &format!("{new}: active style"),
        );
        assert_eq!(
            own_watermarks(&a.watermarks),
            own_watermarks(&b.watermarks),
            "{new}: watermarks"
        );

        // Scenes: the conversion maps the 2017 property bits 1–64 onto the
        // same bits; every saved part reads the same.
        assert_eq!(a.scenes.len(), b.scenes.len(), "{new}: scene count");
        for (sa, sb) in a.scenes.iter().zip(&b.scenes) {
            let what = format!("{new}: scene {:?}", sa.name);
            assert_eq!(sa.name, sb.name, "{what}");
            assert_eq!(sa.description, sb.description, "{what}: description");
            assert_eq!(sa.saved.0 & 0x7F, sb.saved.0 & 0x7F, "{what}: saved");
            assert_eq!(sa.in_animation, sb.in_animation, "{what}: animation");
            assert_eq!(sa.style, sb.style, "{what}: style");
            let layer_names = |m: &Model, idx: &[usize]| -> Vec<String> {
                idx.iter().map(|&i| m.layers[i].name.clone()).collect()
            };
            assert_eq!(
                layer_names(&a, &sa.hidden_layers),
                layer_names(&b, &sb.hidden_layers),
                "{what}: hidden layers"
            );
            match (&sa.camera, &sb.camera) {
                (Some(x), Some(y)) => same_camera(x, y, &what),
                (None, None) => {}
                _ => panic!("{what}: camera presence"),
            }
            match (&sa.rendering, &sb.rendering) {
                (Some(x), Some(y)) => same_rendering(x, y, &what),
                (None, None) => {}
                _ => panic!("{what}: rendering presence"),
            }
            match (&sa.shadows, &sb.shadows) {
                (Some(x), Some(y)) => same_shadows(x, y, &what),
                (None, None) => {}
                _ => panic!("{what}: shadow presence"),
            }
            match (&sa.axes, &sb.axes) {
                (Some(x), Some(y)) => assert_eq!(x, y, "{what}: axes"),
                (None, None) => {}
                _ => panic!("{what}: axes presence"),
            }
            // house-plus's hidden entities are among the ids the conversion
            // renumbers, so only the count is compared there.
            assert_eq!(
                sa.hidden_entities.len(),
                sb.hidden_entities.len(),
                "{what}: hidden entities"
            );
            assert_eq!(
                sa.active_section_planes.len(),
                sb.active_section_planes.len(),
                "{what}: section planes"
            );
        }

        // Annotations, matched by content (house-plus's ids are renumbered).
        // Persistent ids survive the conversion unless the 2017 file held
        // duplicates, in which case the converter renumbers (house-plus).
        let instance_pids = |m: &Model| -> std::collections::BTreeSet<u32> {
            m.instances.iter().map(|i| i.pid).collect()
        };
        let same_ids = instance_pids(&a) == instance_pids(&b);
        let texts = |m: &Model| -> BTreeMap<String, openskp::Text> {
            m.texts
                .iter()
                .map(|t| (t.content.clone(), t.clone()))
                .collect()
        };
        let (ta, tb) = (texts(&a), texts(&b));
        assert_eq!(
            ta.keys().collect::<Vec<_>>(),
            tb.keys().collect::<Vec<_>>(),
            "{new}: texts"
        );
        for (key, x) in &ta {
            let y = &tb[key];
            let what = format!("{new}: text {key:?}");
            assert_eq!(x.content, y.content, "{what}");
            assert!(
                close(x.screen_position[0], y.screen_position[0], 1e-9),
                "{what}: screen x"
            );
            assert!(
                close(x.screen_position[1], y.screen_position[1], 1e-9),
                "{what}: screen y"
            );
            assert_eq!(x.anchor.kind, y.anchor.kind, "{what}: anchor kind");
            same_anchor_entity(x.anchor.entity, y.anchor.entity, same_ids, &what);
            assert!(
                close3(x.anchor.point_m, y.anchor.point_m, 1e-6),
                "{what}: anchor point"
            );
            assert!(
                close3(x.leader_offset, y.leader_offset, 1e-6),
                "{what}: leader offset"
            );
            assert_eq!(x.leader, y.leader, "{what}: leader");
            assert_eq!(x.arrow, y.arrow, "{what}: arrow");
            assert_eq!(font_of(&a, x.font), font_of(&b, y.font), "{what}: font");
        }
        let dims = |m: &Model| -> BTreeMap<i64, openskp::Dimension> {
            m.dimensions
                .iter()
                .map(|d| ((d.offset_m * 1e6).round() as i64, d.clone()))
                .collect()
        };
        let (da, db) = (dims(&a), dims(&b));
        assert_eq!(
            da.keys().collect::<Vec<_>>(),
            db.keys().collect::<Vec<_>>(),
            "{new}: dimensions"
        );
        for (key, x) in &da {
            let y = &db[key];
            let what = format!("{new}: dimension at {key}");
            assert_eq!(x.text_override, y.text_override, "{what}");
            for (p, q) in [(&x.start, &y.start), (&x.end, &y.end)] {
                assert_eq!(p.kind, q.kind, "{what}: anchor kind");
                same_anchor_entity(p.entity, q.entity, same_ids, &what);
                assert!(close3(p.point_m, q.point_m, 1e-6), "{what}: anchor point");
            }
            assert!(close3(x.normal, y.normal, 1e-9), "{what}: normal");
            assert!(close3(x.x_axis, y.x_axis, 1e-9), "{what}: x axis");
            assert!(close(x.offset_m, y.offset_m, 1e-6), "{what}: offset");
            assert_eq!(x.text_position, y.text_position, "{what}: text position");
            assert_eq!(x.aligned, y.aligned, "{what}: aligned");
            assert_eq!(font_of(&a, x.font), font_of(&b, y.font), "{what}: font");
        }

        // Section planes: the same planes, in the same local frames.
        let planes = |m: &Model| -> Vec<[i64; 4]> {
            let mut v: Vec<[i64; 4]> = m
                .geometry
                .iter()
                .flat_map(|r| r.sections.iter())
                .map(|s| s.plane.map(|x| (x * 1e6).round() as i64))
                .collect();
            v.sort();
            v
        };
        assert_eq!(planes(&a), planes(&b), "{new}: section planes");

        // Definitions: behaviour and timestamp, by name.
        let defs = |m: &Model| -> BTreeMap<String, (openskp::Behaviour, u32)> {
            m.definitions
                .iter()
                .map(|d| (d.name.clone(), (d.behaviour.clone(), d.timestamp)))
                .collect()
        };
        let (fa, fb) = (defs(&a), defs(&b));
        let mut checked = 0;
        for (name, x) in &fa {
            if let Some(y) = fb.get(name) {
                assert_eq!(x, y, "{new}: definition {name:?}");
                checked += 1;
            }
        }
        assert!(
            checked >= fa.len().min(fb.len()) / 2,
            "{new}: definitions compared {checked}"
        );
    }
}
