# OpenSKP

**An open, clean-room reader and specification for the SketchUp `.skp`
file format: the 2026 container that current SketchUp saves, and the
2017 binary format that older files still carry.**

SketchUp's native format has no public specification and, until now, no
open-source reader; the only way to read a `.skp` has been Trimble's
proprietary SDK. OpenSKP documents both containers and reads them into
one model:

- **A format specification** — [`docs/SKP_FORMAT.md`](docs/SKP_FORMAT.md)
  plus the Kaitai Struct grammar [`ksy/skp.ksy`](ksy/skp.ksy), detailed
  enough to build an independent parser.
- **A Rust SDK** — [`crates/openskp`](crates/openskp), a pure-Rust
  library (its only dependency is a DEFLATE decoder) that reads meshes,
  materials, UVs, hierarchy, layers, scenes, attributes, and the
  document's settings (camera, rendering options, shadows, units,
  styles, watermarks, and text/dimension annotations).
- **A CLI and a C ABI** — `openskp id|model|json|mesh`, and `libopenskp`
  with a plain-C header for every other language.
- **An evidence corpus** — [`corpus/`](corpus/), the authored minimal
  pairs and ground-truth exports behind every format claim, doubling as
  the test suite's oracle.

Everything derives from observing `.skp` files, their COLLADA exports,
and public knowledge of MFC `CArchive` serialization and the ZIP format.
**No Trimble SDK,
headers, or SDK-derived knowledge is used** — see the clean-room policy
in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## The format in one paragraph

A current `.skp` (SketchUp 2026, `{26.x}`) is two UTF-16 string records
followed by a ZIP archive. Its `model.dat` is a tree of tagged records
(`u16 tag | u32 length | payload`) holding the entities, their persistent
ids, transforms, and every document setting; materials are XML files next
to their texture images. A 2017 `.skp` (`{17.x}`, the last free desktop
release) is a fixed header followed by one uncompressed **MFC `CArchive`
object stream** with a single shared object map, where classes are
defined lazily inline and object references index that map. Both store
the same model: a half-edge kernel (`CVertex` / `CEdge` / `CEdgeUse` /
`CLoop` / `CFace`) in **f64 inches**, components and groups placing
shared definitions through 13-value transforms, and textures mapped
through per-face projective matrices. The full account is in
[`docs/SKP_FORMAT.md`](docs/SKP_FORMAT.md): §16 for the 2026 container,
§2–§13 for 2017.

## Quick start

```sh
git clone https://github.com/hew3d/openskp
cd openskp

cargo run -p openskp-cli --release -- model corpus/2026/house-plus.skp
cargo run -p openskp-cli --release -- mesh  corpus/2026/box.skp   # JSON out
cargo run -p openskp-cli --release -- model corpus/2017/house.skp  # a 2017 file
```

As a library:

```rust
let model = openskp::Model::read("model.skp")?;
for run in &model.geometry {
    // a 1 m box: 8 vertices / 12 edges / 6 faces
    println!("{:?}", run.topology);
}
for node in model.scene() {
    // composed world transforms, inherited materials
}
```

From C (or anything that speaks JSON):

```c
openskp_model *m = openskp_open("model.skp");
puts(openskp_mesh_json(m));    /* meshes + composed scene */
openskp_free(m);
```

See [`docs/SDK.md`](docs/SDK.md) for the full API tour.

## How correctness is established

The corpus is the oracle. Every decoded structure is validated three
ways: **by construction** (files authored with typed exact dimensions, so
the right bytes are knowable in advance), **against COLLADA** (SketchUp's
own exports provide ground-truth coordinates, connectivity, colors, and
UVs), and **at scale** (a 10.7 MB production model must match its export
exactly — 41,725 world vertices, 26,918 face rings, and 7,649 textured
faces' UVs do).

```sh
cargo test --workspace --release     # the acceptance suite
./scripts/verify.sh                  # Kaitai grammar + reference-parser checks
```

`verify.sh` needs a Python venv with `kaitaistruct` and an
`npm install kaitai-struct-compiler js-yaml`.

## Scope

- **SketchUp 2026 (`{26.x}`)**, the format every current release saves,
  the free web app included. Every 2017 corpus file has a 2026 twin;
  each twin reads equivalent to its original (geometry, hierarchy,
  materials, layers, UVs, scenes, guides, document settings), and the
  COLLADA-oracle tests run on both containers; the 2026 reader is the
  faster and lighter of the two (a 122 MB
  production model: 1.2 s and 384 MB as a 60 MB 2026 file, 2.2 s and
  544 MB as the 2017 original). A damaged record is skipped and reported
  as a diagnostic; a record tag this reader does not know, at the top
  level or in an entity container, is reported rather than dropped.
- **SketchUp 2017 (`{17.x}`)**, the last release with a free desktop
  edition, so many existing models are in this format. The same model
  comes out of either container. The 2017 walk reads production-scale
  models well beyond the corpus (a 464 MB third-party house in 12 s and
  2 GB) with no desync; a process that cannot hold the model gets a named
  out-of-memory error instead of aborting.
- **Untested:** releases 2018–2025 are unobserved. Any file whose header
  strings are followed by a ZIP archive is read as a 2026 file.
- **Identified but degraded:** 2013–2016 saves parse header-level with
  loud diagnostics.
- **Read-only:** writing `.skp` is a parked milestone
  ([`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md)).

## Repository layout

```
docs/       SKP_FORMAT.md (the spec) · SDK.md (using the SDK) · DEVELOPMENT.md
ksy/        skp.ksy — Kaitai Struct grammar (header + record catalogue)
crates/     openskp (core library) · openskp-cli · openskp-capi (C ABI)
corpus/     the evidence base: 2026 saves and their 2017 originals as
            minimal pairs, 2013–2016 saves, a full-scale third-party
            benchmark in both containers (see corpus/README.md)
tools/      Python analysis instruments and the frozen reference parser
scripts/    verify.sh
```

## License

GPL-3.0-only — see [`LICENSE`](LICENSE). Alternative commercial licensing
may be offered in the future; contributions are accepted under the terms
in [`CONTRIBUTING.md`](CONTRIBUTING.md), which keep that possible.

SketchUp is a trademark of Trimble Inc. This project is independent of
and unaffiliated with Trimble; it interoperates with the `.skp` format
based on clean-room analysis of files. Texture images embedded in the
corpus `.skp` files are third-party content outside the GPL grant — see
[`corpus/README.md`](corpus/README.md#licensing).
