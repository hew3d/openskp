//! Clean-room reader for the SketchUp `.skp` binary format (SketchUp 2017,
//! v17.3.116 and nearby). Derived solely from observed `.skp` files and their
//! COLLADA exports — no Trimble SDK. See `docs/SKP_FORMAT.md` for the format
//! and `docs/SDK.md` for how to use this crate.

mod carchive;
pub mod ctx;
mod entity;
mod extract;
mod header;
mod matwalk;
mod mesh;
mod model;
mod resolve;
mod walk;
mod walk2;

pub use carchive::{mfc_strlen, Stall};
pub use ctx::{detect_container, Container, Ctx};
pub use mesh::{FaceTexture, Mesh, MeshEdge, MeshFace, Side, UvXform};
pub use model::{Error, Model, Node};

/// Header-only identification: `(version, format_guid)` — works even when the
/// container is unreadable past the fixed header (e.g. post-2017 files), so
/// tooling can always say WHAT a file is before refusing it.
pub fn header_info(d: &[u8]) -> Option<(String, String)> {
    header::parse_header(d).map(|h| (h.version, h.format_guid))
}

/// 1 metre in inches; `.skp` stores coordinates as f64 inches.
pub const INCH: f64 = 39.37007874015748;

/// A component or group definition: `{ name, guid }`.
#[derive(Debug, Clone, PartialEq)]
pub struct Definition {
    pub name: String,
    pub guid: String,
    /// The definition object's GLOBAL archive map slot (SKP_FORMAT §4s) —
    /// exactly what instance def-refs carry. `Some` on the continuous
    /// path; `None` on the legacy byte-scan path (which links positionally
    /// through `Model::definition_links` instead).
    pub map_index: Option<usize>,
}

/// A placed component/group instance.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    /// Name of the definition this instance places (if resolvable).
    pub definition: Option<String>,
    /// The referenced definition's declared map index (§4h; on the
    /// continuous path the definition object's GLOBAL map slot, §4s —
    /// u32: big maps pass 0x7FFF slots via the MFC big-tag escape, §4t).
    pub defref: u32,
    /// File offset of the instance's transform block — used to tell
    /// top-level instances (scene roots) from in-definition children.
    pub offset: usize,
    /// Translation in metres (row 9..11 of the transform, ÷ INCH).
    pub translation_m: [f64; 3],
    /// Full 13-f64 transform: 3×3 row-major rotation/scale + translation + w.
    pub transform: [f64; 13],
    /// Drawbase matref (§4q) — material painted on the instance, inherited
    /// by default-material faces below it. `None` on the byte-scan path
    /// (the legacy transform-anchored scan never sees the drawbase).
    pub material: Option<u16>,
    /// The instance's own name ("Slab", "Front Wall", …). `None` on the
    /// legacy byte-scan path, which cannot see it.
    pub name: Option<String>,
    /// `Some(true)` = a `CGroup`, `Some(false)` = a `CComponentInstance`
    /// (§4s: identical layouts, distinct classes). `None` on the legacy
    /// path, which cannot tell them apart.
    pub is_group: Option<bool>,
    /// §4q drawbase hidden flag; `None` on the legacy byte-scan path.
    pub hidden: Option<bool>,
    /// §4q drawbase layer slot; `None` on the legacy byte-scan path.
    pub layer: Option<u16>,
}

/// A material. `.skp` distinguishes solid from textured by on-disk shape.
#[derive(Debug, Clone, PartialEq)]
pub enum Material {
    Solid {
        name: String,
        rgba: [u8; 4],
        opacity: f64,
    },
    Textured {
        name: String,
        texture: Option<String>,
        applied_size_in: Option<(f64, f64)>,
        /// The material's embedded image, byte-for-byte (Phase 3.5): the
        /// inline CDib payload (PNG or JPEG).
        image_bytes: Option<Vec<u8>>,
        /// Average texture colour (§4v: the RGBA right after the texture
        /// filename) — what `.dae` exports emit as the material's diffuse.
        /// API-level only, like `image_bytes` (the frozen-oracle JSON
        /// predates it).
        avg_rgba: Option<[u8; 4]>,
        /// Effective opacity 0..1 — same stored-slider semantics as
        /// solids: the record's f64 applies only when the use-opacity
        /// flag byte is set, else 1.0 (house.skp's "[Translucent Glass
        /// Tinted]" stores 0.52 flag-on, the value its `.dae` export
        /// carries). API-level only, like `image_bytes`.
        opacity: f64,
    },
}

