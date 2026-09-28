# Using the OpenSKP SDK

OpenSKP reads SketchUp `.skp` files — the 2017 format and the post-2017
ZIP container that current releases (SketchUp 2026) save — without the
Trimble SDK. The core is a pure-Rust library whose only dependency is a
DEFLATE decoder; a CLI and a C ABI are built on top of it, and any
language that can parse JSON can consume the C ABI's output.

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
| `version`, `model_guid` | header identification (`model_guid` is `None` outside the 2013–2017 container) |
| `container` | which file layout the model was read from: `Container::Carchive2017` or `Container::Zip` |
| `definitions` | component/group definitions (`name`, `guid`, map slot, `behaviour`, `timestamp`) |
| `instances` | placements: persistent id (`pid`), definition ref, 13-value transform, name, global store-map `slot` |
| `geometry` | per-definition/root `GeometryRun`s: resolved `Topology` counts, a materialized `Mesh`, child `PlacedInstance`s, placed `SectionPlane`s |
| `materials`, `layers`, `guides`, `attributes`, `images` | appearance and document data |
| `scenes` | the document's named views, as typed `Scene`s (see below) |
| `camera`, `rendering`, `shadows`, `units` | the current view and document settings: `Camera`, `RenderingOptions`, `ShadowInfo`, `Units` |
| `styles`, `active_style`, `watermarks` | saved `Style`s, the index into `styles` of the active one, and the current view's `Watermark`s |
| `fonts`, `texts`, `dimensions` | annotation entities: `Font`s, and `Text`/`Dimension` entities that index into `fonts` |
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

Each `Instance`/`PlacedInstance` carries the placing object's persistent
id (`pid`) — the join key into `Scene::hidden_entities` — and, on the
2017 continuous walk, its global store-map `slot`; `slot` is `None` on
the legacy byte-scan path and on post-2017 files, and `pid` is `0` on
the legacy path. `GeometryRun` also carries `sections: Vec<SectionPlane>`
— the section planes placed directly in that run, in the run's LOCAL
frame (unit normal `plane[0..3]`, offset `plane[3]`, inches), read on
both containers; `Scene::active_section_planes` names them by `pid`.

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

### Document settings

`Model` carries the document's saved view and display settings as typed
structs, shared by both readers: `Camera`, `RenderingOptions`,
`ShadowInfo`, `Units`, `Style`, `Watermark`, `Axes`, `TextDefaults`,
`DimensionDefaults`, `Animation`, `SceneProperties`, `Scene`, `Font`,
`Anchor`, `Leader`, `Text`, `Dimension`, and `Behaviour` (a definition's
component behaviour). Positions are in metres and angles in degrees,
matching the rest of the API; the one value kept in the document's
display unit is `Units::length_snap_length`.

```rust
if let Some(cam) = &model.camera {
    println!("eye {:?}, fov {} deg", cam.eye_m, cam.fov_deg);
}
println!("length unit {}", model.units.length_unit);
for style in &model.styles {
    println!("{}: {} watermark(s)", style.name, style.watermarks.len());
}
```

`RenderingOptions` holds the full set of display-settings fields
(edges, profiles, shadows-on-faces-style toggles, background/sky/ground
colors, section-plane display, and more), including the Model Info ▸
Components settings `component_axes` (show component axes while
editing) and `fade_rest_of_model` / `fade_similar_components`
(opacities 0–1); `style_view()` reduces it to just the fields a saved
style records — component editing isn't one of them — and
`section_planes()` / `section_cuts()` decode the packed section-display
bits. A `Style` pairs a `RenderingOptions` with its own `Watermark`s.
2017 layout: `docs/SKP_FORMAT.md` §10.7; post-2017: §16.10.

`Model::axes` is the model axes (`origin_m`, and unit vectors `x`, `y`,
`z`), and `Model::text_defaults` / `Model::dimension_defaults` are the
defaults applied to newly created texts and dimensions: a `font` index
into `Model::fonts` (`None` when unset), an `arrow` style (0 none, 1
slash, 2 dot, 3 closed, 4 open), and RGBA colors — dimensions also
carry `aligned` (text aligned to the dimension line vs. the screen) and
`text_position` (0 above, 1 centred, 3 below). All three are `None`
only when the record could not be located; in a 2017 file they sit in
the document tail after the shadow record (`docs/SKP_FORMAT.md`
§10.11), in a 2026 file under `0x01fc` / `0x01fe` / `0x01ff` (§16.3,
§16.13). `Model::anti_aliased_textures` (Model Info ▸ Rendering) is
`Option<bool>` for the same reason, read from the style manager's tail
in 2017 (§10.11) and from `0x020c` in 2026 (§16.3).

`Model::animation` (`Animation`: `transitions`, `transition_s`,
`delay_s` — seconds each scene is shown — and `loop_slideshow`) and
`Model::geo_located` (the model has a geo-location set) are read from
the model's attribute dictionary in both containers rather than a
dedicated settings record; `animation` defaults to all-zero/`false`
fields when the corresponding attributes are absent.

