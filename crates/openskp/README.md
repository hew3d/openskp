# openskp

Clean-room reader for the SketchUp `.skp` format — the 2017 binary
format and the post-2017 ZIP container. Reads meshes, materials (solid
and textured, including embedded images and per-corner UVs), the
component/group hierarchy with transforms, layers, scenes, guides,
attribute dictionaries, and the document's settings (camera, rendering
options, shadows, units, styles and watermarks, fonts, and text/
dimension annotations) — with zero runtime dependencies and **no
Trimble SDK anywhere in its lineage**.

Every format fact derives from observed `.skp` files, their COLLADA
exports, and public knowledge of MFC `CArchive` serialization. The
evidence corpus and the full format specification live in the
[repository](https://github.com/hew3d/openskp);
[`docs/SKP_FORMAT.md`](https://github.com/hew3d/openskp/blob/main/docs/SKP_FORMAT.md)
is detailed enough to build an independent parser.

## Reading a model

```rust
let model = openskp::Model::read("model.skp")?;   // from a path
let model = openskp::Model::parse(&bytes)?;       // from memory

println!("{} ({} definitions, {} materials)",
         model.version, model.definitions.len(), model.materials.len());
```

`Model` exposes the document as plain data: `definitions`, `instances`,
per-definition `geometry` runs with materialized meshes, `materials`,
`layers`, `scenes` (typed `Scene`s), `guides`, `attributes`, the
document's `camera`, `rendering` options, `shadows` and `units`, its
`styles` and `watermarks`, `fonts`/`texts`/`dimensions`, and
`diagnostics` (an empty desync set is the clean-parse signal). `scene()`
composes the instance tree into `Node`s carrying world transforms,
inherited materials, and visibility. Coordinates are exposed in metres
(`openskp::INCH` converts from the file's f64 inches). `header_info`
identifies any `.skp` from its header alone.

**Supported:** SketchUp's 2017 classic binary format (internally
versioned `{17.x}`), the format of the last free desktop edition, and the
post-2017 ZIP container that current releases save (observed: SketchUp
2026, `{26.x}`; 2018–2025 untested). Both produce the same `Model`.
Writing `.skp` is not supported.

See [`docs/SDK.md`](https://github.com/hew3d/openskp/blob/main/docs/SDK.md)
for the full guide, and the companion crates
[`openskp-cli`](https://crates.io/crates/openskp-cli) (command-line
tool) and [`openskp-capi`](https://crates.io/crates/openskp-capi)
(C ABI).

## License

GPL-3.0-only. Contributions must satisfy the clean-room policy in
[`CONTRIBUTING.md`](https://github.com/hew3d/openskp/blob/main/CONTRIBUTING.md).
