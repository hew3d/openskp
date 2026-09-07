# OpenSKP

**An open, clean-room reader and specification for the SketchUp 2017
`.skp` file format.**

SketchUp's native format has no public specification and, until now, no
open-source reader — the only way to read a `.skp` has been Trimble's
proprietary SDK. OpenSKP documents the format and reads it natively:

- **A format specification** — [`docs/SKP_FORMAT.md`](docs/SKP_FORMAT.md)
  plus the Kaitai Struct grammar [`ksy/skp.ksy`](ksy/skp.ksy), detailed
  enough to build an independent parser.
- **A Rust SDK** — [`crates/openskp`](crates/openskp), a zero-dependency
  library that reads meshes, materials, UVs, hierarchy, layers, scenes,
  and attributes.
- **A CLI and a C ABI** — `openskp id|model|json|mesh`, and `libopenskp`
  with a plain-C header for every other language.
- **An evidence corpus** — [`corpus/`](corpus/), the authored minimal
  pairs and ground-truth exports behind every format claim, doubling as
  the test suite's oracle.

Everything derives from observing `.skp` files, their COLLADA exports,
and public knowledge of MFC `CArchive` serialization. **No Trimble SDK,
headers, or SDK-derived knowledge is used** — see the clean-room policy
in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## The format in one paragraph

A `.skp` is a fixed header (UTF-16 string records, version, format GUID)
followed by one uncompressed **MFC `CArchive` object stream** with a
single shared object map. Geometry is a half-edge kernel (`CVertex` /
`CEdge` / `CEdgeUse` / `CLoop` / `CFace`) in **f64 inches**; components
and groups place shared definitions through 13-value transforms; textures
map through per-face projective matrices. Classes are defined lazily
inline, so class tags differ per file, and object references index the
one global map. The full story is in
[`docs/SKP_FORMAT.md`](docs/SKP_FORMAT.md).

## Quick start

```sh
git clone https://github.com/hew3d/openskp
cd openskp

cargo run -p openskp-cli --release -- model corpus/2017/house.skp
cargo run -p openskp-cli --release -- mesh  corpus/2017/box.skp   # JSON out
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

- **Supported:** files saved by SketchUp 2017 (v17.x). This release
  matters — it is the last SketchUp with a free desktop edition, so
  2017-format files are what the free ecosystem produces.
- **Identified but degraded:** 2013–2016 saves parse header-level with
  loud diagnostics.
- **Identified and refused:** post-2017 saves (the container became a ZIP
  archive).
- **Read-only:** writing `.skp` is a parked milestone
  ([`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md)).

## Repository layout

```
docs/       SKP_FORMAT.md (the spec) · SDK.md (using the SDK) · DEVELOPMENT.md
ksy/        skp.ksy — Kaitai Struct grammar (header + record catalogue)
crates/     openskp (core library) · openskp-cli · openskp-capi (C ABI)
corpus/     the evidence base: 2017 minimal pairs, legacy saves,
            a full-scale third-party benchmark (see corpus/README.md)
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
