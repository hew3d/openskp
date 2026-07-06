# Using the OpenSKP SDK

OpenSKP reads SketchUp 2017 `.skp` files without the Trimble SDK. The core
is a zero-dependency Rust library; a CLI and a C ABI are built on top of
it, and any language that can parse JSON can consume the C ABI's output.

Read support covers the full 2017 surface validated by the test suite:
concrete meshes (world-space capable), the component/group hierarchy with
transforms, materials (solid and textured, including embedded images and
per-corner UVs), layers, per-entity visibility, scenes, guides, attribute
dictionaries, and annotations surfaced as counts and data. Writing `.skp`
files is not supported (see `docs/DEVELOPMENT.md` for the parked design).

## The Rust crate

```toml
[dependencies]
openskp = { git = "https://github.com/hew3d/openskp" }
```

The core crate has no runtime dependencies.

### Reading a model

```rust
let model = openskp::Model::read("model.skp")?;          // from a path
let model = openskp::Model::parse(&bytes)?;              // from memory

println!("{} ({} definitions, {} materials)",
         model.version, model.definitions.len(), model.materials.len());
```

`Model` exposes the document as plain data:

| field / method | contents |
|---|---|
| `version`, `format_guid` | header identification |
| `definitions` | component/group definitions (`name`, `guid`, map slot) |
| `instances` | placements: definition ref, 13-value transform, name |
| `geometry` | per-definition/root `GeometryRun`s: resolved `Topology` counts, a materialized `Mesh`, child `PlacedInstance`s |
| `materials`, `layers`, `scenes`, `guides`, `attributes`, `images` | appearance and document data |
| `diagnostics` | every anomaly the walk recorded — an empty desync set is the clean-parse signal |
| `scene()` | the composed instance tree as `Node`s with world transforms, inherited materials, visibility |
| `material_of(slot)`, `layer_of(slot)`, `applied_size_of(slot)` | resolve store-map slot references from meshes/instances |
| `to_json()`, `mesh_json()` | the whole model, or per-run meshes + composed scene, as JSON |

### Geometry

Coordinates are exposed in **metres** (the file stores f64 inches;
`openskp::INCH` is the conversion constant). A `Mesh` holds vertices,
edges with soft/smooth flags, and faces as ordered vertex rings (outer
ring plus hole rings, wound to agree with the decoded plane normal).

```rust
for run in &model.geometry {
    let t = &run.topology;                 // a 1 m box: 8 / 12 / 6
    println!("v={} e={} f={}", t.vertices, t.edges, t.faces);
}
```

### World space and UVs

`Model::scene()` composes transforms down the instance tree. For textured
faces, `MeshFace::uv_xform(side, applied_size)` returns the projection
that maps face positions to UVs — matching SketchUp's own COLLADA export
per corner:

```rust
for node in model.scene() {
    // node.world: row-major 4x4; node.material: inherited material slot
    // node.hidden / node.layer: visibility inputs
}
```

### Error handling and diagnostics

`Model::parse` fails only when the container is unreadable (e.g. a
post-2017 ZIP-container file). Everything else parses; anomalies are
never silent — they land in `model.diagnostics` (resyncs, skipped
records, filtered runs, fallback decisions). `openskp::header_info`
identifies any SketchUp file's version and format GUID, even ones the
reader then refuses.

Pre-2017 files (2013–2016) parse header-level with recorded degradation;
their body layouts are not decoded.

## The command-line tool

```sh
cargo run -p openskp-cli --release -- <command> <file.skp>
```

| command | output |
|---|---|
| `openskp id <file>` | version, format GUID, container class count |
| `openskp model <file>` | human-readable summary: definitions, geometry, materials, layers, diagnostics |
| `openskp json <file>` | the full model as JSON on stdout |
| `openskp mesh <file>` | concrete meshes + the composed scene as JSON on stdout |

## The C ABI

`openskp-capi` builds `libopenskp` as both a static and a dynamic library
with the header [`crates/openskp-capi/include/openskp.h`](../crates/openskp-capi/include/openskp.h):

```c
#include "openskp.h"

openskp_model *m = openskp_open("model.skp");     /* or openskp_parse(buf, len) */
if (m) {
    const char *json = openskp_model_json(m);     /* owned by m */
    const char *mesh = openskp_mesh_json(m);      /* meshes + composed scene */
    /* ... */
    openskp_free(m);
}
```

Build and run the bundled smoke test:

```sh
cargo build -p openskp-capi --release
cc crates/openskp-capi/examples/smoke.c \
   -I crates/openskp-capi/include \
   -L target/release -lopenskp \
   -o /tmp/openskp_smoke
/tmp/openskp_smoke corpus/2017/box.skp
```

All decoding lives in the Rust core; the C layer is a binding only.
Returned strings are owned by the handle and valid until `openskp_free`.
The JSON accessors are the portable surface — typed accessors are added
as consumers need them.

### The JSON surfaces

`openskp_model_json` / `Model::to_json` emit the document: version, GUID,
definitions, instances, geometry-run topology, materials (with texture
metadata), layers, scenes, guides, attributes, images, and any desync
diagnostics.

`openskp_mesh_json` / `Model::mesh_json` emit render-ready data: per-run
vertices (metres), edges, faces as index rings with per-side material
slots and UV transforms, plus a `scene` array of leaves — run index,
row-major 4×4 world matrix, and the inherited material slot — for
world-space assembly in any language.

## Guarantees and limits

- **Validated equivalence.** The test suite proves world-space vertex,
  face-ring, material, and UV equivalence against SketchUp's own COLLADA
  exports, from minimal pairs up to a 10.7 MB production model
  (`cargo test --workspace --release`).
- **2017 scope.** Files saved by SketchUp 2017 (v17.x) are the supported
  input. 2013–2016 degrade loudly; post-2017 is detected and refused.
- **Read-only.** No write/round-trip support.
- Some record contents remain unknown but are skipped with exact extents
  — see `docs/SKP_FORMAT.md` §15 for the inventory.