/// A model layer: name, visibility, and display colour.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    pub rgba: [u8; 4],
}

/// A construction line (guide): a point + unit direction, point in metres.
#[derive(Debug, Clone, PartialEq)]
pub struct Guide {
    pub point_m: [f64; 3],
    pub direction: [f64; 3],
}

/// A typed attribute-dictionary value.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    Int(i32),
    F64(f64),
    Bool(bool),
}

/// One attribute key/value (model options, geo-location, dynamic-component params).
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub key: String,
    pub value: AttrValue,
}

/// An embedded image (`CDib`): thumbnail or material texture.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// "png" or "jpg".
    pub kind: String,
    /// Declared image length in bytes.
    pub bytes: usize,
}

/// An in-list component/group instance placement (Phase 3.3): which
/// definition it places (by declared map index — match against
/// `GeometryRun::def_index`) and its 13-f64 transform.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedInstance {
    pub defref: u32,
    pub transform: [f64; 13],
    /// `true` = a `CGroup` placement, `false` = a `CComponentInstance`
    /// (§4s: identical layouts, distinct classes — the class carries the
    /// group-vs-component identity, exactly as on [`Instance`]).
    pub is_group: bool,
    /// Drawbase matref (§4q): material painted on the instance itself,
    /// inherited by default-material faces in its subtree (0 = none).
    pub material: u16,
    /// §4q drawbase hidden flag — a hidden instance hides its subtree
    /// (exports are WYSIWYG: hidden subtrees are dropped).
    pub hidden: bool,
    /// §4q drawbase layer slot (0 = default layer); an instance on a
    /// hidden layer is not displayed/exported.
    pub layer: u16,
}

/// Resolved concrete topology of one geometry run (a box → 8/12/6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topology {
    pub vertices: usize,
    pub edges: usize,
    pub faces: usize,
    pub loops: usize,
    pub edge_uses: usize,
    pub curves: usize,
    /// Non-mesh annotation entities in the run (Phase 6.2: house-plus's
    /// extras surface as counts, never as geometry): linear dimensions,
    /// texts, section planes, placed images, construction points.
    pub dimensions: usize,
    pub texts: usize,
    pub section_planes: usize,
    pub images_placed: usize,
    pub construction_points: usize,
}

/// One user-geometry run (a top-level drawing, or one component definition).
#[derive(Debug, Clone)]
pub struct GeometryRun {
    /// File offset where the run's entity stream begins.
    pub start: usize,
    /// File offset just past the last byte the walk consumed.
    pub end: usize,
    /// Top-level entity-list elements read (edges/faces/instances — inline
    /// children like a CEdge's vertices are pool objects, not list elements).
    pub top_level: usize,
    /// The list's declared element count (`u32` immediately before the list,
    /// Phase 1.1), when plausible. `top_level == frame` means the walk
    /// consumed the list EXACTLY as framed.
    pub frame: Option<usize>,
    /// The owning object's archive map index as DECLARED by the pre-list
    /// prelude (Phase 1.2). For a component definition's run this equals the
    /// def-ref u16 its instances carry — the structural instance→definition
    /// link.
    pub def_index: Option<usize>,
    /// Resolved concrete counts (from the object pool).
    pub topology: Topology,
    /// (satisfied, total) back-reference constraints.
    pub resolved: (usize, usize),
    /// The materialized mesh (Phase 3.1): vertices in metres, faces as
    /// oriented rings, edges with flags.
    pub mesh: Mesh,
    /// In-list child instances (Phase 3.3): the run's definition places
    /// these children — the hierarchy's edges.
    pub placed: Vec<PlacedInstance>,
    /// Welded/freehand `CCurve` records in the run (§4t): the declared
    /// member-edge count of each, serialization order. Continuous-path
    /// only (the legacy walk never decodes CCurve bodies).
    pub curve_members: Vec<u32>,
}

