# Development

How OpenSKP was built, how its pieces fit together, and what is
deliberately parked. The format itself is specified in
[`SKP_FORMAT.md`](SKP_FORMAT.md); how to consume the SDK is in
[`SDK.md`](SDK.md).

## Method

The format was reverse engineered clean-room, with no Trimble SDK input,
from three kinds of evidence:

1. **Authored minimal pairs.** Files drawn from the origin along axes
   with typed exact dimensions, each isolating one feature against a
   named base file (see [`corpus/`](../corpus)). Known constants (e.g.
   1 m = 39.37007874015748") are searchable as f64 bit patterns, which
   turns "where is the coordinate?" into a deterministic query.
2. **COLLADA ground truth.** SketchUp's own `.dae` export of every
   geometry sample supplies exact coordinates, connectivity, material
   colors, and UVs to validate against.
3. **Public MFC `CArchive` knowledge.** The container is a standard MFC
   object stream; its tag protocol and string encodings are documented
   public behavior, which converted decoding from guessing into walking
   known conventions.

Record bodies were decoded smallest-first (vertex → edge → face → solids
→ components → materials → UVs), each promoted into the spec only with a
corpus file and offsets as evidence. Three oracle tiers gate correctness:
by-construction values (typed dimensions), COLLADA equivalence, and a
full-scale third-party model.

Two practical lessons shaped the tooling: byte diffs only align for
same-length in-place edits (structural changes shift every later offset
and renumber the object map), and MFC streams have no record framing —
"skip an unknown record" is impossible without per-class knowledge, so
continue-past-unknown parsing has to be architected, not patched in.

## Architecture

- **`tools/` (Python, stdlib-only)** — the reverse-engineering
  instruments, kept as the frozen differential oracle:
  - `skptool.py` — byte-level analysis CLI (`id | classes | walk |
    classdelta | diff | window | strings16`).
  - `carchive.py` / `skpwalk.py` / `skpparse.py` — the frozen reference
    parser lineage; `tests/oracle.json` captures its output and the Rust
    parser must reproduce it.
  - `contwalk.py` — the living prototype of the continuous walk; new
    structures are usually decoded here first, then ported to Rust.
  - `extract_images.py`, `validate.py`, `ksy_validate.py`,
    `walk_validate.py` — extraction and validation harnesses.
- **`ksy/skp.ksy`** — the Kaitai Struct grammar: the deterministic header
  and the formal record-type catalogue. The entity stream itself is a
  stateful object graph (running map indexes, lazily defined classes)
  outside Kaitai's declarative model, so the grammar documents types
  while the hand-written walker does the decode.
- **`crates/openskp`** — the reference implementation. Key seams:
  - `carchive.rs` — the MFC tag protocol, one shared store map, big-tag
    escapes, escalated strings.
  - `walk2.rs` — the continuous single-archive walk with deterministic
    slot-base calibration; the primary path for every 2017 file.
  - `walk.rs` / `resolve.rs` — the legacy run-based walk and modal-base
    back-reference voting; frozen, serves 2013–2016 degraded reads.
  - `entity.rs` — per-class body readers in a `(class, schema range)`
    registry with three tiers: Full (complete reader), Skip (exact
    extent), Resync (scan forward, loudly). Unexpected schema numbers
    downgrade instead of running a mismatched reader.
  - `extract.rs` — signature-based scanners for pre-model content the
    walk never meets (materials, scenes, options dictionaries).
  - `mesh.rs` / `model.rs` — mesh materialization (ring chaining,
    plane-oriented winding, UV transforms) and model assembly with JSON
    writers.
  - Diagnostics are load-bearing: anomalies are recorded, never silent,
    and "zero desync diagnostics on every 2017 corpus file" is a standing
    regression test.
- **`crates/openskp-cli`**, **`crates/openskp-capi`** — thin consumers of
  the core (see `SDK.md`).

## Provenance conventions in source comments

- `§4x`-style anchors cite format facts; they resolve via
  `SKP_FORMAT.md` Appendix B.
- `Phase N.M` labels cite the development plan that drove the work,
  summarized below. They mark why a test or seam exists; the numbering is
  historical.

| phase | delivered |
|---|---|
| 0 | Architecture: version/container context, the `(class, schema)` reader registry, tiered read outcomes, recorded diagnostics, MFC string escalation, removal of all default-template assumptions |
| 1 | Structural segmentation: entity-list count framing, the definition-list prelude, structural list discovery replacing cluster heuristics |
| 2 | Class-body coverage: dimensions, text, fonts, section planes, construction points, images; the class-tier checklist |
| 3 | Semantics: concrete mesh API, face→material linkage, scene hierarchy with composed transforms, per-entity hidden/layer flags, texture bytes |
| 4 | UVs: the `CFaceTextureCoords` body, pin lists, and the projective UV formula |
| 5 | Hardening: persistent-id bitmask, big-tag escapes, version degradation for 2013–2016, stress-scale walks |
| 6 | The single-archive model: the continuous walk, whole-model COLLADA equivalence, the third-party benchmark, and the C ABI mesh surface |

## Acceptance state

The current test suite is the acceptance record
(`cargo test --workspace --release` + `scripts/verify.sh`):

- Every 2017-era corpus file parses on the continuous path with zero
  desync diagnostics; the legacy fallback serves 2013–2016 only.
- `house.skp` ≡ `house.dae` in world space (vertices, face rings,
  materials, UVs, node tree); `house-plus.skp` ≡ `house.skp` with its
  annotations surfacing as data only.
- The third-party benchmark: 41,725 world vertices and 26,918 face rings
  match the model's COLLADA export exactly in both directions, with all
  7,649 textured faces' UVs reproduced.
- The class catalogue in the spec is machine-checked against the
  format's own `CVersionMap` inventory.
- Every remaining unknown byte range is catalogued in `SKP_FORMAT.md`
  §15.

## Parked / future work

Roughly in order of likely value:

- **Write support.** Staged plan: same-length in-place edits first
  (material color is known to round-trip byte-exactly), then full
  `CArchive` authoring with save-noise reproduction (doc id, re-rendered
  thumbnails). A distinct, larger effort than reading.
- **Post-2017 containers.** Newer releases wrap a ZIP archive after the
  header string records (`corpus/future/`). Reading them is a new
  container backend behind the `detect_container` seam; the inner model
  serialization is unexplored.
- **2013–2016 body decode.** The container parses today; class bodies
  differ per release. The `(class, schema range)` registry is the
  intended landing zone; an evidence corpus per release would be needed.
- **Remaining record fields.** The unknowns of `SKP_FORMAT.md` §15 —
  camera intrinsics, dimension tail semantics, layer tail fields —
  plus corpus files for never-observed classes (`CDimensionRadial`,
  a true `CPolyline3d` instance, a finite guide segment, an
  arc saved by 2013/14 to widen `CArcCurve`'s schema range).
- **Typed C accessors.** The C ABI currently exposes JSON plus counts;
  typed accessors can be added as consumers appear.
- **Ergonomics.** Publishing the crate to a registry; iterator-style
  mesh traversal; optional serde derives behind a feature gate.
