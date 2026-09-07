# corpus/2017 — the SketchUp 2017 minimal-pair corpus

Every file was authored in SketchUp Make 2017 (v17.3.116) following the
conventions in [`../README.md`](../README.md): drawn from the origin along
the axes with typed exact dimensions, one isolated feature per file. A
same-basename `.dae` is the COLLADA ground-truth export where geometry,
materials, or UVs are asserted; the exporter's texture folders are not
kept (see [`../README.md`](../README.md)). Files without a `.dae` isolate
structures the COLLADA exporter does not carry (annotations, guides, save
noise).

Unless noted, files start from the 2017 default template (which seeds the
scale-figure component into every save — parsers must treat template
content as ordinary model content).

## Baselines and primitive geometry

| file | purpose |
|---|---|
| `empty.skp` | The default template with nothing drawn — the baseline every pair diffs against. |
| `empty-2.skp` | A second save of `empty.skp`: isolates save-to-save noise (doc id, re-rendered thumbnails) from stable payload. |
| `single-line.skp` | One 1 m line from the origin along X — the smallest records (`CVertex`, `CEdge`) with a searchable known coordinate. |
| `two-lines.skp` | Two lines sharing an endpoint — the first object back-reference (shared vertex). |
| `triangle-face.skp` | Three edges closed into the first face: `CFace`, `CLoop`, `CEdgeUse`. |
| `ngon-face.skp` | A hexagonal face — variable edge-use counts in one loop. |
| `face-with-hole.skp` | A face with an inner loop — multi-loop faces. |
| `box.skp` | The 1 m cube, the standing topology oracle: 8 vertices / 12 edges / 6 faces. |
| `blank-template-box.skp` | The cube in a truly blank template — proves the parser carries no default-template assumptions. |
| `cylinder.skp` | A push/pulled circle — larger mixed counts (72 edges / 26 faces) and curve-grouped side edges. |

## Curves and derived edges

| file | purpose |
|---|---|
| `arc.skp` | An open arc (radius 0.5 m) — edges grouped under one `CArcCurve` (center/axes/radius/sweep record). |
| `circle.skp` | A closed circle — a full `CArcCurve` ring that also produces a face. |
| `curve.skp` | Freehand (`CCurve`) polylines — the member-count curve record, distinct from `CArcCurve`. |
| `polyline.skp` | A drawn polyline — stored as plain `CEdge`s (`CPolyline3d` is declared but never instantiated). |

## Materials and textures

| file | purpose |
|---|---|
| `box-one-material.skp` | One painted cube face — solid material record, RGBA byte order. |
| `box-two-materials.skp` | Two distinct solid materials — material list layout and per-face binding. |
| `paint-one-face.skp` | Pure red on one face of the box — a same-length in-place diff against `box.skp`; face→material reference slot. |
| `back-material.skp` | Different front and back paint on one face — the independent front/back material references. |
| `material-one-face.skp` | A bundled library texture (`[Wood Floor Light]`) on one face — textured material layout, embedded JPEG `CDib`, applied size in inches. |
| `png-texture.skp` | A PNG (alpha) texture — the `CDib` PNG subtype and its differing applied-size framing. |

## UV mapping (`CFaceTextureCoords`)

| file | purpose |
|---|---|
| `uv-quad.skp` | A 1 m textured quad at default position — no FTC record is written; UVs reduce to position ÷ applied size. |
| `texture-scaled.skp` | Texture scaled via the position tool — a non-identity projection matrix (front side). |
| `texture-rotated.skp` | Texture rotated 90° — localizes the rotation slots. |
| `texture-rot45.skp` | Texture rotated exactly 45° — the 0.7071 values pin the 2×2 rotation block unambiguously. |
| `texture-offset.skp` | Texture offset by exactly 0.5 m — localizes the translation slots. |
| `texture-scale2x.skp` | A pure fixed-pin 2× scale — proves scale-via-applied-size writes no FTC at all. |
| `pin-identity.skp` | Free-pin mode entered and committed with no drag — no pins are written. |
| `pin-one.skp` | One pin dragged to the bottom-edge midpoint — a single pin record and a projective (non-affine) matrix. |
| `pin-four.skp` | All four pins dragged to authored midpoints — the full pin list; final positions only (drag order is not stored). |
| `pin-fixed-distort.skp` | Fixed-pin distortion — proves fixed-pin edits also write pin records. |

## Components, groups, and hierarchy

| file | purpose |
|---|---|
| `group.skp` / `box-group.skp` | Grouped geometry — a group is a component instance with an anonymous definition. |
| `box-component.skp` | One named component — definition + placed instance. |
| `box-component-two-instances.skp` | The same definition placed twice — instance reuse via a shared definition reference. |
| `two-components.skp` | Two distinct definitions — multi-definition serialization and per-definition entity lists. |
| `nested-component.skp` | A component inside a component — two-level hierarchy. |
| `nested-3-deep.skp` | Three levels of nesting — transform composition depth. |
| `component-move.skp` | An instance translated exactly 2 m — pins the transform's translation lanes. |
| `component-rotate.skp` | An instance rotated exactly 90° about Z — pins the 3×3 layout and row/column convention. |
| `instance-scaled.skp` | Non-uniform 2×1×1 scale plus a mirror — proves scale/mirror live in the 3×3 (negative determinant included). |
| `mixed-definition.skp` | A definition holding both loose geometry and a nested instance — the in-list instance body. |
| `long-name.skp` | A definition name longer than 255 characters — MFC string-length escalation, and a stale duplicate definition to disambiguate linkage. |

## Layers, visibility, annotation, document features

| file | purpose |
|---|---|
| `layers.skp` | Three user layers with per-layer boxes of authored sizes — layer records and per-entity layer references. |
| `hidden-entities.skp` | Exactly one hidden edge and one hidden face vs `box.skp` — the per-entity hidden flag. |
| `soft-smooth-edges.skp` | A cube with softened+smoothed edges — the edge soft/smooth flag bytes. |
| `dimension.skp` | Two linear dimensions — `CDimensionLinear` + `CSkFont` (inline and back-referenced forms). |
| `text.skp` | One leader text and one screen text — both `CText` variants. |
| `section-plane.skp` | An active section plane at exactly 0.5 m — the plane equation record. |
| `construction-point.skp` | A guide point at exactly (1 m, 2 m, 3 m) — position plus tape-measure anchor. |
| `guide.skp` | An infinite construction line 1 m off axis — anchor, unit direction, and the ±1e30 infinite-bounds sentinel. |
| `image.skp` | An imported raster image placed at 1 m width — the `CImage` entity. |
| `two-scenes.skp` | Two saved scenes — page records and their embedded cameras. |
| `attributes.skp` | A dynamic component — attribute containers/dictionaries and every typed value encoding. |

## Scale and stress

| file | purpose |
|---|---|
| `pid-stress.skp` | 2,703 boxes (~70k entities) — persistent-id byte-mask escalation, allocator jumps, and the MFC big-tag escape. |
| `house.skp` | A complete multi-room house (46+ definitions, 58 groups, 9 materials incl. textures) — the primary whole-model equivalence oracle vs `house.dae`. |
| `house-plus.skp` | `house.skp` plus dimensions, text, a placed image, and scenes — proves annotations surface as data without disturbing geometry (world-equal to `house.skp`). |
