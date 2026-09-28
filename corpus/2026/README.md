# corpus/2026 — SketchUp 2026 saves

The evidence base for the 2026 container (`docs/SKP_FORMAT.md` §16),
the format every current SketchUp release saves, the free web app
included. Every model of the 2017 corpus is here as saved by SketchUp
2026 (`{26.2.0}`), so its fully decoded 2017 original pins every value.
`crates/openskp/tests/read26.rs` checks every file here equivalent to its
2017 original — geometry, scene, materials, layers, UVs, scenes, guides
and document settings — and the COLLADA-oracle tests run on these files
as well as on the originals; `tools/skp26_check.py` does the equivalence
check from the Python instrument.

Each converted file is the same model as a fully decoded 2017 file, so
that file's decode (and its `.dae`, where one exists) is the ground truth
for every value in the 2026 bytes. Entities can be matched between the
two by their coordinates. No separate `.dae` is kept here.

| file | 2017 original | contents |
|---|---|---|
| `box.skp` | [`../2017/box.skp`](../2017/box.skp) | The 1 m box, including the unplaced scale-figure definition the 2017 file carries. |
| `feature-pack.skp` | [`../2017/feature-pack.skp`](../2017/feature-pack.skp) | Soft/smooth/hidden flags, a distorted texture, typed attributes, a guide line and point, a section plane, leader and screen text, a dimension, a placed image, and two scenes in one model. |
| `house-plus.skp` | [`../2017/house-plus.skp`](../2017/house-plus.skp) | The multi-room house with dimensions, text, an image and scenes. |
| `scene-properties.skp` | [`../2017/scene-properties.skp`](../2017/scene-properties.skp) | A scene with a description and a cleared Hidden Geometry property: the scene record fields beyond the defaults (§16.15). |
| `scene-no-camera.skp` | [`../2017/scene-no-camera.skp`](../2017/scene-no-camera.skp) | A scene saved without its camera: a scene record without `0x714a` (§16.15). |
| `units-engineering.skp` | [`../2017/units-engineering.skp`](../2017/units-engineering.skp) | 2017 Engineering length format, which the conversion rewrites to decimal feet (§16.12). |
| `dimension-defaults.skp` | [`../2017/dimension-defaults.skp`](../2017/dimension-defaults.skp) | Dimension defaults aligned to the line and centred: `0x5fb6`, `0x5fc6` (§16.13). |
| `dimension-color.skp` | [`../2017/dimension-color.skp`](../2017/dimension-color.skp) | A dimension default colour: `0x5fc4` (§16.13). |
| `dimension-arrow-none.skp` | [`../2017/dimension-arrow-none.skp`](../2017/dimension-arrow-none.skp) | Dimension default endpoint None: `0x5fbb` 0 (§16.13). |
| `text-defaults.skp` | [`../2017/text-defaults.skp`](../2017/text-defaults.skp) | A leader-text default colour: `0x57ec` (§16.13). |
| `text-screen-color.skp` | [`../2017/text-screen-color.skp`](../2017/text-screen-color.skp) | A screen-text default colour: `0x57ed` (§16.13). |
| `text-arrow-none.skp` | [`../2017/text-arrow-none.skp`](../2017/text-arrow-none.skp) | Text default endpoint None: `0x57e7` 0 (§16.13). |
| `render-aa-off.skp` | [`../2017/render-aa-off.skp`](../2017/render-aa-off.skp) | Anti-aliased textures off: `0x020c` 0 (§16.3). |
| `component-fade.skp` | [`../2017/component-fade.skp`](../2017/component-fade.skp) | "Fade rest of model" at 60 %: `0x736c` 0.6 (§16.10). |
| `component-fade-similar.skp` | [`../2017/component-fade-similar.skp`](../2017/component-fade-similar.skp) | "Fade similar components" at 80 %: `0x736d` 0.8 (§16.10). |
| `component-axes.skp` | [`../2017/component-axes.skp`](../2017/component-axes.skp) | Component axes shown: `0x7349` 1 (§16.10). |
| `animation.skp` | [`../2017/animation.skp`](../2017/animation.skp) | A 5-second scene transition: `TransitionTime` (§16.12). |
| `geo-located.skp` | [`../2017/geo-located.skp`](../2017/geo-located.skp) | A manual geo-location: `UsesGeoReferencing`, `Latitude`, `Longitude` (§16.12). |
| `arc.skp` | [`../2017/arc.skp`](../2017/arc.skp) | An open arc (radius 0.5 m) |
| `attributes.skp` | [`../2017/attributes.skp`](../2017/attributes.skp) | A dynamic component |
| `back-material.skp` | [`../2017/back-material.skp`](../2017/back-material.skp) | Different front and back paint on one face |
| `blank-template-box.skp` | [`../2017/blank-template-box.skp`](../2017/blank-template-box.skp) | The cube in a truly blank template |
| `box-component.skp` | [`../2017/box-component.skp`](../2017/box-component.skp) | One named component |
| `box-component-two-instances.skp` | [`../2017/box-component-two-instances.skp`](../2017/box-component-two-instances.skp) | The same definition placed twice |
| `box-group.skp` | [`../2017/box-group.skp`](../2017/box-group.skp) | Grouped geometry: a group is a component instance with an anonymous definition. |
| `box-one-material.skp` | [`../2017/box-one-material.skp`](../2017/box-one-material.skp) | One painted cube face |
| `box-two-materials.skp` | [`../2017/box-two-materials.skp`](../2017/box-two-materials.skp) | Two distinct solid materials |
| `circle.skp` | [`../2017/circle.skp`](../2017/circle.skp) | A closed circle |
| `component-move.skp` | [`../2017/component-move.skp`](../2017/component-move.skp) | An instance translated exactly 2 m |
| `component-rotate.skp` | [`../2017/component-rotate.skp`](../2017/component-rotate.skp) | An instance rotated exactly 90° about Z |
| `construction-point.skp` | [`../2017/construction-point.skp`](../2017/construction-point.skp) | A guide point at exactly (1 m, 2 m, 3 m) |
| `curve.skp` | [`../2017/curve.skp`](../2017/curve.skp) | Freehand (`CCurve`) polylines |
| `cylinder.skp` | [`../2017/cylinder.skp`](../2017/cylinder.skp) | A push/pulled circle |
| `dimension.skp` | [`../2017/dimension.skp`](../2017/dimension.skp) | Two linear dimensions |
| `empty.skp` | [`../2017/empty.skp`](../2017/empty.skp) | The default template with nothing drawn |
| `empty-2.skp` | [`../2017/empty-2.skp`](../2017/empty-2.skp) | A second save of `empty.skp`: isolates save-to-save noise (doc id, re-rendered thumbnails) from stable payload. |
| `face-with-hole.skp` | [`../2017/face-with-hole.skp`](../2017/face-with-hole.skp) | A face with an inner loop |
| `group.skp` | [`../2017/group.skp`](../2017/group.skp) | Grouped geometry, as `box-group.skp`. |
| `guide.skp` | [`../2017/guide.skp`](../2017/guide.skp) | An infinite construction line 1 m off axis |
| `hidden-entities.skp` | [`../2017/hidden-entities.skp`](../2017/hidden-entities.skp) | Exactly one hidden edge and one hidden face vs `box.skp` |
| `house.skp` | [`../2017/house.skp`](../2017/house.skp) | A complete multi-room house (46+ definitions, 58 groups, 9 materials incl. textures) |
| `image.skp` | [`../2017/image.skp`](../2017/image.skp) | An imported raster image placed at 1 m width |
| `instance-scaled.skp` | [`../2017/instance-scaled.skp`](../2017/instance-scaled.skp) | Non-uniform 2×1×1 scale plus a mirror |
| `layers.skp` | [`../2017/layers.skp`](../2017/layers.skp) | Three user layers with per-layer boxes of authored sizes |
| `long-name.skp` | [`../2017/long-name.skp`](../2017/long-name.skp) | A definition name longer than 255 characters |
| `material-one-face.skp` | [`../2017/material-one-face.skp`](../2017/material-one-face.skp) | A bundled library texture (`[Wood Floor Light]`) on one face |
| `mixed-definition.skp` | [`../2017/mixed-definition.skp`](../2017/mixed-definition.skp) | A definition holding both loose geometry and a nested instance |
| `nested-3-deep.skp` | [`../2017/nested-3-deep.skp`](../2017/nested-3-deep.skp) | Three levels of nesting |
| `nested-component.skp` | [`../2017/nested-component.skp`](../2017/nested-component.skp) | A component inside a component |
| `ngon-face.skp` | [`../2017/ngon-face.skp`](../2017/ngon-face.skp) | A hexagonal face |
| `paint-one-face.skp` | [`../2017/paint-one-face.skp`](../2017/paint-one-face.skp) | Pure red on one face of the box |
| `pid-stress.skp` | [`../2017/pid-stress.skp`](../2017/pid-stress.skp) | 2,703 boxes (~70k entities) |
| `pin-fixed-distort.skp` | [`../2017/pin-fixed-distort.skp`](../2017/pin-fixed-distort.skp) | Fixed-pin distortion |
| `pin-four.skp` | [`../2017/pin-four.skp`](../2017/pin-four.skp) | All four pins dragged to authored midpoints |
| `pin-identity.skp` | [`../2017/pin-identity.skp`](../2017/pin-identity.skp) | Free-pin mode entered and committed with no drag |
| `pin-one.skp` | [`../2017/pin-one.skp`](../2017/pin-one.skp) | One pin dragged to the bottom-edge midpoint |
| `png-texture.skp` | [`../2017/png-texture.skp`](../2017/png-texture.skp) | A PNG (alpha) texture |
| `polyline.skp` | [`../2017/polyline.skp`](../2017/polyline.skp) | A drawn polyline |
| `section-plane.skp` | [`../2017/section-plane.skp`](../2017/section-plane.skp) | An active section plane at exactly 0.5 m |
| `single-line.skp` | [`../2017/single-line.skp`](../2017/single-line.skp) | One 1 m line from the origin along X |
| `soft-smooth-edges.skp` | [`../2017/soft-smooth-edges.skp`](../2017/soft-smooth-edges.skp) | A cube with softened+smoothed edges |
| `text.skp` | [`../2017/text.skp`](../2017/text.skp) | One leader text and one screen text |
| `texture-offset.skp` | [`../2017/texture-offset.skp`](../2017/texture-offset.skp) | Texture offset by exactly 0.5 m |
| `texture-rot45.skp` | [`../2017/texture-rot45.skp`](../2017/texture-rot45.skp) | Texture rotated exactly 45° |
| `texture-rotated.skp` | [`../2017/texture-rotated.skp`](../2017/texture-rotated.skp) | Texture rotated 90° |
| `texture-scale2x.skp` | [`../2017/texture-scale2x.skp`](../2017/texture-scale2x.skp) | A pure fixed-pin 2× scale |
| `texture-scaled.skp` | [`../2017/texture-scaled.skp`](../2017/texture-scaled.skp) | Texture scaled via the position tool |
| `triangle-face.skp` | [`../2017/triangle-face.skp`](../2017/triangle-face.skp) | Three edges closed into the first face: `CFace`, `CLoop`, `CEdgeUse`. |
| `two-components.skp` | [`../2017/two-components.skp`](../2017/two-components.skp) | Two distinct definitions |
| `two-lines.skp` | [`../2017/two-lines.skp`](../2017/two-lines.skp) | Two lines sharing an endpoint |
| `two-scenes.skp` | [`../2017/two-scenes.skp`](../2017/two-scenes.skp) | Two saved scenes |
| `uv-quad.skp` | [`../2017/uv-quad.skp`](../2017/uv-quad.skp) | A 1 m textured quad at default position |

The third-party benchmark's conversion is
[`../third-party/theater-2026.skp`](../third-party/theater-2026.skp).

## How the conversions were made

Each 2017 file was uploaded to the SketchUp free web app and immediately
downloaded with **Download ▸ SKP**: no edits, no save, and the purge
suggestion declined. Purging would remove unused definitions and
materials and break the correspondence with the 2017 original; the
download itself does not purge (`box.skp` keeps the unused scale-figure
definition).

To add a file, convert an existing 2017 corpus file the same way and keep
its basename.

## What the files show so far

After the two leading UTF-16 string records (the second is the version,
`{26.2.0}`), a ZIP archive starts at 0x41. Its entries include
`model.dat` (the model), `meta/meta.dat` (version and entry list),
thumbnails, one `materials/<name>/material.xml` per material and per
layer, the texture images as ordinary files next to their material,
`styles/`, `watermarks/`, and `classifications/`.

As a structural cross-check (not yet a decode), each converted file holds
the same number of vertex-shaped records in `model.dat`, and the same
material and layer counts, as its 2017 original's decode.
