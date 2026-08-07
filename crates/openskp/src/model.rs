//! The unified [`Model`]: everything the reader decodes from a `.skp`, plus a
//! deterministic JSON serializer (used by the CLI and the differential tests).
//!
//! Port of `parse_model` in `tools/skpwalk.py`.

use crate::{
    extract, geometry_runs_with_diagnostics, header, AttrValue, Attribute, Definition, Diagnostic,
    GeometryRun, Guide, Image, Instance, Layer, Material, PlacedInstance, RunFilterReason,
    Topology, INCH,
};

/// Everything decoded from a `.skp` file.
#[derive(Debug, Clone)]
pub struct Model {
    pub version: String,
    pub format_guid: String,
    pub definitions: Vec<Definition>,
    pub instances: Vec<Instance>,
    pub geometry: Vec<GeometryRun>,
    pub materials: Vec<Material>,
    pub layers: Vec<Layer>,
    pub scenes: Vec<String>,
    pub guides: Vec<Guide>,
    pub attributes: Vec<Attribute>,
    pub images: Vec<Image>,
    /// Definition linkage (Phase 1.2/3.3): `(declared def map index,
    /// index into definitions)`, sorted. The def-refs carried by instances
    /// (top-level and in-list) resolve through this.
    pub definition_links: Vec<(u32, usize)>,
    /// Layer linkage (Phase 3.4): `(layer archive slot, index into
    /// layers)`. Slot 0 is the default layer (layers[0]); nonzero slots
    /// suffix-zip against layers[1..] — the same documented relative-order
    /// rule as materials (SKP_FORMAT §4q).
    pub layer_links: Vec<(u16, usize)>,
    /// Face-material linkage (Phase 3.2): `(archive slot, index into
    /// materials)`, sorted by slot. Built by the documented relative-order
    /// rule — sorted distinct referenced slots ↔ the trailing file-order
    /// materials (SKP_FORMAT §4n; consecutive-slot layout proven by
    /// back-material.skp's .dae).
    pub material_links: Vec<(u16, usize)>,
    /// Every parse anomaly recorded during the walk (Phase 0.4). Empty on a
    /// clean 2013–2017 file except for informational `RunFiltered` entries
    /// on files with known false-positive/fragment runs.
    pub diagnostics: Vec<Diagnostic>,
}

/// One node of the composed scene hierarchy (Phase 3.3).
#[derive(Debug, Clone)]
pub struct Node {
    /// The placed definition's name, when linked.
    pub definition: Option<String>,
    /// Index into `Model::geometry` of the definition's run, when found.
    pub run: Option<usize>,
    /// The instance's raw 13-f64 transform (inches translation).
    pub local: [f64; 13],
    /// Composed row-major 4×4 world transform; translation in METRES.
    /// Apply to mesh vertices as `p' = M · [p, 1]`.
    pub world: [f64; 16],
    /// EFFECTIVE inherited material slot (§4q instance matref, composed
    /// down the path: an instance's own matref wins, else the parent's;
    /// 0 = none). Default-material faces below this node render with it.
    pub material: u16,
    /// The placing instance's OWN §4q hidden flag (not composed — prune
    /// the subtree yourself for WYSIWYG semantics).
    pub hidden: bool,
    /// The placing instance's §4q layer slot (0 = default layer).
    pub layer: u16,
    /// `Some(true)` = placed by a `CGroup`, `Some(false)` = by a
    /// `CComponentInstance` (§4s class identity, as on [`Instance`]);
    /// `None` on the legacy byte-scan path, which cannot tell them apart.
    pub is_group: Option<bool>,
    pub children: Vec<Node>,
}

const IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// The 13-f64 transform as a row-major 4×4 (SKP_FORMAT §4e): t[0..9] is the
/// 3×3 with t[0..3] as its FIRST ROW (column-vector convention `p' = M·p`
/// — component-rotate.skp's .dae discriminates this from the transposed
/// reading), t[9..12] the translation in inches (metres here), t[12] a
/// weight.
fn mat_of(t: &[f64; 13]) -> [f64; 16] {
    const M_PER_IN: f64 = 0.0254;
    [
        t[0],
        t[1],
        t[2],
        t[9] * M_PER_IN,
        t[3],
        t[4],
        t[5],
        t[10] * M_PER_IN,
        t[6],
        t[7],
        t[8],
        t[11] * M_PER_IN,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

fn mat_mul(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    let mut out = [0.0f64; 16];
    for r in 0..4 {
        for c in 0..4 {
            out[r * 4 + c] = (0..4).map(|k| a[r * 4 + k] * b[k * 4 + c]).sum();
        }
    }
    out
}

/// A read/parse error.
#[derive(Debug, Clone)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for Error {}

impl Model {
    /// Parse a `.skp` file's bytes into a [`Model`].
    ///
    /// The CONTINUOUS single-archive walk (SKP_FORMAT §4s) is tried first: it
    /// is the exact reading of the model section (one global store map, so
    /// cross-list back-refs resolve structurally — house.skp needs it). If
    /// its anchor/calibration fails, it dies mid-walk, or its result fails
    /// the cross-checks, the legacy run-based path serves the file and the
    /// abandoned attempt is recorded as a `ContinuousFallback` diagnostic —
    /// which path ran is never silent.
    pub fn parse(d: &[u8]) -> Result<Model, Error> {
        // Container gate (Phase 0.1): refuse cleanly instead of walking a
        // layout we don't understand (e.g. post-2017 files — see
        // corpus/future/). `header_info` still identifies such files.
        if crate::ctx::detect_container(d) != crate::ctx::Container::Carchive2017 {
            return Err(Error(
                "unsupported .skp container: not the 2013–2017 MFC CArchive layout \
                 (a post-2017 SketchUp file? re-save as SketchUp 2017 to read it here)"
                    .into(),
            ));
        }
        let hdr = header::parse_header(d).ok_or_else(|| Error("malformed .skp header".into()))?;
        match crate::walk2::walk(d) {
            Ok(cw) => match Self::from_continuous(d, &hdr, &cw) {
                Some(m) => Ok(m),
                None => {
                    let mut m = Self::parse_legacy(d, hdr)?;
                    m.diagnostics.push(Diagnostic::ContinuousFallback {
                        stage: "validation".into(),
                        at: cw.end,
                        detail: "completed walk failed the defref cross-check".into(),
                    });
                    Ok(m)
                }
            },
            Err(f) => {
                let mut m = Self::parse_legacy(d, hdr)?;
                m.diagnostics.push(Diagnostic::ContinuousFallback {
                    stage: f.stage.into(),
                    at: f.at,
                    detail: f.detail,
                });
                Ok(m)
            }
        }
    }

    /// The legacy run-based path (fresh per-run maps + byte-scan
    /// extractors) — every pre-§4s behavior, bit-identical.
    fn parse_legacy(d: &[u8], hdr: header::Header) -> Result<Model, Error> {
        let (geometry, mut diagnostics) = geometry_runs_with_diagnostics(d);
        // Phase 1.2: linkage consumes the runs' declared map indexes; a
        // fallback to the positional zip is recorded, never silent.
        let (definitions, instances, definition_links, link_diags) =
            extract::component_tree(d, &geometry);
        diagnostics.extend(link_diags);
        // Shared-texture back-refs stay unresolved on this path: placing
        // the referenced CDib slot needs the §4s global anchor, which the
        // byte-scan path never establishes.
        let (materials, _shared_refs) = extract::materials(d);
        // Phase 3.2: link face material slots to the materials list by the
        // documented relative-order rule (SKP_FORMAT §4n).
        let mut refs: Vec<u16> = geometry
            .iter()
            .flat_map(|r| r.mesh.faces.iter())
            .flat_map(|f| [f.front_material, f.back_material])
            .flatten()
            .collect();
        refs.sort_unstable();
        refs.dedup();
        let material_links: Vec<(u16, usize)> = if refs.len() <= materials.len() {
            let first = materials.len() - refs.len();
            refs.iter()
                .enumerate()
                .map(|(i, &slot)| (slot, first + i))
                .collect()
        } else {
            Vec::new()
        };
        let layers = extract::layers(d);
        let mut lrefs: Vec<u16> = geometry
            .iter()
            .flat_map(|r| {
                r.mesh
                    .edges
                    .iter()
                    .map(|e| e.layer)
                    .chain(r.mesh.faces.iter().map(|f| f.layer))
            })
            .filter(|&l| l != 0)
            .collect();
        lrefs.sort_unstable();
        lrefs.dedup();
        let mut layer_links: Vec<(u16, usize)> = Vec::new();
        if !layers.is_empty() {
            layer_links.push((0, 0)); // slot 0 = the default layer
            if lrefs.len() < layers.len() {
                let first = layers.len() - lrefs.len();
                layer_links.extend(lrefs.iter().enumerate().map(|(i, &s)| (s, first + i)));
            }
        }
        Ok(Model {
            version: hdr.version,
            format_guid: hdr.format_guid,
            definitions,
            instances,
            geometry,
            materials,
            layers,
            scenes: extract::scenes(d),
            guides: extract::guides(d),
            attributes: extract::attributes(d),
            images: extract::images(d),
            definition_links,
            layer_links,
            material_links,
            diagnostics,
        })
    }

    /// Build the [`Model`] from a COMPLETED continuous walk (§4s), or
    /// `None` when the result fails its cross-checks (a wrong calibration
    /// can complete mechanically but mislink — every instance def-ref must
    /// land on a definition object in the global map; house's 47 do).
    fn from_continuous(
        d: &[u8],
        hdr: &header::Header,
        cw: &crate::walk2::Continuous,
    ) -> Option<Model> {
        use crate::carchive::Slot;
        use crate::entity::Entity;
        let map = &cw.map;

        // Cross-check: every placed instance references a definition by its
        // ACTUAL global map slot (§4s; the "Birch Plywood" declared-index
        // drift proves the read-side slot is the authoritative key).
        for slot in map.iter() {
            if let Slot::Object(Entity::InstancePlaced { defref, .. }) = slot {
                let ok = matches!(
                    map.get(*defref as usize),
                    Some(Slot::Object(Entity::ComponentDef { .. }))
                );
                if !ok {
                    return None;
                }
            }
        }

        // Layers: the layer-LIST objects only (each definition's inline
        // Layer0 copy stays out of the model layer table).
        let layers: Vec<Layer> = cw
            .layer_slots
            .iter()
            .filter_map(|&s| match &map[s] {
                Slot::Object(Entity::Layer {
                    name, hidden, rgba, ..
                }) => Some(Layer {
                    name: name.clone(),
                    visible: !hidden,
                    rgba: *rgba,
                }),
                _ => None,
            })
            .collect();
        // Layer linkage is EXACT here: an entity's drawbase layer u16 is the
        // layer object's global map slot (0 = the default layer, §4q). Every
        // CLayer object in the map (list layers AND per-definition Layer0
        // copies) links by display name.
        let mut layer_links: Vec<(u16, usize)> = vec![(0, 0)];
        for (slot, entry) in map.iter().enumerate() {
            if let Slot::Object(Entity::Layer { name, .. }) = entry {
                if let (Ok(s16), Some(i)) = (
                    u16::try_from(slot),
                    layers.iter().position(|l| &l.name == name),
                ) {
                    layer_links.push((s16, i));
                }
            }
        }

        // Definitions, in serialization order, keyed by ACTUAL map slot.
        let mut definitions = Vec::with_capacity(cw.defs.len());
        let mut definition_links: Vec<(u32, usize)> = Vec::new();
        for ds in &cw.defs {
            let Slot::Object(Entity::ComponentDef { name, guid, .. }) = &map[ds.slot] else {
                return None;
            };
            if let Ok(slot) = u32::try_from(ds.slot) {
                definition_links.push((slot, definitions.len()));
            }
            definitions.push(Definition {
                name: name.clone(),
                guid: guid.clone(),
                map_index: Some(ds.slot),
            });
        }
        definition_links.sort_unstable();

        // One geometry run per definition (its subtree is a contiguous
        // global-map slot range) + one for the root entity list.
        let run_of = |start: usize,
                      end: usize,
                      top_level: usize,
                      frame: Option<usize>,
                      def_index: Option<usize>,
                      lo: usize,
                      hi: usize|
         -> GeometryRun {
            let count = |cls: &str| {
                map[lo.min(map.len())..hi.min(map.len())]
                    .iter()
                    .filter(|s| matches!(s, Slot::Object(e) if e.class_name() == cls))
                    .count()
            };
            let resolved = crate::walk2::resolve_range(map, lo, hi);
            GeometryRun {
                start,
                end,
                top_level,
                frame,
                def_index,
                topology: Topology {
                    vertices: count("CVertex"),
                    edges: count("CEdge"),
                    faces: count("CFace"),
                    loops: count("CLoop"),
                    edge_uses: count("CEdgeUse"),
                    curves: count("CArcCurve"),
                    dimensions: count("CDimensionLinear"),
                    texts: count("CText"),
                    section_planes: count("CSectionPlane"),
                    images_placed: count("CImage"),
                    construction_points: count("CConstructionPoint"),
                },
                resolved,
                // Global map ⇒ a back-ref `r` IS `map[r]`: base 1 makes
                // mesh::build_range's `map[r - base + 1]` the identity.
                mesh: crate::mesh::build_range(map, Some(1), lo, hi),
                placed: map[lo.min(map.len())..hi.min(map.len())]
                    .iter()
                    .filter_map(|s| match s {
                        Slot::Object(Entity::InstancePlaced {
                            defref,
                            transform,
                            material,
                            hidden,
                            layer,
                            is_group,
                            ..
                        }) => Some(PlacedInstance {
                            defref: *defref,
                            transform: *transform,
                            material: *material,
                            hidden: *hidden,
                            layer: *layer,
                            is_group: *is_group,
                        }),
                        _ => None,
                    })
                    .collect(),
                curve_members: map[lo.min(map.len())..hi.min(map.len())]
                    .iter()
                    .filter_map(|s| match s {
                        Slot::Object(Entity::Curve { members, .. }) => Some(*members),
                        _ => None,
                    })
                    .collect(),
            }
        };
        let mut geometry = Vec::with_capacity(cw.defs.len() + 1);
        for (i, ds) in cw.defs.iter().enumerate() {
            let lo = ds.slot + 1;
            let hi = cw
                .defs
                .get(i + 1)
                .map(|n| n.slot)
                .unwrap_or(cw.root_slot_floor);
            let (top_level, frame) = match &map[ds.slot] {
                Slot::Object(Entity::ComponentDef {
                    entities, count, ..
                }) => (entities.len(), Some(*count)),
                _ => (0, None),
            };
            geometry.push(run_of(
                ds.start,
                ds.end,
                top_level,
                frame,
                Some(ds.slot),
                lo,
                hi,
            ));
        }
        // The root run's FILE span is deliberately empty (start == end at
        // the count u32): `scene()` treats instances whose transform lies
        // outside every run as scene roots, and the root list's named
        // groups/instances are exactly those.
        geometry.push(run_of(
            cw.root_list_at,
            cw.root_list_at,
            cw.roots.len(),
            Some(cw.roots.len()),
            None,
            cw.root_slot_floor,
            map.len(),
        ));

        // Instances: every CGroup/CComponentInstance in the map, in
        // serialization order, linked by the def-ref = global slot.
        let round5 = |x: f64| (x * 1e5).round() / 1e5;
        let mut instances = Vec::new();
        for entry in map.iter() {
            if let Slot::Object(Entity::InstancePlaced {
                defref,
                transform,
                name,
                tf_at,
                is_group,
                material,
                hidden,
                layer,
                ..
            }) = entry
            {
                let definition = match map.get(*defref as usize) {
                    Some(Slot::Object(Entity::ComponentDef { name, .. })) => Some(name.clone()),
                    _ => None,
                };
                instances.push(Instance {
                    definition,
                    defref: *defref,
                    offset: *tf_at,
                    translation_m: [
                        round5(transform[9] / INCH),
                        round5(transform[10] / INCH),
                        round5(transform[11] / INCH),
                    ],
                    transform: *transform,
                    name: Some(name.clone()),
                    is_group: Some(*is_group),
                    material: Some(*material),
                    hidden: Some(*hidden),
                    layer: Some(*layer),
                });
            }
        }

        // Materials: content still comes from the byte-scan extractor;
        // BINDING is the §4s slot arithmetic, validated against the
        // observed matrefs (fallback = the documented §4n suffix zip).
        // The same anchor resolves shared-texture back-refs to their
        // owning material's image bytes.
        let (mut materials, shared_refs) = extract::materials(d);
        let (material_links, mut diagnostics) =
            continuous_material_links(d, &mut materials, &shared_refs, cw.base, &geometry);

        diagnostics.insert(0, Diagnostic::ContinuousWalk { base: cw.base });
        diagnostics.extend(
            cw.bound
                .iter()
                .map(|(slot, class)| Diagnostic::PadSlotBound {
                    slot: *slot,
                    class: class.clone(),
                }),
        );

        Some(Model {
            version: hdr.version.clone(),
            format_guid: hdr.format_guid.clone(),
            definitions,
            instances,
            geometry,
            materials,
            layers,
            scenes: extract::scenes(d),
            guides: extract::guides(d),
            attributes: extract::attributes(d),
            images: extract::images(d),
            definition_links,
            layer_links,
            material_links,
            diagnostics,
        })
    }

    /// The scene hierarchy (Phase 3.3): one [`Node`] per top-level
    /// instance (instances whose transform block lies OUTSIDE every
    /// definition run — in-definition children are reachable through
    /// `GeometryRun::placed` and appear as node children instead). World
    /// transforms compose `parent × local` with translations in metres.
    pub fn scene(&self) -> Vec<Node> {
        let top_level = self.instances.iter().filter(|i| {
            !self
                .geometry
                .iter()
                .any(|r| r.start <= i.offset && i.offset < r.end)
        });
        top_level
            .map(|i| {
                self.node_for(
                    i.defref,
                    &i.transform,
                    &IDENTITY,
                    i.material.unwrap_or(0),
                    i.hidden.unwrap_or(false),
                    i.layer.unwrap_or(0),
                    i.is_group,
                    0,
                )
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn node_for(
        &self,
        defref: u32,
        local: &[f64; 13],
        parent: &[f64; 16],
        material: u16,
        hidden: bool,
        layer: u16,
        is_group: Option<bool>,
        depth: u32,
    ) -> Node {
        let world = mat_mul(parent, &mat_of(local));
        let run = self
            .geometry
            .iter()
            .position(|r| r.def_index == Some(defref as usize));
        let definition = self
            .definition_links
            .iter()
            .find(|(dr, _)| *dr == defref)
            .map(|(_, k)| self.definitions[*k].name.clone());
        let children = if depth < 64 {
            run.map(|ri| {
                self.geometry[ri]
                    .placed
                    .iter()
                    .map(|p| {
                        // §4q inheritance: the child's own matref wins
                        let m = if p.material != 0 {
                            p.material
                        } else {
                            material
                        };
                        self.node_for(
                            p.defref,
                            &p.transform,
                            &world,
                            m,
                            p.hidden,
                            p.layer,
                            Some(p.is_group),
                            depth + 1,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
        } else {
            Vec::new() // cycle guard: malformed self-referential files
        };
        Node {
            definition,
            run,
            local: *local,
            world,
            material,
            hidden,
            layer,
            is_group,
            children,
        }
    }

    /// The layer an entity's drawbase slot refers to (Phase 3.4).
    pub fn layer_of(&self, slot: u16) -> Option<&crate::Layer> {
        self.layer_links
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, i)| &self.layers[*i])
    }

    /// The material a face-side archive slot refers to (Phase 3.2).
    pub fn material_of(&self, slot: u16) -> Option<&crate::Material> {
        self.material_links
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, i)| &self.materials[*i])
    }

    /// The applied texture size (inches) behind a face-side material slot —
    /// `Some` only for textured materials, i.e. exactly when the §4v UV
    /// formula applies to that side.
    pub fn applied_size_of(&self, slot: u16) -> Option<(f64, f64)> {
        match self.material_of(slot)? {
            crate::Material::Textured {
                applied_size_in: Some((w, h)),
                ..
            } if *w > 0.0 && *h > 0.0 => Some((*w, *h)),
            _ => None,
        }
    }

    /// Read and parse a `.skp` file from disk.
    pub fn read(path: impl AsRef<std::path::Path>) -> Result<Model, Error> {
        let d = std::fs::read(path).map_err(|e| Error(e.to_string()))?;
        Model::parse(&d)
    }

    /// Serialize to JSON (stable key order; mirrors the reference parser's shape).
    pub fn to_json(&self) -> String {
        let mut j = Json::new();
        j.begin_obj();
        j.m_str("version", &self.version);
        j.m_str("format_guid", &self.format_guid);

        j.key("definitions");
        j.arr(&self.definitions, |j, def| {
            j.begin_obj();
            j.m_str("name", &def.name);
            j.m_str("guid", &def.guid);
            j.end_obj();
        });

        j.key("instances");
        j.arr(&self.instances, |j, inst| {
            j.begin_obj();
            j.key("definition");
            match &inst.definition {
                Some(s) => j.raw_str(s),
                None => j.raw_null(),
            }
            j.key("translation_m");
            j.f64_arr(&inst.translation_m);
            j.end_obj();
        });

        j.key("geometry_runs");
        j.arr(&self.geometry, |j, r: &GeometryRun| {
            j.begin_obj();
            j.m_usize("start", r.start);
            j.m_usize("vertices", r.topology.vertices);
            j.m_usize("edges", r.topology.edges);
            j.m_usize("faces", r.topology.faces);
            j.m_usize("loops", r.topology.loops);
            j.m_usize("edge_uses", r.topology.edge_uses);
            j.m_usize("curves", r.topology.curves);
            j.key("resolved");
            j.begin_arr();
            j.comma();
            j.raw_usize(r.resolved.0);
            j.comma();
            j.raw_usize(r.resolved.1);
            j.end_arr();
            // §4v: per-face UVs (outer-ring corners, `[u,v]` pairs) for every
            // face side painted with a TEXTURED material, plus the §4u pin
            // lists when present. Solid-painted and bare faces are omitted.
            j.key("textured_faces");
            let textured: Vec<&crate::MeshFace> = r
                .mesh
                .faces
                .iter()
                .filter(|f| {
                    [f.front_material, f.back_material]
                        .iter()
                        .any(|m| m.and_then(|slot| self.applied_size_of(slot)).is_some())
                })
                .collect();
            j.arr(&textured, |j, f| {
                j.begin_obj();
                j.m_usize("pid", f.pid as usize);
                for (name, mat, side) in [
                    ("uv_front", f.front_material, crate::Side::Front),
                    ("uv_back", f.back_material, crate::Side::Back),
                ] {
                    let Some(size) = mat.and_then(|slot| self.applied_size_of(slot)) else {
                        continue;
                    };
                    let Some(x) = f.uv_xform(side, size) else {
                        continue;
                    };
                    j.key(name);
                    j.begin_arr();
                    for &vi in &f.outer {
                        j.comma();
                        j.f64_arr(&x.apply(r.mesh.vertices[vi as usize]));
                    }
                    j.end_arr();
                }
                if let Some(t) = &f.texture {
                    for (name, pins) in [("pins_front", &t.front_pins), ("pins_back", &t.back_pins)]
                    {
                        if !pins.is_empty() {
                            j.key(name);
                            j.arr(pins, |j, p| j.f64_arr(p));
                        }
                    }
                }
                j.end_obj();
            });
            j.end_obj();
        });

        j.key("materials");
        j.arr(&self.materials, |j, m| match m {
            Material::Solid {
                name,
                rgba,
                opacity,
            } => {
                j.begin_obj();
                j.m_str("name", name);
                j.m_str("kind", "solid");
                j.key("rgba");
                j.u8_arr(rgba);
                j.m_f64("opacity", *opacity);
                j.end_obj();
            }
            Material::Textured {
                name,
                texture,
                applied_size_in,
                // API-level only; the frozen-oracle JSON predates them
                image_bytes: _,
                avg_rgba: _,
                opacity: _,
            } => {
                j.begin_obj();
                j.m_str("name", name);
                j.m_str("kind", "textured");
                j.key("texture");
                match texture {
                    Some(s) => j.raw_str(s),
                    None => j.raw_null(),
                }
                j.key("applied_size_in");
                match applied_size_in {
                    Some((w, h)) => j.f64_arr(&[*w, *h]),
                    None => j.raw_null(),
                }
                j.end_obj();
            }
        });

        j.key("layers");
        j.arr(&self.layers, |j, l| {
            j.begin_obj();
            j.m_str("name", &l.name);
            j.m_bool("visible", l.visible);
            j.key("rgba");
            j.u8_arr(&l.rgba);
            j.end_obj();
        });

        j.key("scenes");
        j.arr(&self.scenes, |j, s| j.raw_str(s));

        j.key("guides");
        j.arr(&self.guides, |j, g: &Guide| {
            j.begin_obj();
            j.key("point_m");
            j.f64_arr(&g.point_m);
            j.key("direction");
            j.f64_arr(&g.direction);
            j.end_obj();
        });

        j.key("attributes");
        j.arr(&self.attributes, |j, a: &Attribute| {
            j.begin_arr();
            j.comma();
            j.raw_str(&a.key);
            j.comma();
            match &a.value {
                AttrValue::Int(v) => {
                    j.raw_str("int");
                    j.comma();
                    j.raw_i64(*v as i64);
                }
                AttrValue::F64(v) => {
                    j.raw_str("f64");
                    j.comma();
                    j.raw_f64(*v);
                }
                AttrValue::Bool(b) => {
                    j.raw_str("bool");
                    j.comma();
                    j.raw_bool(*b);
                }
            }
            j.end_arr();
        });

        j.key("images");
        j.arr(&self.images, |j, im: &Image| {
            j.begin_obj();
            j.m_str("kind", &im.kind);
            j.m_usize("bytes", im.bytes);
            j.end_obj();
        });

        // Only DESYNC diagnostics reach the JSON, and only when present: the
        // frozen Python-reference oracle predates diagnostics, and CLEAN
        // corpus files must keep comparing equal to it — including files
        // whose informational RunFiltered entries fire by design
        // (attributes.skp's fragment filtering). The zero-desync invariant
        // makes absence the norm; RunFiltered stays API-level on
        // `Model::diagnostics`.
        let desync: Vec<Diagnostic> = self
            .diagnostics
            .iter()
            .filter(|di| di.is_desync())
            .cloned()
            .collect();
        if !desync.is_empty() {
            j.key("diagnostics");
            j.arr(&desync, |j, di: &Diagnostic| {
                j.begin_obj();
                match di {
                    Diagnostic::Resync {
                        at,
                        resumed_at,
                        class,
                    } => {
                        j.m_str("kind", "resync");
                        j.m_usize("at", *at);
                        j.m_usize("resumed_at", *resumed_at);
                        j.m_str("class", class);
                    }
                    Diagnostic::Truncated { at } => {
                        j.m_str("kind", "truncated");
                        j.m_usize("at", *at);
                    }
                    Diagnostic::Skipped { class, at, bytes } => {
                        j.m_str("kind", "skipped");
                        j.m_str("class", class);
                        j.m_usize("at", *at);
                        j.m_usize("bytes", *bytes);
                    }
                    Diagnostic::HeuristicFraming { at } => {
                        // Informational; excluded by the is_desync filter
                        // above — serialized here only for match completeness.
                        j.m_str("kind", "heuristic_framing");
                        j.m_usize("at", *at);
                    }
                    Diagnostic::HeuristicAnchor { at } => {
                        // Informational; excluded by the is_desync filter
                        // above — serialized here only for match completeness.
                        j.m_str("kind", "heuristic_anchor");
                        j.m_usize("at", *at);
                    }
                    Diagnostic::HeuristicLinkage { definitions } => {
                        // Informational; excluded by the is_desync filter
                        // above — serialized here only for match completeness.
                        j.m_str("kind", "heuristic_linkage");
                        j.m_usize("definitions", *definitions);
                    }
                    Diagnostic::ContinuousWalk { base } => {
                        // Informational; excluded by the is_desync filter
                        // above — serialized here only for match completeness.
                        j.m_str("kind", "continuous_walk");
                        j.m_usize("base", *base);
                    }
                    Diagnostic::ContinuousFallback { stage, at, detail } => {
                        // Informational; excluded by the is_desync filter
                        // above — serialized here only for match completeness.
                        j.m_str("kind", "continuous_fallback");
                        j.m_str("stage", stage);
                        j.m_usize("at", *at);
                        j.m_str("detail", detail);
                    }
                    Diagnostic::PadSlotBound { slot, class } => {
                        // Informational; excluded by the is_desync filter
                        // above — serialized here only for match completeness.
                        j.m_str("kind", "pad_slot_bound");
                        j.m_usize("slot", *slot);
                        j.m_str("class", class);
                    }
                    Diagnostic::MaterialLinkFallback {
                        declared,
                        extracted,
                    } => {
                        // Informational; excluded by the is_desync filter
                        // above — serialized here only for match completeness.
                        j.m_str("kind", "material_link_fallback");
                        j.m_usize("declared", *declared);
                        j.m_usize("extracted", *extracted);
                    }
                    Diagnostic::RunFiltered { start, reason } => {
                        j.m_str("kind", "run_filtered");
                        j.m_usize("start", *start);
                        match reason {
                            RunFilterReason::Degenerate => j.m_str("reason", "degenerate"),
                            RunFilterReason::LowConfidence {
                                satisfied,
                                constraints,
                            } => {
                                j.m_str("reason", "low_confidence");
                                j.m_usize("satisfied", *satisfied);
                                j.m_usize("constraints", *constraints);
                            }
                        }
                    }
                }
                j.end_obj();
            });
        }

        j.end_obj();
        j.out
    }

    /// The concrete mesh as JSON (Phase 6.3, the C-ABI mesh surface):
    /// per-run vertices (METRES) / edges / faces (ordered rings, §4v
    /// texture data), plus the composed `scene` leaves — `(run, 16-f64
    /// row-major world matrix, inherited material slot)` — from which a
    /// consumer assembles the world-space model exactly like the
    /// equivalence harness does.
    pub fn mesh_json(&self) -> String {
        let mut j = Json::new();
        j.begin_obj();
        j.key("runs");
        j.arr(&self.geometry, |j, r: &GeometryRun| {
            j.begin_obj();
            j.key("vertices_m");
            j.arr(&r.mesh.vertices, |j, v| j.f64_arr(v));
            j.key("edges");
            j.arr(&r.mesh.edges, |j, e| {
                j.begin_obj();
                j.m_usize("v0", e.v0 as usize);
                j.m_usize("v1", e.v1 as usize);
                j.m_bool("soft", e.soft);
                j.m_bool("smooth", e.smooth);
                j.m_bool("hidden", e.hidden);
                j.end_obj();
            });
            j.key("faces");
            j.arr(&r.mesh.faces, |j, f| {
                j.begin_obj();
                j.m_usize("pid", f.pid as usize);
                j.key("outer");
                j.begin_arr();
                for &vi in &f.outer {
                    j.comma();
                    j.raw_usize(vi as usize);
                }
                j.end_arr();
                j.key("holes");
                j.arr(&f.holes, |j, h| {
                    j.begin_arr();
                    for &vi in h {
                        j.comma();
                        j.raw_usize(vi as usize);
                    }
                    j.end_arr();
                });
                j.key("normal");
                j.f64_arr(&f.normal);
                j.key("front_material");
                match f.front_material {
                    Some(s) => j.raw_usize(s as usize),
                    None => j.raw_null(),
                }
                j.key("back_material");
                match f.back_material {
                    Some(s) => j.raw_usize(s as usize),
                    None => j.raw_null(),
                }
                j.m_bool("hidden", f.hidden);
                j.end_obj();
            });
            j.end_obj();
        });
        // Composed scene leaves: DFS over Model::scene().
        j.key("scene");
        j.begin_arr();
        fn rec(j: &mut Json, n: &Node) {
            if let Some(ri) = n.run {
                j.comma();
                j.begin_obj();
                j.m_usize("run", ri);
                j.key("world");
                j.f64_arr(&n.world);
                j.m_usize("material", n.material as usize);
                j.end_obj();
            }
            for c in &n.children {
                rec(j, c);
            }
        }
        for n in self.scene() {
            rec(&mut j, &n);
        }
        // the root run (loose model-level geometry) at identity
        if let Some(root) = self.geometry.iter().position(|r| r.def_index.is_none()) {
            j.comma();
            j.begin_obj();
            j.m_usize("run", root);
            j.key("world");
            j.f64_arr(&IDENTITY);
            j.m_usize("material", 0);
            j.end_obj();
        }
        j.end_arr();
        j.end_obj();
        j.out
    }
}

/// Face→material binding on the continuous path (SKP_FORMAT §4s): the
/// pre-model slot layout ends `…materials…` then the CLayer class at
/// `base + 1`, with each textured material's inline CDib consuming one slot
/// right after it — so the FIRST material sits at `base − (M + D) + 1`
/// where `M` = material count and `D` = the number of materials with their
/// OWN dib (a duplicate texture is an object back-ref and consumes no slot:
/// house's "[Wood Floor Light]1" refs slot 23, the original's dib).
/// house.skp: base 30, M 9, D 5 → first slot 17, and the observed matrefs
/// {17,18,20,21,22,26,28,29} land exactly on the 9 material slots.
///
/// Validation gates the arithmetic: the manager's declared count (the u32
/// before the CMaterial new-class record) must match the extractor's list,
/// and every observed face matref must land on a material slot. On any
/// disagreement the documented §4n suffix alignment serves instead,
/// recorded as `MaterialLinkFallback`.
///
/// The slot anchor also resolves shared-texture back-refs (§8.1): the
/// record's u16 names the OWNING material's CDib global map slot, so once
/// slots are placed the owner is exact and its image bytes are copied onto
/// the sharing material (house.skp: "[Wood Floor Light]1" refs 23, the
/// slot after "[Wood Floor Light]" at 22; the theater's four shared
/// records land on their like-named originals' dibs the same way). The
/// suffix fallback places no slots, so back-refs stay unresolved there.
fn continuous_material_links(
    d: &[u8],
    materials: &mut [Material],
    shared_refs: &[(usize, u16)],
    base: usize,
    geometry: &[GeometryRun],
) -> (Vec<(u16, usize)>, Vec<Diagnostic>) {
    let mut refs: Vec<u16> = geometry
        .iter()
        .flat_map(|r| r.mesh.faces.iter())
        .flat_map(|f| [f.front_material, f.back_material])
        .flatten()
        .collect();
    refs.sort_unstable();
    refs.dedup();

    let declared = cmaterial_declared_count(d);
    let own_dib: Vec<bool> = materials
        .iter()
        .map(|m| {
            matches!(
                m,
                Material::Textured {
                    image_bytes: Some(_),
                    ..
                }
            )
        })
        .collect();
    // Exact path first: walk the CMaterial region as the object sequence
    // it is (matwalk) and anchor the relative slots on the face refs. The
    // walk validates itself end-to-end (record adjacency + footer); any
    // surprise falls through to the old arithmetic, then the suffix rule.
    //
    // Walk records correlate to the EXTRACTED material list BY NAME, in
    // order — the signature extractor can miss a record the walk finds
    // (its slot then simply links to no material), and duplicate names
    // stay unambiguous because both sequences are file-ordered.
    if let Some(slots) = crate::matwalk::walk_region(d) {
        if let Some(links) = crate::matwalk::links_from_walk(&slots, &refs, base + 1) {
            fn name_of(m: &Material) -> &str {
                match m {
                    Material::Solid { name, .. } => name,
                    Material::Textured { name, .. } => name,
                }
            }
            let mut mapped: Vec<(u16, usize)> = Vec::with_capacity(links.len());
            let mut by_walk: Vec<Option<usize>> = vec![None; slots.len()];
            let mut j = 0usize;
            for &(slot, wi) in &links {
                let want = &slots[wi].name;
                let mut k = j;
                while k < materials.len() && name_of(&materials[k]) != want {
                    k += 1;
                }
                if k < materials.len() {
                    mapped.push((slot, k));
                    by_walk[wi] = Some(k);
                    j = k + 1;
                }
                // else: the extractor missed this record; its slot links to
                // no material (faces on it degrade to unpainted).
            }
            if !mapped.is_empty() {
                // Anchored shared-texture resolution: back-ref slot →
                // walked dib → owning material's bytes.
                let shift = links[0].0 as usize - slots[links[0].1].rel;
                for &(mi, dib_slot) in shared_refs {
                    let owner = slots
                        .iter()
                        .position(|s| s.dib_rel.is_some_and(|r| r + shift == dib_slot as usize))
                        .and_then(|wi| by_walk[wi]);
                    if let Some(owner) = owner {
                        adopt_shared_bytes(materials, mi, owner);
                    }
                }
                return (mapped, Vec::new());
            }
        }
    }

    let arithmetic = || -> Option<Vec<(u16, usize)>> {
        if let Some(dc) = declared {
            if dc != materials.len() {
                return None;
            }
        }
        let d_count = own_dib.iter().filter(|b| **b).count();
        let first = (base + 1).checked_sub(materials.len() + d_count)?;
        if first == 0 {
            return None;
        }
        let mut links = Vec::with_capacity(materials.len());
        let mut slot = first;
        for (i, own) in own_dib.iter().enumerate() {
            links.push((u16::try_from(slot).ok()?, i));
            slot += 1 + usize::from(*own);
        }
        let slots: std::collections::HashSet<u16> = links.iter().map(|(s, _)| *s).collect();
        refs.iter().all(|r| slots.contains(r)).then_some(links)
    };
    match arithmetic() {
        Some(links) => {
            // Same shared-texture resolution under the arithmetic model:
            // a material's own dib occupies the slot right after it.
            for &(mi, dib_slot) in shared_refs {
                let owner = links.iter().find_map(|&(s, i)| {
                    (own_dib[i] && s as usize + 1 == dib_slot as usize).then_some(i)
                });
                if let Some(owner) = owner {
                    adopt_shared_bytes(materials, mi, owner);
                }
            }
            (links, Vec::new())
        }
        None => {
            let diag = Diagnostic::MaterialLinkFallback {
                declared: declared.unwrap_or(0),
                extracted: materials.len(),
            };
            // The documented §4n relative-order rule (legacy shape).
            let links = if refs.len() <= materials.len() {
                let first = materials.len() - refs.len();
                refs.iter()
                    .enumerate()
                    .map(|(i, &slot)| (slot, first + i))
                    .collect()
            } else {
                Vec::new()
            };
            (links, vec![diag])
        }
    }
}

/// Copy the owning material's embedded image onto a shared-texture
/// material (§8.1: its back-ref names the owner's CDib slot). A no-op
/// unless the owner actually carries inline bytes.
fn adopt_shared_bytes(materials: &mut [Material], shared: usize, owner: usize) {
    if shared == owner || shared >= materials.len() {
        return;
    }
    let Material::Textured {
        image_bytes: Some(b),
        ..
    } = &materials[owner]
    else {
        return;
    };
    let b = b.clone();
    if let Material::Textured { image_bytes, .. } = &mut materials[shared] {
        *image_bytes = Some(b);
    }
}

/// The material manager's declared count: the u32 immediately before the
/// `CMaterial` new-class record (house.skp: 9 @0x11f50, decl @0x11f54).
fn cmaterial_declared_count(d: &[u8]) -> Option<usize> {
    let pat = b"\x09\x00CMaterial"; // namelen + name of the FF FF record
    let i = d.windows(pat.len()).position(|w| w == pat)?;
    if i < 8 || d.get(i.checked_sub(4)?..i - 2) != Some(&[0xFF, 0xFF][..]) {
        return None;
    }
    let c = u32::from_le_bytes([d[i - 8], d[i - 7], d[i - 6], d[i - 5]]) as usize;
    (c <= 100_000).then_some(c)
}

/// A tiny, dependency-free JSON writer. Containers push a "has content" flag;
/// [`Json::comma`] emits a separator before every element/member after the first.
struct Json {
    out: String,
    need_comma: Vec<bool>,
}

impl Json {
    fn new() -> Self {
        Json {
            out: String::new(),
            need_comma: Vec::new(),
        }
    }
    /// Emit a comma before this element if the current container already has one.
    fn comma(&mut self) {
        if let Some(last) = self.need_comma.last_mut() {
            if *last {
                self.out.push(',');
            } else {
                *last = true;
            }
        }
    }
    fn begin_obj(&mut self) {
        self.out.push('{');
        self.need_comma.push(false);
    }
    fn end_obj(&mut self) {
        self.out.push('}');
        self.need_comma.pop();
    }
    fn begin_arr(&mut self) {
        self.out.push('[');
        self.need_comma.push(false);
    }
    fn end_arr(&mut self) {
        self.out.push(']');
        self.need_comma.pop();
    }
    /// Object member key: `,"k":` — the following value writes only its token.
    fn key(&mut self, k: &str) {
        self.comma();
        escape_into(&mut self.out, k);
        self.out.push(':');
    }
    // raw value tokens (no comma; caller establishes context)
    fn raw_str(&mut self, s: &str) {
        escape_into(&mut self.out, s);
    }
    fn raw_null(&mut self) {
        self.out.push_str("null");
    }
    fn raw_bool(&mut self, b: bool) {
        self.out.push_str(if b { "true" } else { "false" });
    }
    fn raw_usize(&mut self, v: usize) {
        self.out.push_str(&v.to_string());
    }
    fn raw_i64(&mut self, v: i64) {
        self.out.push_str(&v.to_string());
    }
    fn raw_f64(&mut self, v: f64) {
        self.out.push_str(&fmt_f64(v));
    }
    // object members (key + atom)
    fn m_str(&mut self, k: &str, v: &str) {
        self.key(k);
        self.raw_str(v);
    }
    fn m_usize(&mut self, k: &str, v: usize) {
        self.key(k);
        self.raw_usize(v);
    }
    fn m_f64(&mut self, k: &str, v: f64) {
        self.key(k);
        self.raw_f64(v);
    }
    fn m_bool(&mut self, k: &str, v: bool) {
        self.key(k);
        self.raw_bool(v);
    }
    fn f64_arr(&mut self, xs: &[f64]) {
        self.begin_arr();
        for &x in xs {
            self.comma();
            self.raw_f64(x);
        }
        self.end_arr();
    }
    fn u8_arr(&mut self, xs: &[u8]) {
        self.begin_arr();
        for &x in xs {
            self.comma();
            self.out.push_str(&x.to_string());
        }
        self.end_arr();
    }
    /// Array with a per-item writer. Each item is comma-separated; the closure
    /// writes exactly one value (atom or container).
    fn arr<T>(&mut self, items: &[T], mut each: impl FnMut(&mut Json, &T)) {
        self.begin_arr();
        for it in items {
            self.comma();
            each(self, it);
        }
        self.end_arr();
    }
}

fn fmt_f64(v: f64) -> String {
    if v == v.trunc() && v.is_finite() && v.abs() < 1e15 {
        // whole number: emit "N.0" so it is unambiguously a float (JSON-parses fine)
        format!("{:.1}", v)
    } else {
        format!("{v}")
    }
}

fn escape_into(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}
