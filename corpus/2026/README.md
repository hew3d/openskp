# corpus/2026 — SketchUp 2026 saves

Models from the 2017 corpus saved again in SketchUp 2026's format
(`{26.2.0}`). Current SketchUp releases, the free web app included, save
only newer formats, so this is the evidence base for the post-2017
reader (`docs/SKP_FORMAT.md` §16). `crates/openskp/tests/read26.rs`
checks every file here equivalent to its 2017 original — geometry,
scene, materials, layers, UVs, scenes, guides and document settings — and
`tools/skp26_check.py` does the same from the Python instrument.

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