`scenes` changed in 0.4.0 from `Vec<String>` (names only) to
`Vec<Scene>`: each `Scene` carries its name, description, which parts
it saves (`Scene::saved: SceneProperties`), and the saved parts
themselves (`camera`, `rendering`, `style`, `shadows`, `axes`,
`hidden_entities`, `active_section_planes`, `hidden_layers`,
`in_animation`) — present exactly when `saved` reports that property.
`hidden_layers` holds indices into `Model::layers` (2017 layout:
`docs/SKP_FORMAT.md` §10.10; post-2017: §16.15). Code that only read
scene names now reads `scene.name` on each `Scene`.

`Text` and `Dimension` are annotation entities (persistent id, an
`Anchor` — what they're attached to: kind, point, and the anchored
entity's persistent id when one is stored — and a `font` index into
`Model::fonts`); `Definition` gained `behaviour: Behaviour` (glue,
cuts-opening, camera-facing, shadow flags) and `timestamp` (the
definition's last-edit time, UNIX seconds, 0 when unavailable).

The 2017 **legacy byte-scan path** — used only when the continuous walk
fails and recorded as a diagnostic when it triggers — cannot resolve
these as fully as the continuous path: it leaves `texts`, `dimensions`,
and `fonts` empty, and scene entity-id lists (`hidden_entities`,
`active_section_planes`, `hidden_layers`) empty. Instances and placed
components on this path carry `pid = 0` and `slot = None`, since it
cannot resolve persistent ids or global map slots. `camera`, `rendering`,
`shadows`, `units`, `styles`, `watermarks`, `axes`, `text_defaults`,
`dimension_defaults`, `anti_aliased_textures`, `animation`, and
`geo_located` are unaffected, except that the `font` index inside
`text_defaults`/`dimension_defaults` comes back `None` unless the tail
happens to redefine that font inline (fonts otherwise come from the
map, which this path doesn't have).

### Error handling and diagnostics

`Model::parse` fails only when the container is unrecognized or damaged
(a post-2017 archive whose entries fail their CRC-32, or whose records
do not tile), or when the process cannot allocate the continuous walk's
in-memory store map for a very large 2017 file. That case returns an
`Error` naming the file size instead of aborting the process — relevant
to hosts with a fixed memory budget, such as wasm — with a message of
the form `"out of memory while reading the model section at 0x… (N
bytes in): the model is larger than this process can hold"`; the store
map itself is released before texture extraction, to keep the peak
below the model's final resident size. Everything else parses; anomalies are
never silent — they land in `model.diagnostics` (resyncs, skipped
records, filtered runs, fallback decisions). `openskp::header_info`
identifies any SketchUp file's version (and, for 2013–2017 files, its
model GUID), even ones the reader then refuses. `detect_container`
tells the 2017 stream (`Container::Carchive2017`) from the post-2017 ZIP
container (`Container::Zip`); both read to the same `Model`. A 2026
file's `model_guid` is `None` (its header has none), its geometry-run
offsets are positions in the archive's `model.dat`, and `images` lists
every PNG/JPEG entry in the archive.

Pre-2017 files (2013–2016) parse header-level with recorded degradation;
their body layouts are not decoded.

## The command-line tool

```sh
cargo run -p openskp-cli --release -- <command> <file.skp>
```

| command | output |
|---|---|
| `openskp id <file>` | version, model GUID (2013–2017), container class count |
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
definitions (name, guid, timestamp, behaviour), instances (persistent
id, definition, translation, group flag), geometry-run topology,
materials (with texture metadata), layers, scenes (name, description,
saved-property bits, camera, rendering, style name, shadows, axes,
hidden-entity, active-section-plane and hidden-layer index lists,
in-animation flag), guides, attributes, images, the document's
`camera`, `rendering`, `shadows`, `units`, `axes`, `text_defaults`,
`dimension_defaults`, `anti_aliased_textures`, `animation`, and
`geo_located`, `styles` (with their `active_style` index) and their
watermarks, `watermarks` (the current view's), `fonts`, `texts`,
`dimensions`, and any desync diagnostics. `container` is
`"carchive2017"` or `"zip"`.

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
- **Supported versions.** Files saved by SketchUp 2017 (v17.x) and
  SketchUp 2026 (`{26.x}`); every 2026 corpus file reads equivalent to
  its 2017 original in geometry, scene, materials, layers, UVs, scenes,
  guides, and document settings (camera, rendering options, shadows,
  units, styles, scenes' saved parts, axes, annotation defaults,
  anti-aliased textures, animation, and geo-location), and the 2026
  benchmark meets the same COLLADA oracle. `images` (every archive
  image) and `attributes` (explicitly typed in 2026, a byte scan in 2017)
  legitimately differ between the two, as does the conversion's own
  added watermark (see `docs/SKP_FORMAT.md` §16.14). ZIP entries over
  1 GiB, or claiming over 100:1 compression, are refused. 2018–2025 are
  unobserved; 2013–2016 degrade loudly.
- **Read-only.** No write/round-trip support.
- Some record contents remain unknown but are skipped with exact extents
  — see `docs/SKP_FORMAT.md` §15 and §16.9 for the inventory.
