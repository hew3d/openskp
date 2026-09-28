# The SketchUp `.skp` File Format (2026 and 2017)

This document specifies the two containers a SketchUp `.skp` file uses:
the 2026 container that every current SketchUp release writes (a ZIP
archive holding a self-describing record tree, §16) and the 2017 binary
format (one MFC `CArchive` stream, §2–§13), as established by clean-room
reverse engineering. Every statement derives from observing `.skp` files,
their COLLADA exports, or public knowledge of Microsoft MFC `CArchive`
serialization and the ZIP format — never from the Trimble SDK. Facts that
rest on a single observed instance are marked **candidate**; unknown
regions are listed in §15 (2017) and §16.16 (2026).

To read files current SketchUp saves, start at §16; it is self-contained
apart from the entity semantics it shares with the 2017 sections (§5–§10),
which it cites. Section numbers are stable citation anchors and are not
reordered.

Together with the Kaitai Struct grammar [`ksy/skp.ksy`](../ksy/skp.ksy)
(the deterministic header and the formal record-type catalogue), this
specification is sufficient to implement an independent reader. The Rust
implementation in [`crates/openskp`](../crates/openskp) is the reference;
the corpus in [`corpus/`](../corpus) is the evidence base.

**Anchors.** Headings carry bracketed labels like `[§4s]`. These are stable
citation anchors used throughout the repository's source comments and
match the section numbering of the original decode log; the full index is
in [Appendix B](#appendix-b-anchor-index).

## 1. Conventions

- All multi-byte integers are **little-endian**. `u8`/`u16`/`u32` are
  unsigned integers; `f64` is IEEE-754 double precision.
- Offsets are hexadecimal and, where given, refer to the named corpus
  file.
- Coordinates and all dimensional values are **f64 inches**;
  `1 m = 39.37007874015748"`.
- A *record* is a serialized object body; a *slot* is an index in the MFC
  store map (§3.2); *pid* is SketchUp's per-entity persistent id.

## 2. Container and header

A 2017 `.skp` file is a fixed header followed by **one uncompressed MFC
`CArchive` object stream**; it is not CFBF/OLE2 and not ZIP. Files saved
by SketchUp 2026 keep the header's string records and replace the stream
with a ZIP archive (§16).

Layout (offsets from `empty.skp`; all 2017 files match):

```
0x00  FF FE FF 0E + "SketchUp Model" (UTF-16LE)   document-type string record
0x20  FF FE FF 0A + "{17.3.116}"    (UTF-16LE)   version string record
0x3A  16 bytes                                    model GUID (per model)
0x4C  u32                                         doc id / save seed (per-save noise)
0x50  …                                           MFC CArchive object stream
```

- The 16-byte value at 0x3A identifies the MODEL, not the format: it is
  kept across re-saves of one model (`empty.skp`/`empty-2.skp` share
  `b9a4f057…`; `box.skp` and all four `corpus/legacy` boxes share
  `c5777fc7…`) and differs between models. SketchUp 2026's web-app
  conversion does not carry it into the 2026 container (`corpus/2026`).
- The first new-class record in the stream is always `CVersionMap`
  (at 0x56 in minimal files).

### 2.1 String records

Two string encodings appear throughout:

- **UTF-16 string record:** `FF FE FF <len:u8>` then `len` UTF-16LE code
  units. Per public MFC `CString` behavior, the length escalates: a length
  byte of `0xFF` is followed by a `u16` length, and a `u16` of `0xFFFF` by
  a `u32` (observed in practice for names over 255 characters —
  `corpus/2017/long-name.skp`).
- **ASCII string (class names):** `<len:u16>` then `len` ASCII bytes.

### 2.2 Save-to-save noise [§3]

Re-saving an unchanged model changes only: the doc id at 0x4C and the
embedded thumbnail PNGs (re-rendered each save, with different lengths).
The entire object payload after the preview region is byte-identical
across saves — the serialization is deterministic, which is what makes
minimal-pair byte differencing viable.

Byte diffs align only for same-length in-place edits (e.g. a material
color). Structural additions shift all later offsets and renumber the
store map, so structural comparisons must anchor on known constants
(typed exact coordinates searched as f64 inches) or use a record-level
walk.

## 3. The MFC CArchive object layer

### 3.1 Tag protocol

The stream is a graph of objects connected by MFC pointer serialization.
At every object position one of the following appears:

| form | encoding | meaning |
|---|---|---|
| null | `00 00` | null object pointer |
| new class | `FF FF <schema:u16> <namelen:u16> <name:ascii>` | defines a class lazily at first use; the first instance's body follows |
| class ref | `u16` with bit 15 set: `0x8000 \| slot` | new object of an already-defined class |
| object back-ref | `u16` with bit 15 clear | reference to an existing object by store-map slot |
| big-tag escape | `FF 7F` then `u32` | either form above for slots ≥ 0x7FFF |

### 3.2 One shared store map [§4s]

The whole payload from 0x50 is **one archive with a single 1-based store
map**. Every new-class record and every new object consumes one slot of
the same running counter; back-references anywhere in the file (topology
pointers, material references, definition references) are indexes into
this global map.

Consequences:

- **Class tags are per-file.** Classes are defined lazily in
  serialization order, so the same class gets different tags in different
  files. A parser must learn tags from each file's own stream, never
  hardcode them.
- **The pre-model slot base must be calibrated per file.** The number of
  slots consumed before the model section varies (thumbnails, managers,
  materials, imported images). Two structural anchors make calibration
  deterministic: the definition-list header opens with an object pointer
  to the **default layer**, which always sits in the slot directly after
  the `CLayer` class slot (single-layer files: `base = pointer − 2`); in
  multi-layer files the second list layer's class-ref gives the `CLayer`
  class slot directly. A reader should cross-check the calibrated base
  (e.g. that instance definition-references land on definition objects)
  and fail loudly on disagreement.
- Declared indexes can be stale: a definition's prelude (§5.3) may
  declare a writer-side index that differs from the read-side slot when
  purged definition-list slots shifted the numbering. **The read-side
  slot is authoritative** — instance references carry it.

### 3.3 The class inventory: CVersionMap

`CVersionMap` (always the first record) enumerates the file's complete
class list with per-class schema numbers — 58 classes in 2017
([Appendix A](#appendix-a-class-catalogue)). Schema numbers vary between
SketchUp releases (e.g. `CArcCurve` declares schema 2 in 2013/14 but 3 in
2017), so body readers must be keyed by `(class, schema range)` and
degrade safely on an unexpected schema.

## 4. Document layout [§4s]

Stream order of the archive:

```
CVersionMap
CSketchUpModel root header:  5 × u32  (2, 1200, 6, <max pid>, 0)
preview thumbnails (CDib), document attribute dictionaries,
camera / styles / options managers            (extent varies per file)
material manager: u32 count + count × CMaterial       [§4n]
model section:
  <u32 26> <u8 0> <count:u32>   layer list (count × CLayer)
  <u16 0x20> <count:u32>        definition list (count × CComponentDefinition;
                                purged slots serialize as null tags, and may
                                leave one stray object back-ref word)
  CComponentDefinition × N      trailing definitions, back-to-back
  <count:u32>                   THE ROOT ENTITY LIST (the model's own
                                entities: groups, instances, loose geometry)
root tail: u32 + <pointer> + <pointer> + 00 + u32 + 00 +
           geo-location strings (city, country) + latitude/longitude f64  [§4l]
page list: CViewPage × N (the scenes, §10.6) — after the root record,
           not in the pre-model manager region
```

The exact composition of the pre-model region is not specified — a reader
reaches the model section via the calibration anchors of §3.2 rather than
by decoding every manager. The root list ends exactly where the root tail
begins on every 2017 corpus file: a u32 (0 in every corpus file, 1 in a
third-party house), two nullable object pointers (the first is the root
list's ACTIVE section plane, a back-reference to its map slot: null in
every file without one, the lone plane's slot in the three corpus files
holding a plane, and in section-plane-deactivated.skp — two planes, the
older one deactivated — the newer plane's slot; in that house it is a
new `CRelationship` object (§7.1) written inline; the second is null in
every observed file), a zero byte, a u32 (`58 79 F0 6A` in every authored file,
`58 BC CB 3D` in theater-2017; meaning undecoded), a zero byte, then the
utf16 city string (`FF FE FF` at +14). The walk reads a counted list, so
a mis-sized body can still "complete" it on misread elements without a
stall; the reference reader therefore requires this shape exactly at the
list end and otherwise abandons the continuous walk with a recorded
fallback.

In minimal files the root entity-list count sits directly after the
preview thumbnail's PNG `IEND` chunk [§4j].

## 5. Entity structure

### 5.1 Entity preamble [§4i]

Every entity body begins:

```
<attribute pointer>   nullable MFC object pointer (§3.1) — the entity's
                      CAttributeContainer; null (00 00) for plain entities
<mask:u8>             bitmask of which pid bytes are stored
<pid bytes>           popcount(mask) bytes, ascending significance
```

The mask records which little-endian bytes of the pid are nonzero; zero
bytes are omitted and never stored (the encoding is canonical). Examples:
mask `0x03` = two low bytes stored (the common case for pid ≤ 0xFFFF);
`0x07` = three bytes (pid > 0xFFFF); `0x02` = pid ≡ 0 (mod 256) with the
low byte omitted.

Pids are a global allocator sequence: monotonically increasing with
occasional large jumps. They are not contiguous and not a count.

Scanner pitfalls (for tools that pattern-match entity headers): an MFC
new-class record for any 7-character class name (`FF FF … 07 00 43 …`)
mimics a mask-0x07 header — exclude tag `0xFFFF`; the big-tag escape word
`FF 7F` mimics mask 0x01 — no genuine pid ≤ 0xFF occurs in practice.

### 5.2 Drawing-element base ("drawbase") [§4q]

Drawable entities (edges, faces, instances, groups, dimensions, …) follow
the preamble with a fixed 10-byte block:

```
[0..2)   u16  material store-map slot (0 = none)          [§4n]
[2]      u8   hidden flag (1 = entity hidden)
[3..5)        01 01 (constant; semantics unconfirmed)
[5]      u8   soft flag (edges)
[6]      u8   smooth flag (edges)
[7]      u8   unknown (0 in all observed files)
[8..10)  u16  layer store-map slot (0 = the default layer)
```

Material and layer references are **global map slots** (§3.2). On
instances and groups, the material slot is a material painted on the
instance, which default-material faces beneath it inherit (§9.4).

### 5.3 Entity-list framing [§4h]

Every entity list — each definition's list and the model root's — is
immediately preceded by a `u32` count of its **top-level** elements
(inline children such as an edge's vertices or a face's loops are pool
objects, not list elements). A 1 m box list declares 18 = 12 edges + 6
faces; a file of 2,703 boxes declares exactly 48,654.

A **definition's** list carries a longer prelude:

```
<decl:u16, 0x7FFF escalates to u32> <u32 0> <count:u32> <list…>
```

`decl − 1` is the owning definition's writer-side map index — normally
equal to the read-side slot, but stale when purged slots shifted the
numbering (§3.2), and `0` (no index declared) on some definitions of
third-party models; the list and tail behind such a prelude read exactly.

## 6. Geometry kernel [§4c–§4f]

SketchUp geometry is a half-edge kernel: `CVertex`, `CEdge`, `CEdgeUse`
(the half-edge), `CLoop`, `CFace`, with curve records grouping edges.
Topology pointers use the new-or-backref protocol of §3.1: the first
object to mention a vertex embeds it inline; later objects back-reference
its slot.

### 6.1 CVertex (schema 0)

```
preamble (§5.1)
3 × f64        x, y, z in inches
```

### 6.2 CEdge (schema 2)

```
preamble + drawbase (soft/smooth in the flag bytes)
<curve pointer>       null | CCurve | CArcCurve object pointer
<endpoint pointer> ×2 CVertex objects (inline or back-ref)
```

### 6.3 CFace (schema 3)

```
preamble        (a textured face's attribute pointer holds an inline
                 CAttributeContainer whose child is the face's
                 CFaceTextureCoords — §9)
drawbase        ([0..2) = front material slot)
4 × f64         plane [A, B, C, D] — unit normal + offset
u32             loop count (1 + number of holes)
CLoop × count   outer loop first, then inner (hole) loops
(push/pulled faces: a trailing list of edge back-ref words, redundant
with the loops' edge-uses)
u16             back material slot (0 = none)
```

`CLoop` (schema 1): preamble + null-terminated list of `CEdgeUse`
objects. `CEdgeUse` (schema 1): edge object pointer + direction byte
(traversal sense) + parent-loop back-ref. Every edge-use's parent
reference names its loop's true global slot, which makes loops
self-checking alignment oracles for a walking parser.

Face rings are recovered by chaining a loop's edge-uses through shared
endpoints; ring orientation should be normalized to the decoded plane
normal (the plane is stored data; the derived ring order is not
authoritative).

### 6.4 Curve records

- **CArcCurve** (schema 3): preamble + 5-byte header + 14 × f64 — the arc
  frame (center, axes, radius, sweep angle). An arc or circle is a set of
  straight edges whose curve pointers share one `CArcCurve`; the first
  member edge defines it inline.
- **CCurve** (schema 4, freehand/welded curves): preamble + u8 + u32
  member-edge count. It owns no children — member edges are ordinary list
  elements that back-reference it.

## 7. Components, groups, and instances

### 7.1 CComponentDefinition (schema 10) [§4s]

```
preamble        (attribute pointer may be non-null: attribute dictionaries
                 on e.g. 3D-Warehouse components)
22 bytes        base: 00 00 00 01 01 then zeros in all 752 theater
                definitions (undecoded)
u32             layer count, then that many CLayer objects (each definition
                 carries its own copy of the default layer)
list prelude    (§5.3) + entity list
tail:
  u32           (0 with no relationships, 1 otherwise; semantics unconfirmed)
  <pointer>     nullable object pointer: the head of a CRelationship chain  [§4t]
  16 bytes      GUID
  utf16         name
  utf16         description
  utf16         source path (library provenance; empty on authored files)
  u32           UNIX timestamp (publish/save date) [0x1581]
  u32           [0x1582]: 0 or 1 in the converted models, where it
                equals 0x1582; 0x301 was also observed
  24 bytes      zero, except an f32 at +9 in 41 of 892 definitions
                (no 2026 counterpart found; undecoded)
  u8            glues to a surface          [0x1b5b]
  u8            cuts an opening             [0x1b5c]
  u32           glue plane                  [0x1b59]
  u8            bits: 1 always faces the camera, 2 shadows face the sun
                                            [0x1b5d, 0x1b5e]
  u32           [0x1b5a]
  3–8 bytes     (undecoded; zero)
  CThumbnail    object
```

`CRelationship` (schema 0) is a chain node: preamble + two object
pointers + a nullable pointer to the next relationship. Every authored
file's node has a null next pointer; a third-party apartment model chains
two. The provenance block (GUID through timestamp) belongs to the
definition tail and always follows the head pointer — an earlier reading
placed it inside the relationship record and made the definition's own
block "optional", which was this layout read out of phase.

The component behaviour (brackets: the 2026 record, §16.5) is
pinned by SketchUp Make 2017 pairs editing the stock figure's
behaviour (Components ▸ Edit), and decodes to the 2026 values for
all 191 component definitions of the corpus models.

An instance whose definition has not yet been serialized **inlines the
definition at the reference position** — definition lists are not
exhaustive; definitions can appear nested inside other definitions'
entity lists.

### 7.2 CComponentInstance (schema 5) and CGroup (schema 1) [§4o]

Byte-identical layout — a group is a component instance with an
anonymous definition:

```
preamble + drawbase   (drawbase material = instance-painted material, §9.4)
<definition ref>      MFC object pointer to the CComponentDefinition
                      (u16 back-ref word; big-tag escape on large maps;
                      may inline the definition, §7.1)
13 × f64              transform
utf16                 instance name (often empty)
16 bytes              GUID
```

### 7.3 Transform semantics [§4e]

The 13 f64 values are: 9 values of a 3×3 matrix in **row-major order
under the column-vector convention** (`p' = M·p`; values [0..3) are the
matrix's first row), then the translation (x, y, z) in inches, then a
homogeneous weight (1.0). Non-uniform scale and mirroring (negative
determinant) live in the 3×3. World placement composes parent × local
down the instance tree.

## 8. Appearance

### 8.1 CMaterial [§4e, §4f]

The material manager's list is `<count:u32>` + count tagged records —
record 0 is the `CMaterial` new-class declaration, later records
class-refs — and each record opens with the §5.1 entity preamble (a
non-null attribute pointer carries renderer-plugin dictionaries, e.g.
V-Ray's "VRayInfo"/"VRayPlugins"; pids are stored on some files). The
body after the preamble:

Brackets give the 2026 `material.xml` attribute (§16.6) with the
same value in the conversions of `theater`, `feature-pack`,
`house-plus` and `box` (103 materials).

Solid material:

```
utf16 name      (library/bundled names are bracketed: "[Wood Floor Light]")
u8    0         (no texture)
u8    0
4 bytes         RGBA, one byte each, in that order     [colorRed/Green/Blue]
utf16           texture path (empty in every observed file)
u32             type: 0 solid                          [type]
u32             colorize type                          [colorizeType]
f64             opacity 0..1 — the opacity slider's last position; it
                applies ONLY when the use-opacity flag is set (the RGBA
                alpha byte is an opaque flag, 0xFF)     [trans]
1 byte          use-opacity flag: stale slider values persist with the
                flag clear and render opaque (attributes.skp's *2 and *4
                both store 0.5; only flag-set *4 exports as transparent)
                                                       [useTrans]
```

Textured material:

```
utf16 name
u8    1                    has-texture flag
<pointer>                  nullable object pointer: the texture's own
                           attribute container (null on every authored
                           file; a "VRayTextureHelper" dictionary on a
                           third-party house's "Two Sided" material)
u8                         (0 observed)
<texture image>            an MFC object pointer: EITHER an inline CDib
                           object (class ref + body, §8.2), OR a back-ref
                           to the owning material's CDib GLOBAL map slot —
                           image data is deduplicated across materials
                           sharing a texture (house.skp: "[Wood Floor
                           Light]1" refs 23, the slot after "[Wood Floor
                           Light]" at 22) [§4s]
(JPEG payloads only) u32   JPEG quality (the 2026 `0x32cd`, §16.6)
f64 × 2                    applied texture size: width, height in inches
                           [texture xScale, yScale]
utf16                      texture filename        [textureFilename]
4 bytes                    colour RGBA (what exporters emit as the
                           material's diffuse colour) [colorRed/Green/Blue]
1 byte                     (0 observed)
4 bytes                    the texture's average colour RGBA [texture avgColor]
utf16                      empty in every observed file
u32                        type: 1 textured        [type]
u32                        colorize type           [colorizeType]
f64                        opacity 0..1, as for solids [trans]
1 byte                     use-opacity flag, as for solids (house.skp's
                           "[Translucent Glass Tinted]" stores 0.52
                           flag-set, the transparency its .dae export
                           carries; flag-clear materials read opaque)
                           [useTrans]
```

The eight theater materials with colorize type 1 are the only ones whose
u32 is 1. The conversion recomputes some texture average colours. Three
`house-plus` textures differ by one in a channel, and one `feature-pack`
texture's average is re-derived from its image. Every other field
matches exactly.

The record ends at the flag byte; the next record's tag follows
directly. The material region's slot arithmetic is therefore exact on an
archive walk: the `CLayer` class occupies slot `base + 1` (§3.2), so
the slots the material list consumes count back from there.

### 8.2 CDib (schema 3)

```
u32  subtype    1 = JPEG, 4 = PNG (no others observed)
u32  length
<length bytes>  raw image data
```

Used for material textures, preview thumbnails, and definition
thumbnails.

### 8.3 CLayer (schema 2) [§4s]

```
preamble
utf16   display name                                   [0x3c8d]
u8      hidden flag                                    [0x3c8e]
u16     0 in every observed layer
<pid>   a second §5.1 presence-masked pid field (mask 0 on authored files;
        2–3 bytes on a third-party house whose layers a plugin stamped)
utf16   internal name ("Layer_<name>")                 [0x3c8f material name]
2 bytes 00 01 in every observed layer
4 bytes RGBA layer colour                              [colorRed/Green/Blue]
utf16   empty in every observed layer
u32     type (0)                                       [type]
u32     colorize type (0)                              [colorizeType]
f64     colour opacity (0.5 or 0.0 observed)            [trans]
u8      use-opacity flag (0)                           [useTrans]
u32     0 in every observed layer                      [0x3c90] (candidate)
```

After the internal name comes the solid-material body of the layer's
colour (§8.1). The brackets name the 2026 layer record and its
embedded material document (§16.6). The layers of `theater`,
`feature-pack`, `house-plus` and `box` (99, 40 of them hidden) decode
to their conversions field for field (891 checks).

### 8.4 Material and layer binding [§4n]

Drawbase material/layer fields hold **global map slots**. Materials
occupy consecutive slots in manager order; a material's own texture
`CDib` consumes the following slot. The first user material's slot equals
the `CMaterial` class slot + 1. Layer slot 0 always denotes the default
layer. (Byte-scanning readers that cannot track slots can fall back to
relative order — sorted distinct references zipped against file-order
materials — but slot arithmetic is exact on a full walk.)

## 9. UV mapping: CFaceTextureCoords [§4r, §4u, §4v]

A face painted with a texture in its default position writes **no**
`CFaceTextureCoords` (FTC) record — absence means identity mapping. A
repositioned/rotated/skewed/pinned texture attaches an FTC to the face
through the face's attribute container (§6.3).

### 9.1 Record body (schema 4) [§4u]

```
preamble
u32                  (0 in every observed instance)
24 × f64             two blocks: FRONT k0..k11, BACK k12..k23.
                     k0..k8 (resp. k12..k20) = a row-major 3×3 projective
                     matrix; k9..k11 (k21..k23) = 3 trailing values (zero
                     on user faces; semantics unknown)
u32 front pin count, then count × pin
u32 back  pin count, then count × pin
u32, u32             flags (pattern tracks which sides are painted;
                     semantics unconfirmed)
```

One pin is 4 × f64, all inches:
`(anchor_u, anchor_v, face_x, face_y)` — a texture-space point and the
face-local point it is pinned to. The pins and the matrix are two
encodings of the same mapping (the matrix reproduces every observed pin
list to ≤ 1e-13"); after a pin-fit the stored anchors are re-normalized
so the anchor span equals the face span.

### 9.2 The face-local frame [§4v]

Both sides share one 2D frame derived from the face's **front** plane
normal `n`:

```
if |n_z| = 1:  v = (0, 1, 0)                    (horizontal faces)
else:          v = normalize(Z − (Z·n)·n)       (world Z projected onto the plane)
u = v × n
local(P) = (P·u, P·v)      for P on the face, in inches
```

### 9.3 The UV formula [§4v]

The 3×3 block `K` maps texture space (inches) to the face-local frame as
a row-vector homogeneous product: `[t_u, t_v, 1] · K = [x', y', w]`,
face-local point = `(x'/w, y'/w)`. The projective (third-column) terms
are real — pinned arrangements use them.

Per-vertex UV for a painted side, with the material's applied size
(W, H) inches (§8.1):

```
[x_l, y_l, 1] · K⁻¹ = [t_u', t_v', w']
uv = ( t_u'/w' / W ,  t_v'/w' / H )
```

Faces without an FTC use the identity: `uv = (x_l / W, y_l / H)`.

A pure "scale by N×" applied through fixed pins is stored as the
material's applied size, not as an FTC — scale-via-size and warp-via-FTC
are separate mechanisms.

*Informative:* SketchUp's own COLLADA exporter bakes projectively-pinned
textures into per-face images with bounding-box UVs, and its second
(unpainted-side) copy of each face mirrors the painted side's texture in
U. These are exporter behaviors, not format semantics.

### 9.4 Instance material inheritance [§4v]

A material slot in a group/instance drawbase (§5.2) is a material painted
on the instance. Faces below it whose own material is the default render
with the nearest ancestor's instance material (a face's own non-default
material always wins). UVs for such faces use the identity mapping under
the inherited material's applied size unless the face carries its own
FTC.

## 10. Annotations and document entities

### 10.1 CDimensionLinear (schema 6) + CSkFont (schema 1) [§4k]

Brackets give the 2026 field (§16.13) with the same value in the
conversions of `feature-pack` and `house-plus` (three dimensions):

```
CDimensionLinear:
  preamble + drawbase   (the material slot binds the dimension's material)
  utf16                 text override (empty = automatic text)   [0x59d9]
  <font>                CSkFont object (inline first, back-ref thereafter) [0x59da]
  u8                    0                                        [0x59db] (candidate)
  u32                   1                                        [0x59dc] (candidate)
  2 × <anchor>          start, end                               [0x5bcd, 0x5bce]
  3 f64                 plane normal                             [0x5bcf]
  3 f64                 x axis                                   [0x5bd0]
  u32                   1 or 2                                   [0x5bd1]
  f64                   offset of the dimension line, inches     [0x5bd2]
  f64                   0                                        [0x5bd3]
  u32                   text position                            [0x5bd4]

<anchor>:
  u32                   kind (2 = a vertex)                      [0x5209]
  u32                   4 in every observed anchor
  3 f64                 point                                    [0x520a]
  object reference      the anchored entity                      [0x53fd]
  u16                   0 in every observed anchor
  u32 n, n references   instance path                            [0x53fe]
  u32                   0 in every observed anchor               [0x520c] (candidate)

CSkFont:
  preamble (mask 0 on authored files; third-party dimension fonts carry pids)
  utf16 name ("Tahoma")                                          [0x5015]
  u8, u8                bold, italic (0 in every file)           [0x5016, 0x5017] (candidate order)
  u32                   size in points                           [0x5018]
  u8                    0                                        [0x5019]
  f64                   10.0                                     [0x501a]
```

The references in anchors and instance paths are object references
(§3.1) and escalate to `7F FF` + u32 on large store maps (§11). The
fields observed with only one value (arrow, alignment, bold, italic)
are paired by order and are **candidates**. Dimensions attach to model
geometry through their anchors and store no endpoint coordinates.

### 10.2 CText (schema 9) [§4k]

Leader text and screen text share one layout (`text.skp`,
`feature-pack`, `house-plus`; brackets as in §10.1):

```
preamble + drawbase                                              [0x07d0]
<font>            CSkFont object (inline or back-ref)            [0x55f9]
2 f64             screen position as fractions of the view       [0x55f2, 0x55f3]
<anchor>          as §10.1; kind 0 for screen text               [0x55f4]
3 f64             leader offset                                  [0x55f5]
3 f64             view direction when placed                     [0x55f7]
u32               leader: 0 none, 1 view-based, 2 pushpin         [0x55fd]
u32               1                                              [0x55fb] (candidate)
u8, u8            1, 0                                           [0x55f6, 0x55f8] (candidate)
u32               arrow (3 closed in every file)                 [0x55fa] (candidate)
u8                leader shown (1 in every file)                 [0x55fc] (candidate)
utf16             the text content                               [0x55f1]
5 bytes           zeros (u32 hidden-leader flag [0x55fe] and a u8, candidate)
```

The 11 bytes before the string (`01 00 00 00 01 00 03 00 00 00 01`)
were constant in every observed text, and the Rust reader still locates
the string by them.

### 10.3 Construction geometry [§4l, §4w]

**CConstructionPoint** (schema 0): preamble + drawbase + 3 × f64 position
+ 3 × f64 reference (tape-measure anchor) point + u8 (`01` in both corpus
instances; 0 in a third-party bathroom model's mid-definition points,
whose next entity's class-ref pins the extent). In feature-pack.skp the
section plane's new-class record follows the u8 directly;
construction-point.skp's point is the last root entity, so a u32 reading
there only swallows root-tail zeros.

**CConstructionLine** (schema 1, guides): preamble + drawbase + 3 × f64
anchor + 3 × f64 unit direction + 2 × f64 line-parameter bounds (±1.0e30
exactly = the infinite-guide sentinel) + 4 zero bytes (feature-pack.skp:
the guide point's new-class record follows directly).

**CSectionPlane** (schema 2): preamble + drawbase + 4 × f64 plane
(A, B, C, D), nothing after; the plane is in the owning entity list's
local frame (unit normal + offset, inches). The record carries no active
flag — an active and a deactivated plane serialize identically
(section-plane-deactivated.skp); which root plane cuts is the root tail's
first pointer (§4l). feature-pack.skp's plane
(0, 1, 0, 0) — the front face of the authored box — is followed directly
by a CText new-class record; in section-plane.skp and house-plus.skp the
plane is the last root entity and the four zero bytes behind it open the
root tail; a third-party house's mid-definition planes are followed
directly by the definition tail.

### 10.4 CImage (schema 1) [§4l]

Preamble + drawbase + layer object pointer (a back-ref word; the `7F FF`
+ u32 escape on large maps) + 13 × f64 placement block (inch-per-pixel
scale plus pose, instance-transform-shaped) + utf16 source path + 16-byte
GUID, nothing after: in house-plus.skp the next root entity's new-class
record starts immediately behind the GUID. The pixel data lives in the
document's embedded `CDib` pool, not inline; the linkage is undecoded.

### 10.5 Attribute dictionaries [§4s, §4t]

**CAttributeContainer** (schema 0): preamble + child objects until a null
tag. **CAttributeNamed** (schema 1): preamble + 4 bytes + utf16
dictionary name + entries `[utf16 key + type:u8 + value]` terminated by
an empty-string key, + u32.

Value types: `0x00` nil (no bytes), `0x04` i32, `0x06` f64, `0x07` bool
(u8), `0x0A` utf16 string, `0x0B` typed array (u32 count, then per
element a type u8 + value, recursive), `0x0C` an 8-byte scalar, `0x11` a
3 × f64 point/vector (a plugin dictionary on a third-party house).

Dictionaries carry model options (units, snap settings), geo-location,
and dynamic-component parameters.

### 10.6 View records

**CThumbnail** (schema 1): preamble + CCamera object + nullable CDib
(the preview image); the document's first CThumbnail is the preview and
its camera is the **saved view** (it matches the COLLADA export's
"Last_Saved_SketchUp_View" camera). **CCamera** (schema 5): no preamble — 137 raw bytes,
u16 (1), utf16 description, 33-byte tail. Each field is the 2026
camera record in brackets (§16.9):

```
body +0    3 f64   eye            [0x34bd]      tail +0   f64  [0x34c9]
     +24   3 f64   target         [0x34be]           +8   u8   two-point perspective [0x34ca]
     +48   3 f64   up             [0x34bf]           +9   f64  [0x34cb] (1.0)
     +72   f64     [0x34c0] (1.0)                    +17  f64  [0x34cc]
     +80   f64     [0x34c1] (1000.0)                 +25  f64  [0x34cd]
     +88   u8      perspective    [0x34c2]
     +89   f64     field of view, degrees [0x34c4]
     +97   f64     parallel-projection view height [0x34c3]
     +105  32 bytes (zero in every observed camera)
```

SketchUp Make 2017 pairs switching to parallel projection and to
two-point perspective pin `+88`, `+97` and tail `+8`; the document
camera of every corpus model decodes to its 2026 conversion's
values field for field. Scene records (`CViewPage`) are in §10.10.
**CRelationship** (schema 0): see §7.1 — a chain node in definition
tails [§4t].

### 10.7 Rendering options

The document's display settings (the `CRenderingOptions` object,
**candidate** attribution) follow its current-view CCamera (§10.6):
after the camera's 33-byte tail come 3 bytes (`00 00 00` in every
observed file) and then fixed fields at these offsets from that point
(+0). Each field is the 2026 rendering-option record in brackets
(§16.10), which gives its meaning:

| offset | type | field | offset | type | field |
|---|---|---|---|---|---|
| +0 | u32 | face style [`0x733d`] | +109 | u32 | [`0x7364`] |
| +4 | u8 | X-ray [`0x733e`] | +113 | 4 bytes | undecoded |
| +5 | u8 | material transparency [`0x733f`] | +117 | u8 | model axes [`0x7343`] |
| +6 | u8 | jitter [`0x734b`] | +118, +119 | u8 | [`0x7344`], [`0x7345`] |
| +7 | u32 | edges [`0x7341`] | +120 | u8 | guides hidden [`0x7346`] |
| +11 | RGBA | background [`0x7357`] | +121 | RGBA | sky [`0x7365`] |
| +15 | RGBA | edge colour [`0x7358`] | +125 | u32 | [`0x7366`] |
| +19 | RGBA | selected [`0x7359`] | +129 | RGBA | ground [`0x7367`] |
| +23 | RGBA | guides colour [`0x735b`] | +133 | u8 | sky shown [`0x7368`] |
| +27 | u8 | shadows displayed [`0x6599`, §16.11] | +134 | u8 | ground shown [`0x7369`] |
| +28 | u8 | component axes [`0x7349`] | | | |
| +29 | u8 | colour by layer [`0x7347`] | +135 | u8 | ground from below [`0x736a`] |
| +30 | u8 | textures [`0x7340`] | +136 | u32 | ground transparency [`0x736b`] |
| +31 | u32 | edge colour mode [`0x7348`] | +140 | RGBA | active section [`0x7370`] |
| +35 / +36 | u8 / u32 | extension / length [`0x734d` / `0x734e`] | +144 | RGBA | inactive section [`0x7371`] |
| +40 / +41 | u8 / u32 | profiles / width [`0x734f` / `0x7350`] | +148 | RGBA | section cut [`0x7372`] |
| +45 / +46 | u8 / u32 | depth cue / width [`0x7351` / `0x7352`] | +152 | u32 | section cut width [`0x7374`] |
| +50 / +51 | u8 / u32 | endpoints / length [`0x7353` / `0x7354`] | +156 | u32 | section bits [`0x7375`] |
| +55 | 1 byte | undecoded | +160 | u32 | transparency quality [`0x7377`] |
| +56 | u8 | hidden geometry [`0x7380` and `0x7381`] | +164 | u8 | [`0x7379`] |
| +57 | 4 bytes | undecoded | +165 | f64 | [`0x737a`] |
| +61 | RGBA | front face [`0x735c`] | +173 | u8 | [`0x737b`] |
| +65 | RGBA | back face [`0x735d`] | +174 | RGBA | locked [`0x735a`] |
| +69 | f64 | fade rest of model [`0x736c`] | +178 | u8 | watermarks [`0x735e`] |
| +77 | f64 | fade similar components [`0x736d`] | +179 | f64 | [`0x7378`] |
| +85 | u8 | hide rest of model [`0x736e`] | +187 | u8 | back edges [`0x7356`] |
| +86 | u8 | hide similar components [`0x736f`] | +188 / +189 | u8 / f64 | background photo / opacity [`0x737c` / `0x737d`] |
| +87 | u8 | fog [`0x735f`] | +197 / +198 | u8 / f64 | foreground photo / opacity [`0x737e` / `0x737f`] |
| +88 | RGBA | fog colour [`0x7360`] | +206 | 8 bytes | undecoded (`00 00 00 00 01 00 00 00`) |
| +92 | u8 | fog uses background [`0x7361`] | | | |
| +93 / +101 | f64 | fog start / end [`0x7362` / `0x7363`] | | | |

Evidence: each position was found by a one-change pair saved in
SketchUp Make 2017 (Styles, Fog, View ▸ Component Edit, Model Info ▸
Components), and all 64 fields decode to the same values as the
2026 records in the web app's conversion of every such pair (54
one-change pairs) and of `box`, `feature-pack`, `house-plus` and
`theater`. The section-fill settings
(`0x7373`, `0x7376`) have no 2017 counterpart.

### 10.8 Styles and watermarks

A **CSkpStyle** (schema 1) body is a keyed item list, the 2017 form of
the 2026 style document (§16.14):

```
3 bytes          00 00 00 in every observed file
16 bytes         GUID (carried over as 0x6b6d)
utf16            empty in every observed file
u32              3 in every observed file
utf16            name
utf16            description
u32 N            item count (47 in every observed style)
N × item         u32 item id | u32 value count | value × count
value            u32 type | payload: 1 → u8, 4 → u32, 7 → f64
```

Item ids and value types are the 2026 `<sty:item id>` and
`<t:variant type>` of the same setting; colours are RGBA bytes. The
2026 document adds items 1016, 4007, 7015–7018 and 8100–8107.
Item 5001 (the watermark list) holds object references instead of typed
values: each non-null reference is a CWatermark, and a single null
reference marks the model. The entries before it are background
watermarks and the entries after it are overlays. The value count
includes the null, and it is 0 when the style has no watermarks. The
2026 list (`<wmlist>`, type 13) keeps the same order and writes
the null as `<screenimage name="<MODEL SPACE>"/>`.

The style manager stores, in order:

- a u32 style count and that many CSkpStyle records;
- a reference to the active style;
- a CSkpStyle holding the current settings. Its GUID and name are the
  active style's, and it becomes the 2026 `0x697b` (`<name>_1`);
- a u32 that is 1 while the current settings differ from the saved
  active style (an edit or an added watermark) and 0 once the style is
  updated.

A style that a scene (`CViewPage`) references first appears inline in
that scene record (`feature-pack`, `house-plus`), and the manager list
refers back to it.

A **CWatermark** (schema 1) body; brackets give the 2026
`<screenimage>` attribute and `0x2ee0` record (§16.14):

```
u32              0 in every observed file
utf16            source file path          [info_filename, 0x2ee1]
u32              UNIX time the image was loaded [info_time, 0x2ee3]
u32              position: 3×3 grid, row-major from top left, 4 = centre [position, 0x2ee5]
u8               tiled                     [tiled, 0x2ee7]
u8               stretched                 [stretched, 0x2ee8]
u8               keep aspect ratio         [maintainAR, 0x2ee9]
u8               background                [background, 0x2eea]
u8               create mask               [intensForAlpha, 0x2eec]
f64              blend                     [alphaScale, 0x2eed]
f64              scale, 0–1 (the wizard's 0–100 slider) [scale, 0x2eeb]
utf16            name                      [name, 0x2ee4]
CDib             the image (§8.2); watermarks of the same image share one object
```

Tiled and stretched both 0 means positioned. The conversion derives
the 2026 fitting type (`0x2eee`: 0 tiled, 1 stretched,
2 positioned) and stretch type (`0x2eef`: 1 = stretched with its aspect
ratio kept), and it clears keep-aspect-ratio outside stretched mode,
where the wizard hides the control.

Evidence: SketchUp Make 2017 pairs that create a style, update a style,
and add watermarks through the wizard, with one setting changed in each
(background, blend, mask, aspect ratio, tiled and positioned scale, and
grid positions 1, 3, 5 and 9). One file holds two overlays and another
holds an overlay and a background. The style-creation and style-update
pairs pin the manager's layout. The web app converted 85 SketchUp Make
2017 pairs (every pair behind §10.7–§10.10) plus `box`,
`feature-pack`, `house-plus` and `theater`. In every conversion, the
current-settings record and the active style decode to both 2026
style documents value for value (8,188 values plus each watermark
list), because the conversion copies the current settings into the
saved style. The watermark fields equal the converted
`<screenimage>` attributes in all 14 watermark pairs.
`tools/skptool.py styles` prints these records.

### 10.9 Shadow info

The document's shadow settings, and those of each scene that saves
them (§10.10), are one fixed record. Brackets give the 2026 shadow
info field (§16.11):

```
3 bytes          00 00 00 in every observed file
u32              date and time, UNIX seconds      [0x6591]
u8               0 in every observed file         [0x6592]
utf16            city                             [0x6593]
utf16            country                          [0x6594]
f64              longitude                        [0x6595]
f64              latitude                         [0x6596]
f64              time-zone offset, hours          [0x6597]
3 f64            (0, 1, 0) in every observed file [0x6598]
u8               0 in every observed file         [0x659a]
u8               shadows on faces                 [0x659b]
u8               shadows on the ground            [0x659c]
u8               shadows from edges               [0x659d]
u32              light, 0–100                     [0x659e]
u32              dark, 0–100                      [0x659f]
u8               use the sun for shading          [0x65a0]
u8               0 in every observed file
```

Whether shadows are displayed is rendering-options byte +27 (§10.7),
not part of this record. The document's own record is the first in the
file.

Evidence: SketchUp Make 2017 pairs from the Shadows panel (date, time,
light, dark, use sun for shading, and with shadows displayed: on faces,
on the ground, from edges) and View ▸ Shadows. Across the web app's
conversion of every 2017 pair and of `box`, `feature-pack`,
`house-plus` and `theater` (89 files), each field equals its 2026
counterpart (1,424 values). The `[0x659a]` and `[0x6592]` pairings
follow field order and are **candidates**; both are 0 everywhere.

### 10.10 Scenes

A scene (**CViewPage**, schema 12) body stores only the properties the
scene saves. The u32 flags select them:

```
3 bytes          00 00 00 in every observed file
utf16            name                               [0x6f55]
utf16            description                        [0x6f56]
u32              properties saved                   [0x7149] (bits as §16.15)
if bit 1:  CCamera object (§10.6)                   [0x714a]
if bit 2:  3 bytes | the 206 rendering fields at §10.7's +0…+205 [0x714d]
           | CSkpStyle reference (§10.8)            [0x714c]
if bit 4:  shadow info (§10.9)                      [0x714e]
if bit 8:  axes, 110 bytes                          [0x714f]
if bit 16: u32 n | n object references             [0x714b]
if bit 32: u32 n | n object references             [0x7150]
if bit 64: u32 n | n object references             [0x7151]
u8               included in animation              [0x7152]
f64              −1 in every observed file          [0x7154]
f64              −1 in every observed file          [0x7155]
u8, u8           0 in every observed file
u8, u8           has a thumbnail; thumbnail inline  [0x7157]
if 01 01:  u8 (1) | u32 length | PNG image          [0x7158]
if 01 00:  object pointer to a thumbnail stored earlier (a third-party
           house; big-tag back-references)
```

The axes are a drawing element: a preamble with a null attribute
pointer and pid mask 0, then a drawbase (§5.2). Next come the origin and
the x, y and z axis directions (4 × 3 f64, `0x4651`–`0x4654`), and a u8
that is 1 in every file. Bits 128 and above add no fields. The three
lists match the 2026 id lists in length. The bit-16 list holds the
hidden entities (**candidate**), the bit-32 list the hidden layers
(references to CLayer objects: 1, 3, 3 and 4 in a third-party bathroom
model's four scenes, matching its conversion's `0x7150` lists), and the
bit-64 list the active section planes. The scenes added through the Scenes
panel in these pairs save bits 1–64 only and store no thumbnail. The
2026 scene record omits the same fields when their property is not
saved.

Evidence: SketchUp Make 2017 pairs from the Scenes panel. Each clears
one more property than the previous one (camera, shadow, active section
planes, axes, animation, style and fog; the panel keeps the choices for
the next new scene). Further pairs add a scene without its
hidden-geometry or visible-layer property, add one with a description,
and add one with shadows displayed. With the scenes of `feature-pack`
and `house-plus`, every field of all 13 scenes decodes to the web app's
conversion (985 checks), including the hidden-entity and
section-plane lists.

### 10.11 Document tail and annotation defaults

The document's shadow record (§10.9) is followed by a u16 (0 in every
observed file), the u32 scene count and the scene records (§10.10).
After them — or directly after the count when the model has none — the
document continues:

```
object ref       the selected scene (null when there is none)
110 bytes        the model axes, laid out as a scene's (§10.10)    [0x01fc]
70 bytes         0 in every observed file
object           CSkFont: the default dimension font                [0x5fb5]
61 bytes         dimension defaults                                 [0x5fb4]
object ref       CSkFont: the default leader-text font              [0x57e5]
22 bytes         text defaults                                      [0x57e4]
object ref       CSkFont: the default screen-text font              [0x57e6]
```

The font objects are the usual archive objects: an inline CSkFont on
first appearance, a back-reference afterwards, or null. A file that never
held a text carries null text-font references; the conversion points
them at the dimension font. Within the fixed blocks:

| block | offset | type | field |
|---|---|---|---|
| dimension defaults | +0 | u8 | text aligned to the dimension line [`0x5fb6`] |
| | +14 | u32 | arrow: 0 none, 1 slash, 2 dot, 3 closed, 4 open [`0x5fbb`] |
| | +46 | RGBA | colour [`0x5fc4`] |
| | +54 | u32 | text position: 0 above, 1 centred, 3 below [`0x5fc6`] |
| text defaults | +9 | u32 | arrow, same values [`0x57e7`] |
| | +14 | RGBA | leader text colour [`0x57ec`] |
| | +18 | RGBA | screen text colour [`0x57ed`] |

The other bytes of both blocks are undecoded. Separately, the byte 12
after the end of the style manager (§10.8: its unsaved-changes flag,
eight zero bytes, then this one) is Model Info ▸ Rendering's
anti-aliased textures [`0x020c`]. Model Info ▸ Animation and
Geo-location write attribute dictionaries (§10.5): `ShowTransition`,
`TransitionTime`, `SlideTime`, `LoopSlideshow`, and `UsesGeoReferencing`
with `Latitude` and `Longitude` in degrees (§16.12).

Evidence: SketchUp Make 2017 pairs from Model Info's Dimensions (align
to the line, colour, both endpoints, text above and centred), Text (both
endpoints, both colours), Rendering, Components, Animation and
Geo-location pages, each converted by the web app; every decoded value
equals its 2026 record. The text endpoint "Closed Arrow" is the
default (arrow 3), so that pair changes nothing.

## 11. Scale escalations [§4t]

Behaviors that only appear once the store map or pid space grows:

- Object/class references to slots ≥ 0x7FFF use the `FF 7F` big-tag
  escape + u32 (§3.1).
- The definition-list prelude's `decl` field is u16 with `0x7FFF`
  escalating to u32 (§5.3).
- String records escalate their length field at 255 characters (§2.1).
- Pids beyond 0xFFFF store three or four bytes under the presence mask
  (§5.1).

A reader validated only on small files will pass every minimal test and
still fail on real models unless these four paths are exercised.

## 12. Template content [§4m]

Every save from the default 2017 template seeds ~1,400 entities (the
template's scale-figure component) before any user drawing. Template
content is ordinary model content: it parses identically, its definitions
simply have no placed instances when unused. Parsers must not bake in
template-specific constants (pid baselines, fixed class tags, fixed slot
bases).

## 13. Persistent-id space

Pids are u32 in practice (up to 24 bits observed), allocated globally
with jumps (§5.1). Consecutive drawn entities usually get consecutive
pids, which minimal-pair authoring exploits, but nothing in the format
guarantees it.

## 14. Version scope

- **2013–2016**: same outer container (header records, GUID, CArchive
  stream); different class schemas and body layouts. Header
  identification works; body decoding per this spec does not apply.
- **2017 (v17.x)**: §2–§13 and §15.
- **2026 (`{26.x}`)**: the container every current release writes — a
  ZIP archive after the leading string records, holding a self-describing
  record tree; §16 specifies it (`corpus/2026/`). Releases 2018–2025 are
  unobserved; a file whose string records are followed by a ZIP archive
  is read as §16.

## 15. Known unknowns

None of these block reading geometry — each is either positionally
skipped with exact extent or lies outside the walk. Anchors refer to the
owning section.

Unknown bytes inside otherwise-exact records:

- CFaceTextureCoords: the leading u32; the per-side trailing f64 triples;
  the two flag u32s (§9.1).
- CComponentDefinition: the 22-byte base; the u32 before the
  relationship pointer; the f32 after the tail timestamp and the bytes
  after the behaviour (§7.1).
- CCamera: body bytes 105–136 and the fields marked undecoded in §16.9
  (§10.6).
- CLayer: the two bytes after the internal name; the second pid field's
  role (§8.3).
- Drawbase bytes [3..5) and [7] (§5.2).
- CCurve's u8; CRelationship's two object pointers (§6.4, §7.1).
- CConstructionLine's 4-byte tail; CConstructionPoint's u8 (§10.3).
- CDimensionLinear and CText: the constant fields and single-valued
  candidates marked in §10.1 and §10.2; the anchor's u32 (4) and u16.
- Materials: the byte between the two textured colours and the empty
  string; the texture's attribute-container pointer beyond the one
  plugin dictionary observed (§8.1).
- CDib subtypes other than 1 and 4, if any exist (§8.2).
- CSkpStyle: the 3 leading bytes, the empty string and the u32 (3);
  CWatermark: the leading u32 (0) — constant in every file (§10.8).
- Rendering options: the undecoded bytes of §10.7's table.
- Shadow info: the fields that are constant in every file (§10.9).
- CViewPage: the leading 3 bytes, the two u8 before the thumbnail flags,
  the u8 after them, the axes' trailing u8, and the two f64 (§10.10).
- The document tail: the 70 zero bytes and the undecoded bytes of the
  annotation defaults (§10.11).

CSectionPlane and CImage extents are pinned by a directly following
record (feature-pack.skp, house-plus.skp); the CImage → `CDib` linkage
remains undecoded (§10.3, §10.4).

Never observed as entity-list elements: CDimensionRadial, CPolyline3d,
CComponentBehavior, CComponent, and the pre-model manager records
(CRenderingOptions, CShadowInfo, style and page managers) — scene records
and option dictionaries are recoverable by signature scans without them.

Narrow-oracle semantics: the horizontal-face frame rule (§9.2) is
validated on exact ±Z normals; behavior within ~1e-9 of ±Z is untested.
Instance-material inheritance beyond observed child-wins cases is
unproven.

## 16. The SketchUp 2026 container

Evidence: the files of `corpus/2026/` and
`corpus/third-party/theater-2026.skp`, each the SketchUp web app's
conversion of the same-named 2017 file (`{26.2.0}`), plus two third-party
production models converted the same way and kept outside the
repository. Persistent ids survive the conversion (renumbered only where
the 2017 file held duplicates), so every field below is pinned to the
value the 2017 decode gives for the same entity. `tools/skp26.py` is the
instrument; `crates/openskp/tests/read26.rs` checks every corpus pair
equivalent, and the COLLADA-oracle tests run on the 2026 files as well as
on their originals.

Settings the corpus files do not vary are pinned by **one-change pairs**:
`box.skp` saved twice with exactly one setting changed, either edited in
the SketchUp web app and downloaded, or edited in SketchUp Make 2017
(Save A Copy) and converted by the web app; each pair is diffed record
by record. A style setting also appears in the style document
(`styles/*/style.xml`) as `<sty:item id="N">`; tables give that id in
brackets. Colours are 4 bytes R, G, B, A (`12 34 56 FF` for RGB
18, 52, 86); `style.xml` writes the same four bytes as a signed i32.

### 16.1 Container

```
0x00  FF FE FF 0E + "SketchUp Model"     string record (§2.1)
0x20  FF FE FF 08 + "{26.2.0}"           version string record
0x34  13 bytes                            56 46 46 08 00 01 00 11 00 + 4 per-file bytes (undecoded)
0x41  ZIP archive                         PK\x03\x04 …, EOCD at the end
```

ZIP offsets count from the archive's first byte (0x41). Entries are
stored or DEFLATE-compressed with valid CRC-32s:

| entry | contents |
|---|---|
| `model.dat` | the model: a record tree (§16.2) |
| `meta/meta.dat` | the same record format (§16.2) with one-byte tag values: a `0x64` container holding `0x75` the version string (`26.2.0`), `0x76` u16 (26), `0x77`/`0x73`/`0x74` u16 (1, 1, 17), `0x66` the model GUID (16 bytes, new on every save), and `0x67` the archive entry table |
| `materials/<name>/material.xml` | one per material and per layer (`Layer_<name>`); §16.6 |
| `materials/<name>/<image>` | texture images, byte-identical to the 2017 embedded images |
| `thumbnails/<definition>.png`, `meta/*_thumbnail.png`, `scene_thumbnails/*.png` | preview images |
| `styles/*/style.xml`, `watermarks/*`, `classifications/*.skc` | display and classification assets |

The 2017 header's per-model GUID (§2) is not carried over: the
conversion writes a new GUID into `meta/meta.dat` (`0x66`), and the 2017
value appears nowhere in the archive.

### 16.2 The record tree

`model.dat` is a single record; every record is

```
u16 tag | u32 length | payload[length]
```

and a container's payload is exactly a sequence of child records. Tags
are **type-scoped**: one tag has one meaning wherever it appears, so the
structure is self-describing and a reader skips unknown records by
length. Leaf payloads are not self-describing — an all-zero f64 triple
also tiles as four empty tag-0 records — so a reader descends only into
tags known to be containers.

- **Ids**: an id is stored as a little-endian unsigned integer exactly
  as wide as its value needs (1–3 bytes observed; theater-2026 reaches
  3). This replaces every 2017 scale escalation (§11).
- **Id lists** (`0x138e`): repeated `<width:u8> <id>`.
- **Strings**: raw UTF-8, length from the record.
- **Numbers**: `f64` little-endian; transforms are 13 f64 exactly as in
  2017 (§7.3), coordinates f64 inches.

### 16.3 Top-level sections (children of `0x01f4`)

| tag | contents |
|---|---|
| `0x0209` | model attribute dictionaries (§16.8) |
| `0x0200` | option sets (§16.12) |
| `0x01f5` | `0x03e8 > 0x03e9`: the next persistent id to allocate |
| `0x01fa` | camera `0x34bc` (§16.9) |
| `0x01fb` | rendering options `0x733c` (§16.10) |
| `0x0204` | shadow info `0x6590` (§16.11) |
| `0x01fc` | `0x4650`: drawing element and four f64 triples `0x4651`–`0x4654` holding (0,0,0), (1,0,0), (0,1,0), (0,0,1) in every observed file — the model axes (**candidate**; never moved in the evidence) |
| `0x0208` | line styles `0x4074 > 0x4075 > 0x4076` ×N (§16.14) |
| `0x01f7` | materials: `0x30d4 > 0x30d5 > 0x32c8` ×N, and `0x30d6` the current paint material id (§16.6) |
| `0x01f8` | layers: `0x3a98 > 0x3a99 > 0x3c8c` ×N; `0x3a9a` the current layer id; `0x3a9b > 0x3e80` the layer-folder root (§16.6) |
| `0x01fd` | fonts: `0x4e20 > 0x4e21 > 0x5014` ×N (§16.13) |
| `0x01fe` | text defaults `0x57e4` (§16.13) |
| `0x01ff` | dimension defaults `0x5fb4` (§16.13) |
| `0x0203` | watermarks `0x2cec > 0x2ced > 0x2ee0` ×N (§16.14) |
| `0x01f9` | definitions: `0x1770 > 0x1771 > 0x157c` ×N (§16.5) |
| `0x01f6` | the model's own entities: one entity container `0x1388` (§16.4) |
| `0x0206` | styles `0x6978` (§16.14) |
| `0x0207` | scenes `0x6d60` (§16.15) |
| `0x0205` | classification: `0x6784` {`0x6785` source path, `0x6786` name, `0x6787` archive entry} |
| `0x020e` | a behaviour record `0x1b58` for the model itself (all fields 0 in every observed file; §16.5) |
| `0x020c` | u8: anti-aliased textures (Model Info ▸ Rendering; the 2017 flag is in §10.11) |
| `0x0214` (u8 0), `0x020f` (u32 0), `0x0210 > 0x7918`, `0x0213 > 0x7d64` | constant in every observed file (undecoded) |
| `0x020d` | u32: 1 (`box`, `theater`), 2 (`feature-pack`), 3 (`house-plus`); not a style or scene count (undecoded) |
| `0x0063` (u32 0), `0x0201` (empty), `0x020a` (the string `meta/meta.dat`) | constant in every observed file |

### 16.4 Entities

Common headers:

| tag | meaning |
|---|---|
| `0x05dc` | entity base: persistent id `0x05de`; attribute container `0x05dd` when present |
| `0x07d0` | drawing element: entity base, material ref `0x07d1`, layer ref `0x07d2`, flags byte `0x07d3` |

Material and layer refs are the ids of the material/layer records; an
absent layer ref is the default layer. For a face, `0x07d1` is the
**front** material. Flags `0x07d3`: `0x01` hidden, `0x02` casts
shadows, `0x04` receives shadows, `0x08` soft, `0x10` smooth, `0x20`
locked; the record is omitted when the byte would be 0. Hidden, soft and
smooth are pinned across 33,041 theater edges and 13,268 faces; casts,
receives and locked by one-change pairs on a component instance (web-app
edits of `box.skp`).

An entity container `0x1388` (the model's, or a definition's) holds a
drawing-element header (its id is the id instances refer to) and pools:

| pool | item | fields |
|---|---|---|
| `0x1389` vertices | `0x09c4` | entity base, point `0x09c5` (3 f64) |
| `0x138a` edges | `0x0bb8` | drawing element, start vertex `0x0bb9`, end vertex `0x0bba`, owning curve `0x0bbb` (when a curve member) |
| `0x138b` faces | `0x0dac` | drawing element, plane `0x0dad` (a, b, c, d), loops `0x0dae`, back material `0x0daf` |
| `0x138c` components | `0x1964` | instance (§16.5) |
| `0x138d` groups | `0x1d4c` | wraps one `0x1964` instance |
| `0x1390` images | `0x1f40` | wraps one `0x1964` placement of an image definition |
| `0x1391` guide lines | `0x4269` | base `0x4268`, 8 f64 `0x426a` (point, unit direction, parameter bounds) |
| `0x1392` guide points | `0x426c` | base `0x4268`, point `0x426d`, tape-measure anchor `0x426e` |
| `0x1393` section planes | `0x445c` | drawing element, plane `0x445d` (4 f64), name `0x445e`, symbol `0x445f` |
| `0x1394` | — | id of the active section plane (present only while one is active; read as `GeometryRun::active_section`) |
| `0x1396` curves | `0x4a38` | entity base, member-edge count `0x4a39` (sums equal the edges carrying `0x0bbb`), … |
| `0x1397` arc curves | `0x4c2c` | curve `0x4a38`, arc frame `0x4c2d` (16 f64) |
| `0x1398` texts | `0x55f0` | §16.13 |
| `0x1399` dimensions | `0x5bcc` | §16.13 |
| `0x138e` | — | id list of the container's top-level entities, in order |
| `0x13a0` | — | the container's bounding box: min and max corners (6 f64, inches) |
| `0x139f` | — | u8 bit set: bit 0 always set; bit 1 appeared when a text was first created, bit 2 when a section plane was first created (web-app pairs; `house-plus` holds 5, `feature-pack` 1) — meaning **undecoded** |
| `0x139b`, `0x139e` | — | `0x139b > 0x639f > 0x63a0` and an empty `0x139e`, constant in every observed file |

A face's loops: `0x0dae > 0x1194` (loop) `> 0x1195 > 0x0fa0` (edge use:
edge id `0x0fa1`, forward flag `0x0fa2`). The first loop is the outer
boundary. Each use contributes its edge's start vertex when forward,
else its end vertex; outer rings then match the 2017 mesh (and COLLADA)
winding exactly, and inner loops run opposite to it.

### 16.5 Definitions and instances

A definition `0x157c`:

| tag | meaning |
|---|---|
| `0x1388` | its entity container (§16.4); the container's id is what instances refer to |
| `0x157d` | GUID (16 bytes); rewritten whenever the definition's details are edited, and by the conversion for definitions whose entities were renumbered |
| `0x157e` / `0x157f` / `0x1580` | name / description / source path |
| `0x1581` | u32 UNIX time: the 2017 tail's timestamp (§7.1), equal in all 892 definitions of the converted corpus models |
| `0x1582` | u8 0/1 on every kind of definition: the 2017 u32 after the timestamp (§7.1); meaning undecoded |
| `0x1583` | kind: 0 component, 1 group, 2 image (pinned against the instances that place each of 949 definitions) |
| `0x1585` | thumbnail: `0x251c > 0x251d` camera (a `0x34bc` record, §16.9) and `0x251e > 0x2328` image {`0x2329` type (1 JPEG, 4 PNG — the 2017 `CDib` subtype, §8.2), `0x232a` archive path, `0x232c` JPEG quality (JPEG only)} |
| `0x1b58` | behaviour (below) |

Behaviour `0x1b58` (one-change pairs on a web-app component):

| tag | meaning |
|---|---|
| `0x1b5b` | u8: glues to a surface |
| `0x1b59` | u32 glue plane: 0 any, 1 horizontal, 2 vertical, 3 sloped (reset to 0 when gluing is turned off) |
| `0x1b5c` | u8: cuts an opening (kept when gluing is off) |
| `0x1b5d` | u8: always faces the camera (set on the stock "Chris" figure) |
| `0x1b5e` | u8: shadows face the sun |
| `0x1b5a` | u32: 0, except `0x7A` and `0x7F` on two dynamic components (theater, feature-pack) — the same value as the 2017 field after the behaviour bits (§7.1); meaning undecoded |

An instance `0x1964`: drawing element (material ref = the instance's
paint, inherited as in §9.4), name `0x1965`, transform `0x1966` (13 f64,
§7.3), definition ref `0x1967` (the definition container's id), GUID
`0x1968`.

### 16.6 Materials and layers

A material record `0x32c8`: entity base (the id faces refer to), name
`0x32cc` — the archive folder name, which turns 2017's unnamed `*N` into
`_N` — and:

| tag | meaning |
|---|---|
| `0x32ca` | u8: 1 when the material belongs to another entity (every layer colour, 166 of them; both image entities' `Image1`), else 0 |
| `0x32cb` | empty in every observed file |
| `0x32cd` | u32 JPEG quality (0–100), present only on JPEG-textured materials: 44, 53, 70, 80, 90, 92 and 99 observed, 70 the most common. Two library textures whose images use the standard scaled quantisation tables store exactly those tables' quality (92); imported images with standard tables of quality 75–100 store 70, so the field is a quality setting rather than a measurement of the image. It is the "u32 after JPEG texture payloads" of 2017, value for value (§8.1) |
| `0x32ce` | `0x2134`: archive path of the material's thumbnail |

Its `material.xml` holds the rest, on one `<mat:material>` element:

- `name`: the original name (`*N` kept);
- `colorRed`/`colorGreen`/`colorBlue`; opacity is `trans` when `useTrans`
  is `1`, else 1. No separate colour alpha is stored (a 2017 file's
  stored alpha byte is not carried over; §8.1);
- `hasTexture="1"` with `<mat:texture textureFilename xScale yScale
  avgColor>`: the applied size in inches is (`xScale`, `yScale`);
  `avgColor` packs the average colour as `0xAABBGGRR`. For a colourised
  texture (`type="2"`) the colour 2017 files store (and exporters emit)
  is the tint `colorRed/Green/Blue` instead;
- `<mat:image path>`: `./<file>` relative to the material's folder, or
  archive-rooted (`materials/<other>/<file>`) when another material owns
  the bytes;
- `colorizeType` (1 only on a library glass material), `workflow` (0),
  `pbrPromoState` (0; 2 on layer colours created in the web app) and a
  `<mat:pbrMR>` block (`enable_metalness`, `enable_roughness`,
  `enable_normal`, `enable_occlusion`, `roughness_texture_invert`,
  `metallicFactor`, `roughnessFactor`, `normalMapStyle`, `normalScale`,
  `occlusionStrength`, `baseColorFactor`) whose values are identical in
  all 323 observed materials.

`0x30d6` (in `0x01f7`) is the current paint material's id (web-app pair:
selecting a material for painting).

A layer record `0x3c8c`: entity base, name `0x3c8d`, hidden byte
`0x3c8e` (0 = visible; 40 hidden theater layers agree), `0x3c8f`
embedding a `0x32c8` material record whose XML (`Layer_<name>`) holds
the layer colour (web-app pair: changing a tag's colour rewrites only
that XML), and `0x3c90` (u32 0 in every observed file; the 2017 layer's trailing u32, §8.3). The layer-folder
root `0x3a9b > 0x3e80` {`0x3e81`, `0x3e83`, `0x3e84` empty; `0x3e82`,
`0x3e85` u8 0} never varies: folders and per-tag dash patterns are
paid-only features the free web app cannot create.

### 16.7 Texture placement

As in 2017 (§9), a face's texture placement lives in its attribute
container: `0x05dd > 0x36b1 > 0x36b2 > 0x2710`, with `0x2711` (front)
and `0x2712` (back) each holding `0x2713`: flags u32 `0x2714`, the 3×3
projective matrix `0x2715` (9 f64, §9.1), 3 f64 `0x2716`, and pins
`0x2717 > 0x2718` {anchor `0x2719` (2 f64), face point `0x271a` (2 f64)}.
Values are identical to the 2017 `CFaceTextureCoords` of the same face
(2,961 textured theater faces); the 2017 per-face flag pair is (front
`0x2714`, back `0x2714`).

### 16.8 Attributes

An attribute set `0x36b1 > 0x36b2 > 0x36b3` holds a dictionary: name
`0x36b4`, and `0x36b5` with alternating key `0x36b6` / typed value
`0x38a4` records. A typed value is one child: `0x38a7` i32, `0x38a9`
f64, `0x38aa` bool (1 byte), `0x38ad` string, `0x38ae` array. Option
sets (`0x61ac`) use the same values with key tag `0x61ad`.

### 16.9 Camera

`0x34bc` — the model's current view (`0x01fa`), each scene's camera
(`0x714a`), and each definition thumbnail's camera (`0x251c > 0x251d`):

| tag | type | meaning |
|---|---|---|
| `0x34bd` / `0x34be` / `0x34bf` | 3 f64 | eye, target, up (inches; eye and target match the COLLADA camera node) |
| `0x34c2` | u8 | perspective (0 = parallel projection) |
| `0x34c3` | f64 | parallel-projection view height, inches |
| `0x34c4` | f64 | field of view, degrees (the COLLADA `yfov`) |
| `0x34ca` | u8 | two-point perspective |
| `0x34c0` / `0x34c1` | f64 | 1.0 / 1000.0 in every observed camera (undecoded) |
| `0x34c5`, `0x34c9`, `0x34cc`, `0x34cd` | f64 | 0 (undecoded) |
| `0x34c6` / `0x34c7` / `0x34ce` | u8 | 1 / 0 / 0 (undecoded) |
| `0x34c8` | — | empty (undecoded) |
| `0x34cb` | f64 | 1.0 (undecoded) |

Evidence: web-app pairs switching to parallel projection (`0x34c2`
1 → 0, `0x34c3` 133 → 264.96), to two-point perspective and setting the
field of view (35 → 80).

### 16.10 Rendering options

`0x733c` — the model's display settings (`0x01fb`) and each scene's
(`0x714d`). One record per setting, in this order:

| tag | type | meaning [style item] |
|---|---|---|
| `0x733d` | u32 | face style: 0 wireframe, 1 hidden line, 2 shaded, 5 monochrome [2001] |
| `0x733e` | u8 | X-ray [2004] |
| `0x733f` | u8 | material transparency [2005] |
| `0x7340` | u8 | textures (shaded vs shaded with textures) [2007] |
| `0x7341` | u32 | edges [1000] |
| `0x7342` | u32 | 0; 1 under every sketchy-edge style (**candidate**: stroke-image edges) |
| `0x7343` | u8 | model axes [7008] |
| `0x7344`, `0x7345` | u8 | 1 (undecoded) |
| `0x7346` | u8 | guides **hidden** (inverted) [7012] |
| `0x7347` | u8 | colour by layer [7011] |
| `0x7348` | u32 | edge colour: 0 by material, 1 all same, 2 by axis [1002] |
| `0x7349` | u8 | component axes shown while editing (Model Info ▸ Components) |
| `0x7380` | u8 | hidden geometry [7017] |
| `0x7381` | u8 | hidden objects [7010, 7018]; a 2017 file's single "hidden geometry" setting converts to both |
| `0x734b` | u8 | jitter [1012] |
| `0x734c` | u8 | 1 (undecoded) |
| `0x734d` / `0x734e` | u8 / u32 | extension / its length [1004 / 1005] |
| `0x734f` / `0x7350` | u8 / u32 | profiles / their width [1006 / 1007] |
| `0x7351` / `0x7352` | u8 / u32 | depth cue / its width [1008 / 1009] |
| `0x7353` / `0x7354` | u8 / u32 | endpoints / their length [1010 / 1011] |
| `0x7355` | u8 | 0 (undecoded) |
| `0x7356` | u8 | back edges [1015] |
| `0x7357` | RGBA | background [4000] |
| `0x7358` | RGBA | edge colour [1014] |
| `0x7359` / `0x735a` / `0x735b` | RGBA | selected / locked / guides colour [7000 / 7001 / 7002] |
| `0x735c` / `0x735d` | RGBA | front / back face colour [2002 / 2003] |
| `0x735e` | u8 | watermarks [5000] |
| `0x735f` | u8 | fog |
| `0x7360` | RGBA | fog colour |
| `0x7361` | u8 | fog uses the background colour |
| `0x7362` / `0x7363` | f64 | fog start / end distance (−1 = unset) |
| `0x7364` | u32 | 1 (undecoded) |
| `0x7365` | RGBA | sky [4001] |
| `0x7366` | u32 | 0 (undecoded) |
| `0x7367` | RGBA | ground [4003] |
| `0x7368` / `0x7369` | u8 | sky / ground shown [4002 / 4004] |
| `0x736a` | u8 | ground visible from below [4006] |
| `0x736b` | u32 | ground transparency, 0–100 [4005] |
| `0x736c` / `0x736d` | f64 | component editing: fade the rest of the model / fade similar components, opacity 0–1 (Model Info ▸ Components) |
| `0x736e` / `0x736f` | u8 | component editing: hide the rest of the model / hide similar components |
| `0x7370` / `0x7371` | RGBA | active / inactive section plane colour [7003 / 7004] |
| `0x7372` / `0x7373` | RGBA | section cut / section fill colour [7005 / 7016] |
| `0x7374` | u32 | section cut width [7014] |
| `0x7375` | u32 | bits: 1 section planes, 2 section cuts [7013] |
| `0x7376` | u8 | section fill [7015] |
| `0x7377` | u32 | transparency quality: 0 faster, 2 nicer [2006] |
| `0x7378` | f64 | [2008]: 0.65 in the default style, 0.5 in theater's (**candidate**: X-ray opacity; the control is disabled in SketchUp Make 2017) |
| `0x7379` / `0x737b` | u8 | 1 (undecoded) |
| `0x737a` | f64 | 0.3491 (20° in radians; undecoded) |
| `0x737c` / `0x737d` | u8 / f64 | Match Photo background photo / its opacity [8000 / 8001] |
| `0x737e` / `0x737f` | u8 / f64 | Match Photo foreground photo / its opacity [8002 / 8003] |
| `0x7382` | u8 | 1 when the paint tool was active at save time (no style item; not a setting) |
| `0x7383`, `0x738b` | u8 | 0 (undecoded) |
| `0x7385` / `0x7389` | f32 | 10.0 / 1.0 (undecoded) |
| `0x7386` | u32 | 0 (undecoded) |
| `0x738a` | RGBA | `00 00 00 FF` (undecoded) |

Evidence: every bracketed setting has a SketchUp Make 2017 pair (Styles
▸ Edit, one control each) converted by the web app, changing exactly
that record and that style item; fog, the section fill and component
editing come from web-app pairs; the face-style values and
`0x7342` from applying the web app's built-in styles; the component
axes and the two fade opacities from SketchUp Make 2017 Model Info ▸
Components pairs. The same settings in a 2017 file are laid out in
§10.7.

### 16.11 Shadow info

`0x6590` — the model's (`0x0204`) and each scene's (`0x714e`):

| tag | type | meaning |
|---|---|---|
| `0x6591` | u32 | date and time, UNIX seconds |
| `0x6592` | u8 | 0 (**candidate**: daylight saving — the one 2017 key, `DaylightSavings`, left after aligning the others) |
| `0x6593` / `0x6594` | UTF-8 | city / country |
| `0x6595` / `0x6596` | f64 | longitude / latitude |
| `0x6597` | f64 | time-zone offset, hours (web pair: UTC−9:30 → −9.5) |
| `0x6598` | 3 f64 | (0, 1, 0) (**candidate**: north direction — the 2017 `NorthAngle` key; never varied) |
| `0x6599` | u8 | shadows displayed |
| `0x659a` | u8 | north direction displayed |
| `0x659b` / `0x659c` / `0x659d` | u8 | shadows on faces / on the ground / from edges |
| `0x659e` / `0x659f` | u32 | light / dark (0–100) |
| `0x65a0` | u8 | use the sun for shading |

The 2017 file holds these settings in a binary record (§10.9), with
the displayed flag in the rendering options, and each field equals its
2026 counterpart in every conversion. It also carries a
`TempShadowInfo` option set (`City`, `Country`, `Dark`,
`DaylightSavings`, `DisplayNorth`, `DisplayOnAllFaces`,
`DisplayOnGroundPlane`, `DisplayShadows`, `EdgesCastShadows`,
`Latitude`, `Light`, `Longitude`, `NorthAngle`, `TZOffset`,
`UseSunForAllShading`). That set did not change in any shadow pair, so
it is not the live source for these settings.

### 16.12 Option sets

`0x0200 > 0x61a8 > 0x61a9 > 0x61aa` ×N: name `0x61ab`, and `0x61ac`
holding alternating key `0x61ad` / typed value `0x38a4` records (§16.8).
The keys are self-describing: `PageOptions` (`ShowTransition`,
`TransitionTime`), `NamedOptions` (empty), `SlideshowOptions`
(`LoopSlideshow`, `SlideTime`) and `UnitsOptions`:

| key | values (web-app pairs) |
|---|---|
| `LengthFormat` | 0 decimal, 1 architectural, 3 fractional |
| `LengthUnit` | 0 inches, 1 feet, 2 millimetres, 3 centimetres, 4 metres |
| `LengthPrecision`, `AreaPrecision`, `VolumePrecision`, `AnglePrecision` | decimal places (fractions: 1/2^n) |
| `LengthSnapEnabled` / `LengthSnapLength` | bool / f64 in the display unit |
| `AngleSnapEnabled` / `SnapAngle` | bool / f64 degrees |
| `AreaUnit` | 0 in², 1 ft², 2 mm², 3 cm², 4 m², 5 yd² |
| `VolumeUnit` | 0 in³, 1 ft³, 2 mm³, 3 cm³, 4 m³, 5 yd³, 6 litres, 7 US gallons |
| `SuppressUnitsDisplay`, `ForceInchDisplay` | bool |

2017 files carry `UnitsOptions` too, without the area and volume keys;
the conversion fills those in inconsistently. `LengthFormat` 2
(engineering, feet with decimals) exists only in 2017 files: the
conversion writes format 0 with `LengthUnit` 1 and keeps the precision
and snap length.

The animation keys are Model Info ▸ Animation: `ShowTransition` (bool),
`TransitionTime` (seconds), `SlideTime` (seconds each scene is shown),
`LoopSlideshow` (bool). `GeoReference` holds `UsesGeoReferencing`
(bool), `Latitude` and `Longitude` (degrees), `GeoReferenceNorthAngle`,
`LocationSource` (`Manual` after Model Info ▸ Geo-location ▸ Set Manual
Location, which sets the first three) and `ModelTranslationX` / `Y`.
Both sets carry over from 2017 files unchanged.

### 16.13 Text, dimensions and fonts

A font `0x5014` (in `0x01fd`): entity base, family `0x5015`, bold
`0x5016`, italic `0x5017`, size in points `0x5018` (u32); `0x5019`
(u8 0) and `0x501a` (f64 10.0) are undecoded. Changing a text's font
creates a new font record.

An anchor `0x5208`: kind `0x5209` (0 none, 2 a vertex, 5 a point on an
edge), point or parameter `0x520a` (3 f64; for kind 5 the first is the
edge parameter), and the anchored entity `0x520b > 0x53fc` {entity id
`0x53fd`, instance path `0x53fe` (an id list, §16.2)}; `0x520c` has the
same shape and was empty in every observed anchor.

A text `0x55f0`:

| tag | meaning |
|---|---|
| `0x07d0` | drawing element; its material is the text colour |
| `0x55f1` | the text |
| `0x55f2` / `0x55f3` | f64: screen position as fractions of the view (screen text) |
| `0x55f4` | anchor `0x5208` (kind 0 for screen text) |
| `0x55f5` | 3 f64: leader offset — model space for a pushpin leader, screen space for a view-based one |
| `0x55f7` | 3 f64: the view direction when the text was placed (**candidate**) |
| `0x55f9` | font id |
| `0x55fa` | arrow: 0 none, 1 slash, 2 dot, 3 closed, 4 open |
| `0x55fc` | u8: leader shown |
| `0x55fd` | leader: 0 none (screen text), 1 view-based, 2 pushpin |
| `0x55fe` | u32: 1 when the leader is hidden (web app, no alignment chosen), else 0 |
| `0x55f6`, `0x55f8`, `0x55fb`, `0x55ff` | u8 1, u8 0, u32 1, empty (undecoded) |

A dimension `0x5bcc`:

| tag | meaning |
|---|---|
| `0x59d8` | base: drawing element, text override `0x59d9`, font id `0x59da`, text aligned to the dimension line `0x59db` (0 = aligned to the screen), arrow `0x59dc` (text's values) |
| `0x5bcd` / `0x5bce` | start / end anchor `0x5208` |
| `0x5bcf` / `0x5bd0` | 3 f64: plane normal / x axis |
| `0x5bd2` | f64: offset of the dimension line, inches |
| `0x5bd4` | u32 text position: 0 above, 1 centred, 3 below (2 not observed) |
| `0x5bd1` | u32: 1 or 2 — the second `house-plus` dimension stores 2; the 2017 u32 after the basis (§10.1); meaning undecoded |
| `0x5bd3` | f64 0 (undecoded) |

Defaults for new texts `0x57e4` (`0x01fe`): leader-text and
screen-text fonts `0x57e5` / `0x57e6`, arrow `0x57e7` (the text values),
leader text colour `0x57ec`, screen text colour `0x57ed`; `0x57e8` (1),
`0x57e9` (0), `0x57ea` (2) and `0x57eb` (1) are undecoded. Defaults for
new dimensions `0x5fb4` (`0x01ff`): font `0x5fb5`, aligned to the line
`0x5fb6`, arrow `0x5fbb`, colour `0x5fc4`, text position `0x5fc6`;
`0x5fb7`–`0x5fba`, `0x5fbc`–`0x5fc3` and `0x5fc5` are undecoded. The
2017 layout of both is §10.11.

Evidence: web-app pairs creating a screen text, a leader text on an
edge and a dimension, then changing each font, style, arrow, leader and
alignment control with "update all" (the web app also rewrites the
defaults with every dimension); SketchUp Make 2017 Model Info ▸ Text and
Dimensions pairs for the default arrows, colours, alignment and text
position, converted by the web app.

### 16.14 Styles, line styles and watermarks

Styles `0x6978` (`0x0206`): `0x6979` lists style records `0x6b6c`
{entity base, GUID `0x6b6d`, description `0x6b6e`, name `0x6b6f`,
scene id list `0x6b70`}; `0x697a` is the active style id, `0x697b` a
style record for the active style's current settings (`<name>_1`),
and `0x697c` a u8. That u8 is 0 in every file, including the
conversions of 2017 files whose unsaved-changes flag is set. Reading
it as that flag (§10.8) is a **candidate**. Each style's settings are
its `styles/<name>/style.xml` (§16.10).

Line styles (`0x0208 > 0x4074 > 0x4075`) `0x4076` ×N: entity base, name
`0x4077`, dash pattern `0x4078` (text: comma-separated lengths,
negative for gaps), `0x407a` / `0x4079` (f64 1.0), colour `0x407b`,
`0x407c` (u8 0). The same twelve stock patterns appear in every file.

A watermark (`0x0203 > 0x2cec > 0x2ced`) `0x2ee0`: entity base, then
fields matching by value the attributes of the style document's
`<screenimage>`: `0x2ee1` source file name, `0x2ee2` found flag,
`0x2ee3` file time, `0x2ee4` name, `0x2ee5` position (u32), `0x2ee6 >
0x2328` image {type `0x2329`, path `0x232a`}, `0x2ee7` tiled, `0x2ee8`
stretched, `0x2ee9` keep aspect ratio, `0x2eee` fitting type, `0x2eef`
stretch type, `0x2eea` background, `0x2eeb` scale (f64), `0x2eec`
intensity-as-alpha, `0x2eed` alpha scale (f64). `0x2cee` is a u32 (0).
The 2017 watermark pairs pin each field. Their meanings, the fitting
and stretch types the conversion derives, and the 3×3 position grid
are given in §10.8. The found flag is 0 in every file. The web app adds
its own "SketchUp" watermark (`/Resources/my_sketchup_watermark.png`,
stretched, scale 0.1845) to every file it saves, so a converted file
carries one watermark more than its 2017 original.

### 16.15 Scenes

`0x6d60` (`0x0207`): `0x6d61` lists scenes `0x7148`; `0x6d62` is the
selected scene's id. A scene `0x7148`:

| tag | meaning |
|---|---|
| `0x6f54` | base: entity base, name `0x6f55`, description `0x6f56` |
| `0x7149` | u32 properties saved: 1 camera, 2 style and fog, 4 shadows, 8 axes, 16 hidden geometry (the 2017 property), 32 visible layers, 64 active section planes, 128 hidden geometry, 256 hidden objects; bits 512–2048 have no control. Converting a 2017 scene sets 128 and 256 to its bit 16 and always sets 512; other bits carry over (`0x7F` → `0x3FF`, `0x0FFF` → `0x0FFF`, `0x0FEF` → `0x0E6F`, `0x0FDF` → `0x0FDF`) |
| `0x714a` | camera `0x34bc` (§16.9) |
| `0x714b` | id list, present while the hidden-geometry bits are set (the hidden entities, **candidate**) |
| `0x714c` | style id |
| `0x714d` | rendering options `0x733c` (§16.10) |
| `0x714e` | shadow info `0x6590` (§16.11) |
| `0x714f` | axes `0x4650` (§16.3) |
| `0x7150` | hidden layer id list, present while bit 32 is set (1, 3, 3 and 4 layers in a third-party bathroom model's scenes, as its 2017 original) |
| `0x7159` | id list, present while bit 32 is set; empty in every observed file |
| `0x7151` | id list: active section planes |
| `0x7152` | u8: included in animation |
| `0x7153` | display name: the name, or `(name)` when excluded from animation |
| `0x7154` / `0x7155` | f64 −1 (**candidate**: per-scene transition time and delay, −1 = model default) |
| `0x7157` | u8: 1 when the scene has a thumbnail (`0x7158` present) |
| `0x7158` | `0x2134`: thumbnail archive path |
| `0x715b` | `0x7b0f`–`0x7b16`: u8, u8, u32, f32 1.0, f32 1.0, u8, u32, u32 (undecoded) |
| `0x715c` | empty (undecoded) |

The record holds `0x714a` only while bit 1 is set, `0x714c` and
`0x714d` only while bit 2 is set, `0x714e` only while bit 4 is set,
`0x714f` only while bit 8 is set, and `0x7151` only while bit 64 is
set. The 2017 record works the same way (§10.10).

Evidence: web-app pairs toggling each "properties to save" control,
editing the name and description, and excluding a scene from animation;
SketchUp Make 2017 pairs adding a scene with and without its hidden
geometry and visible-layer properties, converted by the web app.

### 16.16 Known unknowns

- The 13 header bytes after the version string: `VFF`, u16 8, u16 1,
  u16 17 in every observed file, then a u32 that differs per file and is
  not the CRC-32 of the archive, of any entry, or of the header.
- Top level: `0x020d` (varies 1–3) and the constant sections listed in
  §16.3; the model-axes reading of `0x01fc` (never moved).
- Containers: the meaning of `0x139f`'s bits; `0x139b`, `0x139e`.
  Definitions: `0x1582`.
- Camera fields marked undecoded in §16.9.
- Rendering options marked undecoded in §16.10, and the candidates
  `0x7342` and `0x7378` (X-ray opacity cannot be changed in SketchUp
  Make 2017).
- Shadow info `0x6592` / `0x6598` (candidates; a north angle or daylight
  saving change has not been observed).
- Texts: `0x55f6`, `0x55f8`, `0x55fb`, `0x55ff`, the `0x55f7` reading,
  a hidden leader in a 2017 file. Dimensions: `0x5bd1`, `0x5bd3`, text
  position 2. Fonts: `0x5019`, `0x501a` (point versus model-height size
  is not exposed by the web app). Text defaults `0x57e8`–`0x57eb`;
  dimension defaults `0x5fb7`–`0x5fba`, `0x5fbc`–`0x5fc3`, `0x5fc5`.
- Scenes: property bits 16 and 512, `0x7154`/`0x7155`,
  `0x715b`, `0x715c`.
- Layers `0x3c90` and the layer-folder root (paid-only features).
- `UnitsOptions` `LengthFormat` 2 (engineering) is not offered by the web
  app and does not survive conversion (§16.12), so whether a 2026
  file can hold it is unknown.
- Releases 2018–2025: unobserved. `detect_container` accepts any file
  whose string records are followed by a ZIP archive, so the reader
  applies §16 to them; untested.

## Appendix A. Class catalogue

The complete 2017 class inventory — exactly the 58 classes `CVersionMap`
declares (the map does not list itself). *Coverage* is the reference
implementation's read tier: **Full** = complete body reader with exact
slot bookkeeping; **Skip** = exact-extent consumption, contents not
modeled; **Resync** = no body knowledge (scan to the next plausible
header); **manager** = pre-model document/manager record the entity walk
never meets (signature-based extractors recover any needed content).
This table is machine-checked against `CVersionMap` by
`crates/openskp/tests/class_tiers.rs`.

| class | schema | where it lives | coverage | notes |
|---|---|---|---|---|
| CVertex | 0 | inline in CEdge bodies | Full | 3 × f64 inches (§6.1) |
| CEdge | 2 | entity lists | Full | soft/smooth flags decoded (§6.2) |
| CEdgeUse | 1 | inline in CLoop | Full | the half-edge (§6.3) |
| CLoop | 1 | inline in CFace | Full | null-terminated edge-use list (§6.3) |
| CFace | 3 | entity lists | Full | plane + loops + front/back materials (§6.3) |
| CCurve | 4 | referenced by edges | Full | member count only; owns no children (§6.4) |
| CArcCurve | 3 | referenced by arc edges | Full | 14-f64 arc frame; schema 3 only (§6.4) |
| CComponentInstance | 5 | entity lists | Full | def pointer + 13-f64 transform (§7.2) |
| CGroup | 1 | entity lists | Full | byte-identical to CComponentInstance (§7.2) |
| CDimension | 1 | (base class) | Resync | never observed standalone |
| CDimensionLinear | 6 | entity lists | Full | exact extent; tail fields undecoded (§10.1) |
| CDimensionRadial | 2 | entity lists (expected) | Resync | not in corpus |
| CText | 9 | entity lists | Full | both variants; exact extent (§10.2) |
| CSectionPlane | 2 | entity lists | Full | plane equation in the list's local frame; exact extent (§10.3) |
| CImage | 1 | entity lists | Skip | exact extent; pixel linkage undecoded (§10.4) |
| CConstructionLine | 1 | entity lists | Full | guides (§10.3) |
| CConstructionPoint | 0 | entity lists | Full | typed-dimension proven (§10.3) |
| CConstructionGeometry | 0 | (base class) | Resync | never observed standalone |
| CPolyline3d | 0 | entity lists (expected) | Resync | drawn polylines store plain edges |
| CFaceTextureCoords | 4 | face attribute containers | Full | matrices + pins (§9) |
| CAttribute | 0 | (base class) | Resync | never observed standalone |
| CAttributeNamed | 1 | attribute containers | Full | typed key/values (§10.5) |
| CAttributeContainer | 0 | entity preambles, document | Full | children until null (§10.5) |
| CComponentDefinition | 10 | definition list + nested | Full | §7.1 |
| CComponentBehavior | 5 | definition records | manager | |
| CComponent | 11 | definition-record family | manager | |
| CDefinitionList | 0 | manager | manager | |
| CMaterial | 12 | material manager | Full | archive-walked list record (§8.1) |
| CMaterialManager | 4 | manager | manager | count-prefixed list (§8.4) |
| CTexture | 6 | inside materials | manager | applied size decoded (§8.1) |
| CDib | 3 | textures, thumbnails | Full | subtype + length + payload (§8.2) |
| CLayer | 2 | layer list + definitions | Full | §8.3 |
| CLayerManager | 4 | manager | manager | |
| CFontManager | 0 | manager | manager | |
| CSkFont | 1 | dimension/text bodies | Full | name + fixed tail (§10.1) |
| CSkpStyle | 1 | style region | manager | keyed style items (§10.8) |
| CSkpStyleManager | 2 | manager | manager | |
| CTextStyle | 5 | style region | manager | |
| CDimensionStyle | 4 | style region | manager | |
| CPageList | 1 | manager | manager | scene names by signature (§10.6) |
| CSketchUpPage | 1 | pages | manager | never observed; scenes are CViewPage (§10.10) |
| CViewPage | 12 | pages | manager | scenes (§10.10) |
| CCamera | 5 | views, thumbnails | Full | eye/target/up/projection/field of view (§10.6) |
| CRenderingOptions | 36 | document options | manager | |
| CShadowInfo | 7 | document options | manager | the shadow record is §10.9 (**candidate** attribution) |
| CBackgroundImage | 10 | document/style | manager | |
| CWatermark | 1 | style region | manager | §10.8 |
| CWatermarkManager | 2 | manager | manager | |
| CRelationship | 0 | definition tails | Full | chain node: two pointers + next (§7.1) |
| CRelationshipMap | 0 | manager | manager | |
| CSketchCS | 0 | document | manager | never observed |
| CSketchUpModel | 26 | document root | manager | root list found structurally (§4) |
| CThumbnail | 1 | previews, definition tails | Full | camera + nullable dib (§10.6) |
| CSchemaFile | 1 | schema plumbing | manager | |
| CSchemaFilterFile | 0 | schema plumbing | manager | |
| CSchemaZipFile | 1 | schema plumbing | manager | |
| CEntity | 5 | (base of everything) | Resync | the §5.1 preamble is its footprint |
| CDrawingElement | 9 | (base of drawables) | Resync | the 10-byte drawbase (§5.2) |

## Appendix B. Anchor index

Citation anchors used in source comments, mapped to this document:

| anchor | topic | section |
|---|---|---|
| §3 | save-to-save noise | 2.2 |
| §4c | vertex record | 6.1 |
| §4d | topology model, entity header | 5.1, 6 |
| §4e | materials; instance transform | 7.3, 8.1 |
| §4f | textured materials, face interior | 6.3, 8.1 |
| §4h | entity-list framing, definition prelude | 5.3 |
| §4i | entity preamble, pid presence mask | 5.1 |
| §4j | root-list anchor after the thumbnail | 4 |
| §4k | dimensions, fonts, text | 10.1, 10.2 |
| §4l | root tail; annotation body extents | 4, 10.3, 10.4 |
| §4m | template independence | 12 |
| §4n | material/layer binding | 8.4 |
| §4o | in-list instance body | 7.2 |
| §4p | transform convention | 7.3 |
| §4q | drawbase: hidden flag, layer ref | 5.2 |
| §4r | FTC slot localization | 9 |
| §4s | single archive, store map, document layout | 3.2, 4 |
| §4t | scale escalations, CCurve, CRelationship | 6.4, 11 |
| §4u | FTC body + pin lists | 9.1 |
| §4v | UV formula, face frame, inheritance | 9.2–9.4 |
| §4w | construction lines | 10.3 |
| §4x | third-party benchmark (validation, not format) | — |
| §5 | known unknowns | 15 |