/// A recorded parse anomaly (Phase 0.4). Clean 2013–2017 files parse with
/// **zero desync diagnostics** (`Resync`/`Truncated`/`Skipped` — a standing
/// regression test); `RunFiltered` is also recorded but fires by design on
/// some clean files (thumbnail false-positive runs, dynamic-component
/// fragments), so it is informational, not a desync signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diagnostic {
    /// The walk stalled mid-run and recovered at a later entity header.
    Resync {
        at: usize,
        resumed_at: usize,
        class: String,
    },
    /// The stream ended while a run was still being walked.
    Truncated { at: usize },
    /// An undecodable-but-delimited class body was skipped (Phase 0.3 rule).
    Skipped {
        class: String,
        at: usize,
        bytes: usize,
    },
    /// A candidate run was dropped by the reference filters — recorded, never
    /// silent (Phase 0.4).
    RunFiltered {
        start: usize,
        reason: RunFilterReason,
    },
    /// No plausible entity-list count u32 preceded this run (Phase 1.1) —
    /// the walk fell back to the gap/stall heuristics for its framing.
    /// Informational: junk candidate runs lack frames by nature.
    HeuristicFraming { at: usize },
    /// No structural entity-list candidate was found (Phase 1.3: the
    /// definition-list prelude and root-list "IEND" signatures both
    /// missing), so the walk anchored on the densest-cluster heuristic at
    /// this offset. Informational: the fallback is the long-validated
    /// pre-1.3 behavior.
    HeuristicAnchor { at: usize },
    /// Instance→definition linkage could not be established structurally
    /// (declared map indexes + name-record bracketing, Phase 1.2) and fell
    /// back to the sorted-defref ↔ file-order zip. Informational: the
    /// fallback is the previously validated behavior, but it is positional,
    /// not structural.
    HeuristicLinkage { definitions: usize },
    /// The CONTINUOUS single-archive walk (SKP_FORMAT §4s) parsed this file:
    /// `base` pre-model slots were calibrated, and the model section walked
    /// with zero desyncs (a continuous attempt either completes exactly or
    /// is abandoned). Informational — the path marker.
    ContinuousWalk { base: usize },
    /// The continuous walk was attempted and died at `at` during `stage`
    /// (anchor/calibration/layer-list/definitions/trailing-defs/root-list/
    /// validation); the legacy run-based path served the file instead.
    /// Informational: the fallback IS the long-validated behavior.
    ContinuousFallback {
        stage: String,
        at: usize,
        detail: String,
    },
    /// A pre-model pad slot was bound to a class by its read-site
    /// expectation during the continuous walk (§4s context-directed slot
    /// binding: thumbnail children → CCamera/CDib, entity attribute
    /// pointers → CAttributeContainer, container children →
    /// CAttributeNamed). Informational: calibration evidence, never silent.
    PadSlotBound { slot: usize, class: String },
    /// The §4s material slot arithmetic (first slot = base − (M + D) + 1,
    /// dibs interleaved after their material) disagreed with the observed
    /// face matrefs or the manager's declared count, so face→material
    /// linkage fell back to the documented §4n suffix alignment.
    MaterialLinkFallback { declared: usize, extracted: usize },
    /// `count` shared-texture materials' back-refs (§8.1) could not be
    /// resolved to their owning material's image bytes and keep
    /// `image_bytes: None` — the anchor an owner lookup needs was missing
    /// (the §4n suffix fallback placed no slots, or the file took the
    /// legacy byte-scan path, which never establishes the §4s global
    /// anchor) or a specific owner lookup came up empty. Informational: the
    /// loss is recorded on `Model::diagnostics`, like every other fallback
    /// here, rather than left silent.
    UnresolvedSharedTexture { count: usize },
}

/// Why a candidate run was filtered out of [`geometry_runs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFilterReason {
    /// No vertices and no edges — a thumbnail/scan false positive.
    Degenerate,
    /// Back-reference resolution below the 75% confidence floor.
    LowConfidence {
        satisfied: usize,
        constraints: usize,
    },
}

impl Diagnostic {
    /// Desync diagnostics indicate the walker lost and re-found (or lost and
    /// gave up on) the stream — never expected on a clean 2013–2017 file.
    pub fn is_desync(&self) -> bool {
        !matches!(
            self,
            Diagnostic::RunFiltered { .. }
                | Diagnostic::HeuristicFraming { .. }
                | Diagnostic::HeuristicLinkage { .. }
                | Diagnostic::HeuristicAnchor { .. }
                | Diagnostic::ContinuousWalk { .. }
                | Diagnostic::ContinuousFallback { .. }
                | Diagnostic::PadSlotBound { .. }
                | Diagnostic::MaterialLinkFallback { .. }
                | Diagnostic::UnresolvedSharedTexture { .. }
        )
    }
}

