# Corpus

The evidence base for the format documentation and the test suite. Every
format claim in `docs/SKP_FORMAT.md` traces back to bytes in these files,
and the SDK's acceptance tests parse all of them.

## Layout

| directory | contents |
|---|---|
| [`2017/`](2017/) | The primary corpus: SketchUp 2017 (v17.3.116) files authored as minimal pairs, each isolating one format feature, with COLLADA (`.dae`) ground-truth exports. |
| [`legacy/`](legacy/) | The same 1 m box saved by SketchUp 2013–2016 — container identification and version-degradation coverage. |
| [`third-party/`](third-party/) | A full-scale real production model used as the stress/validation benchmark. |
| [`future/`](future/) | Post-2017 container probes (out of scope for the 2017 spec, kept as evidence). |

## Authoring conventions

Minimal-pair files are drawn from the origin, along the axes, with **typed
exact dimensions** (a line entered as `1m`, a box as `1m,1m,1m`). Endpoints
are then known constants that can be located deterministically in the byte
stream as IEEE-754 f64 inches (`1 m = 39.37007874015748"`). Each file
changes one thing relative to a named base file; each geometry file ships a
same-basename `.dae` export as ground truth for coordinates, connectivity,
material colors, and UVs.

## Provenance

All `.skp` files were saved by retail SketchUp releases from models built
for this project, except `third-party/` (see its README). Files that use
SketchUp's bundled material library embed the corresponding texture images
exactly as SketchUp saved them: the embedded bytes are part of the format
under test. The texture folders that SketchUp's COLLADA exporter writes
alongside a `.dae` are not kept in the repository — nothing in the test
suite or the specification reads them, and `.gitignore` excludes them so a
re-export cannot reintroduce them. A `.dae` whose `<init_from>` names an
absent image file is expected.

## Licensing

The corpus is covered by the repository's GPL-3.0-only license except for
third-party content embedded in the `.skp` files, which is not the
project's to license and is outside the GPL grant:

- Textures from SketchUp's bundled material library and the default
  template's scale-figure component are Trimble Content. They appear here
  as SketchUp wrote them into saved models, the use the SketchUp end-user
  license agreement permits ("in connection with the normal course of the
  operation of such Software"), and they remain Trimble's.
- Textures embedded in `third-party/theater-2017.skp` were supplied by the
  model's author.

Model and component thumbnails embedded in every `.skp` are SketchUp's
renders of the project's own geometry.
