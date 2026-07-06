# corpus/third-party — full-scale validation model

Files here lack by-construction oracles (no typed exact dimensions), so
they serve as stress and validation inputs rather than minimal pairs.

## theater-2017

A complete home-theater construction model, built from scratch in
SketchUp 2017 over several years of real use and contributed to this
project by its author. It is the project's third-party-scale benchmark:
large enough (10.7 MB) to exercise format behavior that no minimal pair
reaches — the MFC big-tag escape past 0x7FFF map slots, u16→u32 field
escalations, shared/deduplicated texture data, nested inline definitions,
and pinned texture projections on real faces.

| file | role |
|---|---|
| `theater-2017.skp` | the model |
| `theater-2017.dae` | its COLLADA export ("Export Hidden Geometry" enabled), the equivalence oracle |
| `theater-2017-stats.txt` | SketchUp's own entity statistics for cross-checking totals |
| `theater-2017/` | texture folder written by the COLLADA exporter |

Scale: 72,074 edges, 27,594 faces, 497 component instances, 920 groups,
149 component definitions, 94 layers, 81 materials.

The acceptance benchmark (`crates/openskp/tests/theater_dae.rs`) proves
world-space equivalence against the export: 41,725 world vertices and
26,918 canonical face rings match exactly in both directions, every
exported diffuse color is an extracted material, and all 7,649 textured
export faces reproduce the computed per-corner UVs.