/// Walk and resolve every user-geometry run, applying the reference parser's
/// filters (drop degenerate + low-confidence runs).
pub fn geometry_runs(d: &[u8]) -> Vec<GeometryRun> {
    geometry_runs_with_diagnostics(d).0
}

/// [`geometry_runs`] plus every recorded [`Diagnostic`] (Phase 0.4): per-run
/// resync/truncation/skip events and the run-filter decisions.
pub fn geometry_runs_with_diagnostics(d: &[u8]) -> (Vec<GeometryRun>, Vec<Diagnostic>) {
    let mut out = Vec::new();
    let mut diagnostics = Vec::new();
    for run in walk::walk_definitions(d) {
        // A run the filters DROP was a false-positive candidate (the scan
        // over-generates by design — thumbnails, dynamic-component
        // fragments); its internal stall/resync churn is noise about
        // non-geometry bytes, so only the RunFiltered record survives.
        // Desync diagnostics are surfaced from runs that pass the filters —
        // there they describe real geometry the walk lost and re-found.
        let (vertices, edges) = (run.count("CVertex"), run.count("CEdge"));
        if vertices == 0 && edges == 0 {
            diagnostics.push(Diagnostic::RunFiltered {
                start: run.start,
                reason: RunFilterReason::Degenerate,
            });
            continue;
        }
        let res = resolve::resolve(&run.map, &run.objs);
        let (sat, con) = (res.satisfied, res.constraints);
        // Phase 0.6: with the template-pid filter gone, thumbnail-PNG bytes
        // can fake a list prelude and yield a tiny UNFRAMED run whose
        // handful of "objects" carry zero back-ref constraints — nothing
        // vouches for it. Real runs are framed (or, like attributes.skp's
        // fragments, carry constraints).
        if run.frame.is_none() && con == 0 && vertices + edges + run.count("CFace") < 4 {
            diagnostics.push(Diagnostic::RunFiltered {
                start: run.start,
                reason: RunFilterReason::Degenerate,
            });
            continue;
        }
        if con > 0 && (sat as f64) < 0.75 * con as f64 {
            diagnostics.push(Diagnostic::RunFiltered {
                start: run.start,
                reason: RunFilterReason::LowConfidence {
                    satisfied: sat,
                    constraints: con,
                },
            });
            continue;
        }
        diagnostics.extend(run.diagnostics.iter().cloned());
        out.push(GeometryRun {
            start: run.start,
            end: run.end,
            top_level: run.objs.len(),
            frame: run.frame,
            def_index: run.def_index,
            topology: Topology {
                vertices,
                edges,
                faces: run.count("CFace"),
                loops: run.count("CLoop"),
                edge_uses: run.count("CEdgeUse"),
                curves: run.count("CArcCurve"),
                dimensions: run.count("CDimensionLinear"),
                texts: run.count("CText"),
                section_planes: run.count("CSectionPlane"),
                images_placed: run.count("CImage"),
                construction_points: run.count("CConstructionPoint"),
            },
            resolved: (sat, con),
            mesh: mesh::build(&run.map, res.base),
            placed: run
                .map
                .iter()
                .filter_map(|s| match s {
                    carchive::Slot::Object(entity::Entity::InstancePlaced {
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
            curve_members: Vec::new(), // legacy walk never decodes CCurve
        });
    }
    (out, diagnostics)
}

/// Enumerate every class in a file via its `CVersionMap`: `(classname, schema)`.
/// Works on any file (the version map is written up front).
pub fn inventory(d: &[u8]) -> Result<Vec<(String, u32)>, Stall> {
    let start = header::object_stream_start(d).ok_or_else(|| Stall_new("<no header>"))?;
    let mut ar = carchive::CArchive::new(d, start);
    let vm = ar.read_object()?;
    if let carchive::Child::Obj(idx) = vm {
        if let carchive::Slot::Object(entity::Entity::VersionMap { entries }) = &ar.map[idx] {
            return Ok(entries.clone());
        }
    }
    Err(Stall_new("<no version map>"))
}

#[allow(non_snake_case)]
fn Stall_new(msg: &str) -> Stall {
    Stall {
        class: msg.to_string(),
        pos: 0,
        mapindex: 0,
        expected: None,
    }
}
