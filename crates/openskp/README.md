# openskp

Clean-room reader for the SketchUp `.skp` format: the 2026 container
that current SketchUp saves, and the 2017 binary format of older files.
Reads meshes, materials (solid and textured, including embedded images
and per-corner UVs), the component/group hierarchy with transforms,
layers, scenes, guides, attribute dictionaries, and the document's
settings (camera, rendering options, shadows, units, styles and
watermarks, fonts, and text/dimension annotations), with one runtime
dependency (a DEFLATE decoder) and **no Trimble SDK anywhere in its
lineage**.

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

**Supported:** SketchUp 2026 files (`{26.x}`, the format every current
release saves) and SketchUp 2017 files (`{17.x}`, the last free desktop
edition). Both produce the same `Model`; every 2017 corpus file has a
2026 twin that reads equivalent to it, and the COLLADA-oracle tests run
on both containers.
Releases 2018–2025 are untested; a file whose header strings are
followed by a ZIP archive is read as a 2026 file. Production-scale
models read on both paths (a 464 MB 2017 house in 12 s and 2 GB); a
process too small to hold a model gets a named out-of-memory error
instead of aborting, and a damaged record is skipped and reported rather
than failing the file. Writing `.skp` is not supported.

See [`docs/SDK.md`](https://github.com/hew3d/openskp/blob/main/docs/SDK.md)
for the full guide, and the companion crates
[`openskp-cli`](https://crates.io/crates/openskp-cli) (command-line
tool) and [`openskp-capi`](https://crates.io/crates/openskp-capi)
(C ABI).

## License

GPL-3.0-only. Contributions must satisfy the clean-room policy in
[`CONTRIBUTING.md`](https://github.com/hew3d/openskp/blob/main/CONTRIBUTING.md).
