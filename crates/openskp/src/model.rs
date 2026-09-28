//! The unified [`Model`]: everything the reader decodes from a `.skp`, plus a
//! deterministic JSON serializer (used by the CLI and the differential tests).
//!
//! Port of `parse_model` in `tools/skpwalk.py`.

use crate::ctx::Container;
use crate::settings::{
    Animation, Axes, Camera, Dimension, DimensionDefaults, Font, RenderingOptions, Scene,
    ShadowInfo, Style, Text, TextDefaults, Units, Watermark,
};
use crate::{
    extract, geometry_runs_with_diagnostics, header, AttrValue, Attribute, Definition, Diagnostic,
    GeometryRun, Guide, Image, Instance, Layer, Material, PlacedInstance, RunFilterReason,
    SectionPlane, Topology, INCH,
};

/// Everything decoded from a `.skp` file.
#[derive(Debug, Clone)]
pub struct Model {
    pub version: String,
    /// Which file layout the model came from.
    pub container: Container,
    /// Per-model identifier from the 2013–2017 header (SKP_FORMAT §3): kept
    /// across re-saves of the same model, distinct between models.
    pub model_guid: Option<String>,
    pub definitions: Vec<Definition>,
    pub instances: Vec<Instance>,
    pub geometry: Vec<GeometryRun>,
    pub materials: Vec<Material>,
    pub layers: Vec<Layer>,
    pub scenes: Vec<Scene>,
    pub guides: Vec<Guide>,
    pub attributes: Vec<Attribute>,
    pub images: Vec<Image>,
    /// The current view.
    pub camera: Option<Camera>,
    /// The document's display settings.
    pub rendering: Option<RenderingOptions>,
    pub shadows: Option<ShadowInfo>,
    pub units: Units,
    pub styles: Vec<Style>,
    /// Index into `styles` of the active style.
    pub active_style: Option<usize>,
    /// The watermarks of the current view (the active style's settings as
    /// last edited, saved or not).
    pub watermarks: Vec<Watermark>,
    pub fonts: Vec<Font>,
    pub texts: Vec<Text>,
    pub dimensions: Vec<Dimension>,
    /// The model axes.
    pub axes: Option<Axes>,
    pub text_defaults: Option<TextDefaults>,
    pub dimension_defaults: Option<DimensionDefaults>,
    /// Use anti-aliased textures (Model Info ▸ Rendering).
    pub anti_aliased_textures: Option<bool>,
    pub animation: Animation,
    /// The model has a geographic location set.
    pub geo_located: bool,
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
    /// The placing object's GLOBAL map slot (§4s), the identity a scene's
    /// hidden-entity list names; `None` on the legacy byte-scan path.
    pub slot: Option<usize>,
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
        // Container gate (Phase 0.1): the 2026 container has its
        // own reader; anything unrecognized is refused cleanly instead of
        // walked. `header_info` still identifies such files.
        match crate::ctx::detect_container(d) {
            crate::ctx::Container::Carchive2017 => {}
            crate::ctx::Container::Zip => {
                let (version, _) =
                    crate::header_info(d).ok_or_else(|| Error("malformed .skp header".into()))?;
                return crate::read26::parse(d, version);
            }
            crate::ctx::Container::Unknown => {
                return Err(Error(
                    "unsupported .skp container: neither the 2013–2017 MFC CArchive \
                     layout nor the 2026 container layout"
                        .into(),
                ))
            }
        }
        let hdr = header::parse_header(d).ok_or_else(|| Error("malformed .skp header".into()))?;
        match crate::walk2::walk(d) {
            Ok(cw) => {
                let end = cw.end;
                match Self::from_continuous(d, &hdr, cw) {
                    Some(m) => Ok(m),
                    None => {
                        let mut m = Self::parse_legacy(d, hdr)?;
                        m.diagnostics.push(Diagnostic::ContinuousFallback {
                            stage: "validation".into(),
                            at: end,
                            detail: "completed walk failed the defref cross-check".into(),
                        });
                        Ok(m)
                    }
                }
            }
            Err(f) if f.detail == crate::carchive::OUT_OF_MEMORY => Err(Error(format!(
                "out of memory while reading the model section at 0x{:x} ({} bytes in): \
                 the model is larger than this process can hold",
                f.at,
                d.len()
            ))),
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
        // byte-scan path never establishes — recorded rather than a silent
        // image loss.
        let (materials, shared_refs) = extract::materials(d);
        diagnostics.extend(unresolved_shared_texture_diags(shared_refs.len()));
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
        let attributes = extract::attributes(d);
        let units = Units::from_attributes(&attributes);
        let animation = Animation::from_attributes(&attributes);
        let geo_located = geo_located(&attributes);
        let s17 = crate::settings17::read(d, None, &[]);
        Ok(Model {
            container: Container::Carchive2017,
            version: hdr.version,
            model_guid: Some(hdr.model_guid),
            definitions,
            instances,
            geometry,
            materials,
            layers,
            scenes: s17.scenes,
            guides: extract::guides(d),
            attributes,
            images: extract::images(d),
            camera: s17.camera,
            rendering: s17.rendering,
            shadows: s17.shadows,
            units,
            styles: s17.styles,
            active_style: s17.active_style,
            watermarks: s17.watermarks,
            fonts: s17.fonts,
            texts: s17.texts,
            dimensions: s17.dimensions,
            axes: s17.axes,
            text_defaults: s17.text_defaults,
            dimension_defaults: s17.dimension_defaults,
            anti_aliased_textures: s17.anti_aliased_textures,
            animation,
            geo_located,
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
    /// Takes the walk by value: the global map (the parse's largest
    /// allocation — 64 bytes per object, tens of millions of objects on a
    /// production model) is released as soon as the meshes and instances
    /// are built, before the material extractor copies every texture.
    fn from_continuous(
        d: &[u8],
        hdr: &header::Header,
        mut cw: crate::walk2::Continuous,
    ) -> Option<Model> {
        use crate::carchive::Slot;
        use crate::entity::Entity;
        let owned_map = std::mem::take(&mut cw.map);
        let map = &owned_map;
        let cw = &cw;

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
            let Slot::Object(Entity::ComponentDef { meta, .. }) = &map[ds.slot] else {
                return None;
            };
            let (name, guid) = (&meta.name, &meta.guid);
            let (behaviour, timestamp) = (&meta.behaviour, &meta.timestamp);
            if let Ok(slot) = u32::try_from(ds.slot) {
                definition_links.push((slot, definitions.len()));
            }
            definitions.push(Definition {
                name: name.clone(),
                guid: guid.clone(),
                behaviour: behaviour.clone(),
                timestamp: *timestamp,
                map_index: Some(ds.slot),
            });
        }
        definition_links.sort_unstable();

        // One geometry run per definition (its subtree is a contiguous
        // global-map slot range) + one for the root entity list.
        // Guides come from the map, run by run (the byte scan only finds
        // the first CConstructionLine, the one carrying the class name).
        let guides17 = std::cell::RefCell::new(Vec::new());
        let run_of = |start: usize,
                      end: usize,
                      top_level: usize,
                      frame: Option<usize>,
                      def_index: Option<usize>,
                      lo: usize,
                      hi: usize|
         -> GeometryRun {
            let r5 = |x: f64| (x * 1e5).round() / 1e5;
            guides17.borrow_mut().extend(
                map[lo.min(map.len())..hi.min(map.len())]
                    .iter()
                    .filter_map(|s| match s {
                        Slot::Object(Entity::ConstructionLine { params, .. }) => Some(Guide {
                            def_index,
                            point_m: [
                                r5(params[0] / INCH),
                                r5(params[1] / INCH),
                                r5(params[2] / INCH),
                            ],
                            direction: [r5(params[3]), r5(params[4]), r5(params[5])],
                        }),
                        _ => None,
                    }),
            );
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
                    .enumerate()
                    .filter_map(|(k, s)| match s {
                        Slot::Object(Entity::InstancePlaced {
                            pid,
                            defref,
                            transform,
                            material,
                            hidden,
                            layer,
                            is_group,
                            ..
                        }) => Some(PlacedInstance {
                            pid: *pid,
                            slot: Some(lo + k),
                            defref: *defref,
                            transform: **transform,
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
                active_section: None, // set below for the root run (§4l); definition tails undecoded
                sections: map[lo.min(map.len())..hi.min(map.len())]
                    .iter()
                    .filter_map(|s| match s {
                        Slot::Object(Entity::SectionPlane { pid, plane, hidden }) => {
                            Some(SectionPlane {
                                pid: *pid,
                                plane: *plane,
                                hidden: *hidden,
                            })
                        }
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
        let mut root = run_of(
            cw.root_list_at,
            cw.root_list_at,
            cw.roots.len(),
            Some(cw.roots.len()),
            None,
            cw.root_slot_floor,
            map.len(),
        );
        // §4l: the root tail's first pointer names the root list's active
        // section plane by map slot; a slot that is not one of the root
        // list's own planes is not an active plane.
        root.active_section = cw
            .root_active_slot
            .and_then(|slot| match map.get(slot) {
                Some(Slot::Object(Entity::SectionPlane { pid, .. })) => Some(*pid),
                _ => None,
            })
            .filter(|pid| root.sections.iter().any(|s| s.pid == *pid));
        geometry.push(root);

        // Instances: every CGroup/CComponentInstance in the map, in
        // serialization order, linked by the def-ref = global slot.
        let round5 = |x: f64| (x * 1e5).round() / 1e5;
        let mut instances = Vec::new();
        for (slot, entry) in map.iter().enumerate() {
            if let Slot::Object(Entity::InstancePlaced {
                pid,
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
                    Some(Slot::Object(Entity::ComponentDef { meta, .. })) => {
                        Some(meta.name.clone())
                    }
                    _ => None,
                };
                instances.push(Instance {
                    pid: *pid,
                    slot: Some(slot),
                    definition,
                    defref: *defref,
                    offset: *tf_at,
                    translation_m: [
                        round5(transform[9] / INCH),
                        round5(transform[10] / INCH),
                        round5(transform[11] / INCH),
                    ],
                    transform: **transform,
                    name: Some(name.clone()),
                    is_group: Some(*is_group),
                    material: Some(*material),
                    hidden: Some(*hidden),
                    layer: Some(*layer),
                });
            }
        }

        let s17 = crate::settings17::read(d, Some(map), &cw.layer_slots);
        // Everything below reads the file, not the map: free it first.
        drop(owned_map);
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

        let attributes = extract::attributes(d);
        let units = Units::from_attributes(&attributes);
        let animation = Animation::from_attributes(&attributes);
        let geo_located = geo_located(&attributes);
        Some(Model {
            container: Container::Carchive2017,
            version: hdr.version.clone(),
            model_guid: Some(hdr.model_guid.clone()),
            definitions,
            instances,
            geometry,
            materials,
            layers,
            scenes: s17.scenes,
            guides: guides17.into_inner(),
            attributes,
            images: extract::images(d),
            camera: s17.camera,
            rendering: s17.rendering,
            shadows: s17.shadows,
            units,
            styles: s17.styles,
            active_style: s17.active_style,
            watermarks: s17.watermarks,
            fonts: s17.fonts,
            texts: s17.texts,
            dimensions: s17.dimensions,
            axes: s17.axes,
            text_defaults: s17.text_defaults,
            dimension_defaults: s17.dimension_defaults,
            anti_aliased_textures: s17.anti_aliased_textures,
            animation,
            geo_located,
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
                    i.slot,
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
        slot: Option<usize>,
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
                            p.slot,
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
            slot,
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
        j.m_str(
            "container",
            match self.container {
                Container::Carchive2017 => "carchive2017",
                Container::Zip => "zip",
                Container::Unknown => "unknown",
            },
        );
        j.key("model_guid");
        match &self.model_guid {
            Some(g) => j.raw_str(g),
            None => j.raw_null(),
        }

        j.key("definitions");
        j.arr(&self.definitions, |j, def| {
            j.begin_obj();
            j.m_str("name", &def.name);
            j.m_str("guid", &def.guid);
            j.m_usize("timestamp", def.timestamp as usize);
            j.key("behaviour");
            behaviour_json(j, &def.behaviour);
            j.end_obj();
        });

        j.key("instances");
        j.arr(&self.instances, |j, inst| {
            j.begin_obj();
            j.key("pid");
            j.raw_usize(inst.pid as usize);
            j.key("definition");
            match &inst.definition {
                Some(s) => j.raw_str(s),
                None => j.raw_null(),
            }
            j.key("translation_m");
            j.f64_arr(&inst.translation_m);
            j.key("is_group");
            match inst.is_group {
                Some(b) => j.raw_bool(b),
                None => j.raw_null(),
            }
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
                // (the Python reference never decoded textured opacity, and
                // tools/skpwalk.py is frozen — never a place for new
                // capability — so this field can never join the comparison).
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
        j.arr(&self.scenes, scene_json);

        j.key("guides");
        j.arr(&self.guides, |j, g: &Guide| {
            j.begin_obj();
            j.key("def_index");
            opt_index(j, g.def_index);
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

        j.key("camera");
        opt(&mut j, self.camera.as_ref(), camera_json);
        j.key("rendering");
        opt(&mut j, self.rendering.as_ref(), rendering_json);
        j.key("shadows");
        opt(&mut j, self.shadows.as_ref(), shadow_json);
        j.key("units");
        units_json(&mut j, &self.units);
        j.key("styles");
        j.arr(&self.styles, style_json);
        j.key("active_style");
        match self.active_style {
            Some(i) => j.raw_usize(i),
            None => j.raw_null(),
        }
        j.key("watermarks");
        j.arr(&self.watermarks, watermark_json);
        j.key("fonts");
        j.arr(&self.fonts, |j, f| {
            j.begin_obj();
            j.m_str("family", &f.family);
            j.m_bool("bold", f.bold);
            j.m_bool("italic", f.italic);
            j.m_usize("size_pt", f.size_pt as usize);
            j.end_obj();
        });
        j.key("texts");
        j.arr(&self.texts, text_json);
        j.key("dimensions");
        j.arr(&self.dimensions, dimension_json);
        j.key("axes");
        opt(&mut j, self.axes.as_ref(), axes_json);
        j.key("text_defaults");
        opt(&mut j, self.text_defaults.as_ref(), |j, t| {
            j.begin_obj();
            j.key("font");
            opt_index(j, t.font);
            j.key("screen_font");
            opt_index(j, t.screen_font);
            j.m_usize("arrow", t.arrow as usize);
            j.key("leader_text_color");
            j.u8_arr(&t.leader_text_color);
            j.key("screen_text_color");
            j.u8_arr(&t.screen_text_color);
            j.end_obj();
        });
        j.key("dimension_defaults");
        opt(&mut j, self.dimension_defaults.as_ref(), |j, d| {
            j.begin_obj();
            j.key("font");
            opt_index(j, d.font);
            j.m_bool("aligned", d.aligned);
            j.m_usize("arrow", d.arrow as usize);
            j.m_usize("text_position", d.text_position as usize);
            j.key("color");
            j.u8_arr(&d.color);
            j.end_obj();
        });
        j.key("anti_aliased_textures");
        match self.anti_aliased_textures {
            Some(b) => j.raw_bool(b),
            None => j.raw_null(),
        }
        j.key("animation");
        j.begin_obj();
        j.m_bool("transitions", self.animation.transitions);
        j.m_f64("transition_s", self.animation.transition_s);
        j.m_f64("delay_s", self.animation.delay_s);
        j.m_bool("loop_slideshow", self.animation.loop_slideshow);
        j.end_obj();
        j.m_bool("geo_located", self.geo_located);

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
                    Diagnostic::UnresolvedSharedTexture { count } => {
                        // Informational; excluded by the is_desync filter
                        // above — serialized here only for match completeness.
                        j.m_str("kind", "unresolved_shared_texture");
                        j.m_usize("count", *count);
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
                j.key("is_group");
                match n.is_group {
                    Some(b) => j.raw_bool(b),
                    None => j.raw_null(),
                }
                j.end_obj();
            }
            for c in &n.children {
                rec(j, c);
            }
        }
        for n in self.scene() {
            rec(&mut j, &n);
        }
        // the root run (loose model-level geometry) at identity — no
        // instance places it, so group-vs-component identity doesn't apply.
        if let Some(root) = self.geometry.iter().position(|r| r.def_index.is_none()) {
            j.comma();
            j.begin_obj();
            j.m_usize("run", root);
            j.key("world");
            j.f64_arr(&IDENTITY);
            j.m_usize("material", 0);
            j.key("is_group");
            j.raw_null();
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
    // Exact-exact path first: the archive walk yields ABSOLUTE slots and
    // needs no anchoring — only the check that every face ref lands on a
    // material. Then the byte-scan walk with its unique-shift anchor.
    let archive = crate::matwalk::walk_archive(d, base).and_then(|slots| {
        let set: std::collections::BTreeSet<usize> = slots.iter().map(|s| s.rel).collect();
        refs.iter().all(|&r| set.contains(&(r as usize))).then(|| {
            let links: Vec<(u16, usize)> = slots
                .iter()
                .enumerate()
                .filter_map(|(i, s)| u16::try_from(s.rel).ok().map(|r| (r, i)))
                .collect();
            (slots, links)
        })
    });
    let walked = archive.or_else(|| {
        let slots = crate::matwalk::walk_region(d)?;
        let links = crate::matwalk::links_from_walk(&slots, &refs, base + 1)?;
        Some((slots, links))
    });
    if let Some((slots, links)) = walked {
        {
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
                let unresolved = resolve_shared_bytes(materials, shared_refs, |dib_slot| {
                    slots
                        .iter()
                        .position(|s| s.dib_rel.is_some_and(|r| r + shift == dib_slot as usize))
                        .and_then(|wi| by_walk[wi])
                });
                return (mapped, unresolved_shared_texture_diags(unresolved));
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
            let unresolved = resolve_shared_bytes(materials, shared_refs, |dib_slot| {
                links.iter().find_map(|&(s, i)| {
                    (own_dib[i] && s as usize + 1 == dib_slot as usize).then_some(i)
                })
            });
            (links, unresolved_shared_texture_diags(unresolved))
        }
        None => {
            let mut diags = vec![Diagnostic::MaterialLinkFallback {
                declared: declared.unwrap_or(0),
                extracted: materials.len(),
            }];
            // The suffix fallback places no slots (model.rs doc comment
            // above), so every shared-texture back-ref has nothing to
            // resolve against — recorded rather than a silent image loss.
            diags.extend(unresolved_shared_texture_diags(shared_refs.len()));
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
            (links, diags)
        }
    }
}

/// Zero or one [`Diagnostic::UnresolvedSharedTexture`], the count-to-diag
/// step every call site (all three linking tiers) applies identically.
fn unresolved_shared_texture_diags(unresolved: usize) -> Vec<Diagnostic> {
    if unresolved > 0 {
        vec![Diagnostic::UnresolvedSharedTexture { count: unresolved }]
    } else {
        Vec::new()
    }
}

/// Resolve every shared-texture back-ref to its owning material and copy
/// the bytes over, given a model-specific `dib_slot -> owning material
/// index` lookup. The lookup differs between the exact matwalk anchor and
/// the arithmetic-fallback anchor (model.rs above); this is the one place
/// that turns a resolved owner into an actual byte copy, so a future
/// change to when/how bytes get adopted only needs to land here once.
/// Returns how many entries `owner_of` could not resolve at all — the
/// caller records that count rather than letting the loss stay silent
/// (mirrors the suffix-fallback tier, which has no owner lookup to try).
fn resolve_shared_bytes(
    materials: &mut [Material],
    shared_refs: &[(usize, u16)],
    mut owner_of: impl FnMut(u16) -> Option<usize>,
) -> usize {
    let mut unresolved = 0;
    for &(mi, dib_slot) in shared_refs {
        match owner_of(dib_slot) {
            Some(owner) => adopt_shared_bytes(materials, mi, owner),
            None => unresolved += 1,
        }
    }
    unresolved
}

/// Copy the owning material's embedded image onto a shared-texture
/// material (§8.1: its back-ref names the owner's CDib slot). A no-op
/// unless the owner actually carries inline bytes.
///
/// `shared == owner` and either index being out of range never trigger
/// given today's two call sites (`shared` is always a `shared_refs` index,
/// valid by construction in `extract::materials`; `owner` only ever comes
/// from a lookup already bounded to `materials`, and only ever resolves to
/// a material with its own inline dib, which a shared-texture material
/// never has) — kept as cheap defense-in-depth against a malformed file
/// reaching this differently in the future, not as reachable logic today.
fn adopt_shared_bytes(materials: &mut [Material], shared: usize, owner: usize) {
    if shared == owner || shared >= materials.len() || owner >= materials.len() {
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

fn geo_located(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.key == "UsesGeoReferencing"
            && matches!(a.value, AttrValue::Bool(true) | AttrValue::Int(1..))
    })
}

fn opt_index(j: &mut Json, i: Option<usize>) {
    match i {
        Some(i) => j.raw_usize(i),
        None => j.raw_null(),
    }
}

fn axes_json(j: &mut Json, a: &Axes) {
    j.begin_obj();
    j.key("origin_m");
    j.f64_arr(&a.origin_m);
    j.key("x");
    j.f64_arr(&a.x);
    j.key("y");
    j.f64_arr(&a.y);
    j.key("z");
    j.f64_arr(&a.z);
    j.end_obj();
}

fn opt<T>(j: &mut Json, v: Option<&T>, f: impl Fn(&mut Json, &T)) {
    match v {
        Some(x) => f(j, x),
        None => j.raw_null(),
    }
}

fn camera_json(j: &mut Json, c: &Camera) {
    j.begin_obj();
    j.key("eye_m");
    j.f64_arr(&c.eye_m);
    j.key("target_m");
    j.f64_arr(&c.target_m);
    j.key("up");
    j.f64_arr(&c.up);
    j.m_bool("perspective", c.perspective);
    j.m_f64("fov_deg", c.fov_deg);
    j.m_f64("parallel_height_m", c.parallel_height_m);
    j.m_bool("two_point_perspective", c.two_point_perspective);
    j.m_str("description", &c.description);
    j.end_obj();
}

fn rendering_json(j: &mut Json, ro: &RenderingOptions) {
    use crate::settings::RoValue;
    j.begin_obj();
    for (name, v) in ro.values() {
        match v {
            Some(RoValue::Bool(b)) => j.m_bool(name, b),
            Some(RoValue::U32(x)) => j.m_usize(name, x as usize),
            Some(RoValue::F64(x)) => j.m_f64(name, x),
            Some(RoValue::Rgba(c)) => {
                j.key(name);
                j.u8_arr(&c);
            }
            None => {
                j.key(name);
                j.raw_null();
            }
        }
    }
    j.end_obj();
}

fn shadow_json(j: &mut Json, s: &ShadowInfo) {
    j.begin_obj();
    j.m_usize("time", s.time as usize);
    j.m_str("city", &s.city);
    j.m_str("country", &s.country);
    j.m_f64("longitude", s.longitude);
    j.m_f64("latitude", s.latitude);
    j.m_f64("tz_offset_h", s.tz_offset_h);
    j.m_bool("displayed", s.displayed);
    j.m_bool("on_faces", s.on_faces);
    j.m_bool("on_ground", s.on_ground);
    j.m_bool("from_edges", s.from_edges);
    j.m_usize("light", s.light as usize);
    j.m_usize("dark", s.dark as usize);
    j.m_bool("use_sun_for_shading", s.use_sun_for_shading);
    j.end_obj();
}

fn units_json(j: &mut Json, u: &Units) {
    j.begin_obj();
    j.m_usize("length_format", u.length_format as usize);
    j.m_usize("length_unit", u.length_unit as usize);
    j.m_usize("length_precision", u.length_precision as usize);
    j.m_usize("angle_precision", u.angle_precision as usize);
    j.m_bool("length_snap", u.length_snap);
    j.m_f64("length_snap_length", u.length_snap_length);
    j.m_bool("angle_snap", u.angle_snap);
    j.m_f64("snap_angle", u.snap_angle);
    j.m_bool("suppress_units_display", u.suppress_units_display);
    j.m_bool("force_inch_display", u.force_inch_display);
    for (k, v) in [
        ("area_unit", u.area_unit),
        ("volume_unit", u.volume_unit),
        ("area_precision", u.area_precision),
        ("volume_precision", u.volume_precision),
    ] {
        j.key(k);
        match v {
            Some(x) => j.raw_usize(x as usize),
            None => j.raw_null(),
        }
    }
    j.end_obj();
}

fn style_json(j: &mut Json, s: &Style) {
    j.begin_obj();
    j.m_str("name", &s.name);
    j.m_str("description", &s.description);
    j.m_str("guid", &s.guid);
    j.key("settings");
    rendering_json(j, &s.settings);
    j.key("watermarks");
    j.arr(&s.watermarks, watermark_json);
    j.end_obj();
}

fn watermark_json(j: &mut Json, w: &Watermark) {
    j.begin_obj();
    j.m_str("name", &w.name);
    j.m_str("source_file", &w.source_file);
    j.m_bool("background", w.background);
    j.m_bool("tiled", w.tiled);
    j.m_bool("stretched", w.stretched);
    j.m_bool("keep_aspect_ratio", w.keep_aspect_ratio);
    j.m_usize("position", w.position as usize);
    j.m_bool("mask", w.mask);
    j.m_f64("blend", w.blend);
    j.m_f64("scale", w.scale);
    j.end_obj();
}

fn scene_json(j: &mut Json, s: &Scene) {
    j.begin_obj();
    j.m_str("name", &s.name);
    j.m_str("description", &s.description);
    j.m_usize("saved", s.saved.0 as usize);
    j.key("camera");
    opt(j, s.camera.as_ref(), camera_json);
    j.key("rendering");
    opt(j, s.rendering.as_ref(), rendering_json);
    j.key("style");
    match &s.style {
        Some(n) => j.raw_str(n),
        None => j.raw_null(),
    }
    j.key("shadows");
    opt(j, s.shadows.as_ref(), shadow_json);
    j.key("axes");
    opt(j, s.axes.as_ref(), axes_json);
    j.key("hidden_entities");
    j.arr(&s.hidden_entities, |j, id| j.raw_usize(*id as usize));
    j.key("active_section_planes");
    j.arr(&s.active_section_planes, |j, id| j.raw_usize(*id as usize));
    j.key("hidden_layers");
    j.arr(&s.hidden_layers, |j, i| j.raw_usize(*i));
    j.m_bool("in_animation", s.in_animation);
    j.end_obj();
}

fn anchor_json(j: &mut Json, a: &crate::settings::Anchor) {
    j.begin_obj();
    j.m_usize("kind", a.kind as usize);
    j.key("point_m");
    j.f64_arr(&a.point_m);
    j.key("entity");
    opt_index(j, a.entity.map(|e| e as usize));
    j.end_obj();
}

fn text_json(j: &mut Json, t: &Text) {
    j.begin_obj();
    j.m_usize("pid", t.pid as usize);
    j.m_str("content", &t.content);
    j.key("screen_position");
    j.f64_arr(&t.screen_position);
    j.key("anchor");
    anchor_json(j, &t.anchor);
    j.key("leader_offset");
    j.f64_arr(&t.leader_offset);
    j.m_str(
        "leader",
        match t.leader {
            crate::settings::Leader::None => "none",
            crate::settings::Leader::ViewBased => "view_based",
            crate::settings::Leader::Pushpin => "pushpin",
        },
    );
    j.m_usize("arrow", t.arrow as usize);
    j.key("font");
    match t.font {
        Some(i) => j.raw_usize(i),
        None => j.raw_null(),
    }
    j.end_obj();
}

fn dimension_json(j: &mut Json, d: &Dimension) {
    j.begin_obj();
    j.m_usize("pid", d.pid as usize);
    j.m_str("text_override", &d.text_override);
    j.key("start");
    anchor_json(j, &d.start);
    j.key("end");
    anchor_json(j, &d.end);
    j.key("normal");
    j.f64_arr(&d.normal);
    j.key("x_axis");
    j.f64_arr(&d.x_axis);
    j.m_f64("offset_m", d.offset_m);
    j.m_usize("text_position", d.text_position as usize);
    j.m_bool("aligned", d.aligned);
    j.key("font");
    match d.font {
        Some(i) => j.raw_usize(i),
        None => j.raw_null(),
    }
    j.end_obj();
}

fn behaviour_json(j: &mut Json, b: &crate::settings::Behaviour) {
    j.begin_obj();
    j.m_bool("glues_to_surface", b.glues_to_surface);
    j.m_usize("glue_plane", b.glue_plane as usize);
    j.m_bool("cuts_opening", b.cuts_opening);
    j.m_bool("always_faces_camera", b.always_faces_camera);
    j.m_bool("shadows_face_sun", b.shadows_face_sun);
    j.end_obj();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn corpus(name: &str) -> Vec<u8> {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("../../corpus/2017");
        p.push(name);
        std::fs::read(p).unwrap()
    }

    fn empty_topology() -> Topology {
        Topology {
            vertices: 0,
            edges: 0,
            faces: 0,
            loops: 0,
            edge_uses: 0,
            curves: 0,
            dimensions: 0,
            texts: 0,
            section_planes: 0,
            images_placed: 0,
            construction_points: 0,
        }
    }

    /// Both linking tiers (§4s exact matwalk, §4s slot arithmetic) fail
    /// when `d` has no CMaterial region at all and `materials` is empty
    /// while a face still references a matref — landing in the suffix-zip
    /// fallback. With a nonempty `shared_refs`, the fallback must flag the
    /// unresolved shared-texture loss rather than stay silent about it.
    #[test]
    fn suffix_zip_fallback_flags_unresolved_shared_textures() {
        let geometry = vec![GeometryRun {
            start: 0,
            end: 0,
            top_level: 0,
            frame: None,
            def_index: None,
            topology: empty_topology(),
            resolved: (0, 0),
            mesh: crate::mesh::Mesh {
                vertices: vec![],
                faces: vec![crate::mesh::MeshFace {
                    pid: 0,
                    outer: vec![],
                    holes: vec![],
                    normal: [0.0, 0.0, 1.0],
                    front_material: Some(5),
                    back_material: None,
                    hidden: false,
                    layer: 0,
                    texture: None,
                }],
                edges: vec![],
            },
            placed: vec![],
            curve_members: vec![],
            sections: vec![],
            active_section: None,
        }];
        let mut materials: Vec<Material> = Vec::new();
        let shared_refs = vec![(0usize, 7u16)];
        let (links, diags) =
            continuous_material_links(&[], &mut materials, &shared_refs, 10, &geometry);
        assert!(
            links.is_empty(),
            "no material exists to link the observed matref to"
        );
        assert!(
            diags
                .iter()
                .any(|d| matches!(d, Diagnostic::MaterialLinkFallback { .. })),
            "must record which fallback tier ran: {diags:?}"
        );
        assert!(
            diags
                .iter()
                .any(|d| matches!(d, Diagnostic::UnresolvedSharedTexture { count } if *count == 1)),
            "the shared-texture loss must be flagged, not silent: {diags:?}"
        );
    }

    /// house.skp's "[Wood Floor Light]1" is a real shared-texture material;
    /// the legacy byte-scan path can extract it (materials are pure
    /// byte-scan, independent of which path walked the geometry) but can
    /// never place its back-ref globally, so its image bytes stay
    /// unresolved — the loss must be flagged, not silent.
    #[test]
    fn legacy_path_flags_unresolved_shared_textures() {
        let d = corpus("house.skp");
        let hdr = header::parse_header(&d).expect("house.skp header");
        let m = Model::parse_legacy(&d, hdr).expect("legacy parse");
        assert!(
            m.materials.iter().any(
                |mat| matches!(mat, Material::Textured { name, .. } if name.contains("Wood Floor Light"))
            ),
            "the shared-texture material must still be extracted by name"
        );
        assert!(
            m.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::UnresolvedSharedTexture { count } if *count > 0)),
            "the legacy path's shared-texture loss must be flagged, not silent: {:?}",
            m.diagnostics
        );
    }

    /// The arithmetic tier can place every observed face matref correctly
    /// (an overall SUCCESS) while one shared-texture back-ref still names a
    /// slot no material owns — a per-entry resolution failure distinct from
    /// "the whole tier failed". That partial loss must also be flagged.
    #[test]
    fn arithmetic_success_still_flags_a_partial_unresolved_shared_texture() {
        let geometry = vec![GeometryRun {
            start: 0,
            end: 0,
            top_level: 0,
            frame: None,
            def_index: None,
            topology: empty_topology(),
            resolved: (0, 0),
            mesh: crate::mesh::Mesh {
                vertices: vec![],
                faces: vec![crate::mesh::MeshFace {
                    pid: 0,
                    outer: vec![],
                    holes: vec![],
                    normal: [0.0, 0.0, 1.0],
                    front_material: Some(1),
                    back_material: None,
                    hidden: false,
                    layer: 0,
                    texture: None,
                }],
                edges: vec![],
            },
            placed: vec![],
            curve_members: vec![],
            sections: vec![],
            active_section: None,
        }];
        // One solid material (no inline dib) placed at slot 1 by the
        // arithmetic model (base 1 -> first = (1+1) - (1+0) = 1); the
        // observed matref {1} lands on it, so arithmetic() succeeds.
        let mut materials = vec![Material::Solid {
            name: "M".into(),
            rgba: [0, 0, 0, 0],
            opacity: 1.0,
        }];
        // A shared-texture back-ref naming a dib slot no material owns
        // (the sole material has no inline dib to begin with).
        let shared_refs = vec![(0usize, 99u16)];
        let (links, diags) =
            continuous_material_links(&[], &mut materials, &shared_refs, 1, &geometry);
        assert_eq!(
            links,
            vec![(1u16, 0usize)],
            "the arithmetic tier must still succeed"
        );
        assert!(
            diags.iter().any(
                |d| matches!(d, Diagnostic::UnresolvedSharedTexture { count } if *count == 1)
            ),
            "a per-entry resolution failure inside a successful tier must also be flagged: {diags:?}"
        );
    }
}
