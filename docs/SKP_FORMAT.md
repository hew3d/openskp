# The SketchUp 2017 `.skp` File Format

This document specifies the SketchUp 2017 (v17.3.116) `.skp` binary format
as established by clean-room reverse engineering: every statement derives
from observing `.skp` files, their COLLADA exports, or public knowledge of
Microsoft MFC `CArchive` serialization — never from the Trimble SDK. Facts
that rest on a single observed instance are marked **candidate**; unknown
regions are listed in [Known unknowns](#15-known-unknowns).

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

A `.skp` file is a fixed header followed by **one uncompressed MFC
`CArchive` object stream**. It is not CFBF/OLE2 and not ZIP (post-2017
releases changed this; see §14).

Layout (offsets from `empty.skp`; all 2017 files match):

```
0x00  FF FE FF 0E + "SketchUp Model" (UTF-16LE)   document-type string record
0x20  FF FE FF 0A + "{17.3.116}"    (UTF-16LE)   version string record
0x3A  16 bytes                                    format GUID (constant)
0x4C  u32                                         doc id / save seed (per-save noise)
0x50  …                                           MFC CArchive object stream
```

- The 16-byte value at 0x3A is identical across independent saves and
  files — a format/schema identifier, not a per-document id. The 2017
  value is `b9a4f057 4a564270 ac7dda23 3a0eea83`.
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
root tail: zeros + constant marker 58 79 F0 6A 00 +
           geo-location strings (city, country) + latitude/longitude f64  [§4l]
```

The exact composition of the pre-model region is not specified — a reader
reaches the model section via the calibration anchors of §3.2 rather than
by decoding every manager. The marker `58 79 F0 6A 00` is constant across
every observed 2017 file and delimits the root record's trailing content.

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
numbering (§3.2).

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
22 bytes        base (undecoded)
u32             layer count, then that many CLayer objects (each definition
                 carries its own copy of the default layer)
list prelude    (§5.3) + entity list
tail:
  u32           relationship count, then count × CRelationship  [§4t]
  u16
  16 bytes      GUID
  utf16         name
  utf16         description
  utf16
  u32           UNIX timestamp (publish/save date)
  ~42–47 bytes  (undecoded)
  CThumbnail    object
```

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

Solid material:

```
utf16 name      (library/bundled names are bracketed: "[Wood Floor Light]")
u16   0         (no texture)
4 bytes         RGBA, one byte each, in that order
utf16           texture path (empty)
8 bytes
f64             opacity 0..1 — the opacity slider's last position; it
                applies ONLY when the use-opacity flag is set (the RGBA
                alpha byte is an opaque flag, 0xFF)
1 byte          use-opacity flag: stale slider values persist with the
                flag clear and render opaque (attributes.skp's *2 and *4
                both store 0.5; only flag-set *4 exports as transparent)
```

Textured material:

```
utf16 name
u32   1                    has-texture flag
<texture image>            EITHER an inline CDib object (class ref + body,
                           §8.2), OR a u16 back-ref holding the owning
                           material's CDib GLOBAL map slot — image data is
                           deduplicated across materials sharing a texture
                           (house.skp: "[Wood Floor Light]1" refs 23, the
                           slot after "[Wood Floor Light]" at 22) [§4s]
(JPEG payloads only) u32   (values 70/99 observed; semantics unknown)
f64 × 2                    applied texture size: width, height in inches
utf16                      texture filename
4 bytes                    average color RGBA (what exporters emit as the
                           material's diffuse color)
1 byte                     (0 observed)
4 bytes                    second average color RGBA (near-duplicate of
                           the first; alpha 0xFF observed)
utf16                      (empty)
u32                        (1 observed)
4 bytes
f64                        opacity 0..1 — the same stored-slider value as
                           solids
1 byte                     use-opacity flag, as for solids (house.skp's
                           "[Translucent Glass Tinted]" stores 0.52
                           flag-set, the transparency its .dae export
                           carries; flag-clear materials read opaque)
```

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
utf16   display name
u32     hidden flag
utf16   internal name ("Layer_<name>")
u16
4 bytes RGBA layer color
utf16
21 bytes tail (contains one f64; 0.0/0.5 observed)
```

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

```
CDimensionLinear:
  preamble + drawbase   (the material slot binds the dimension's material)
  utf16                 text override (empty = automatic text)
  <font>                CSkFont object (inline first, back-ref thereafter)
  165 bytes             fixed tail: two anchor back-ref words (offsets
                        [37..39) and [81..83)), direction/placement f64s,
                        a u32 axis selector at [141], and the f64 offset
                        distance at [145..153); remaining fields undecoded

CSkFont:
  pid-less preamble (mask 0)
  utf16 name ("Tahoma")
  u16 = 0,  u32 (height),  f64 (size),  u8 = 0     (15 bytes past the name)
```

Dimensions attach to model geometry via the anchor back-refs rather than
storing coordinates.

### 10.2 CText (schema 9) [§4k]

Two variants (leader text, screen text) share:

```
preamble + drawbase
<font>            CSkFont object (inline or back-ref)
variant middle    leader: anchor coords + leader vector + anchor back-ref;
                  screen: two screen-fraction f64s. NOT fixed-length.
11 bytes          01 00 00 00 01 00 03 00 00 00 01 (constant, both variants)
utf16             the text content
5 bytes           zeros
```

The constant 11-byte block immediately before the string delimits the
variant middle for readers that do not decode it.

### 10.3 Construction geometry [§4l, §4w]

**CConstructionPoint** (schema 0): preamble + drawbase + 3 × f64 position
+ 3 × f64 reference (tape-measure anchor) point + u32.

**CConstructionLine** (schema 1, guides): preamble + drawbase + 3 × f64
anchor + 3 × f64 unit direction + 2 × f64 line-parameter bounds (±1.0e30
exactly = the infinite-guide sentinel) + 7 zero bytes.

**CSectionPlane** (schema 2, candidate): preamble + drawbase + 4 × f64
plane (A, B, C, D) + u32; a trailing back-ref word follows in the single
observed instance.

### 10.4 CImage (schema 1, candidate) [§4l]

Preamble + drawbase + 106-byte placement block (contains inch-per-pixel
f64s consistent with the source image's pixel width) + utf16 source path
+ 16-byte GUID + u32. The pixel data lives in the document's embedded
`CDib` pool, not inline; the linkage is undecoded.

### 10.5 Attribute dictionaries [§4s, §4t]

**CAttributeContainer** (schema 0): preamble + child objects until a null
tag. **CAttributeNamed** (schema 1): preamble + 4 bytes + utf16
dictionary name + entries `[utf16 key + type:u8 + value]` terminated by
an empty-string key, + u32.

Value types: `0x00` nil (no bytes), `0x04` i32, `0x06` f64, `0x07` bool
(u8), `0x0A` utf16 string, `0x0B` typed array (u32 count + element type
u8 + values, recursive).

Dictionaries carry model options (units, snap settings), geo-location,
and dynamic-component parameters.

### 10.6 View records

**CThumbnail** (schema 1): preamble + CCamera object + nullable CDib
(the preview image). **CCamera** (schema 5): no preamble — 137 raw bytes
(eye/target/up/fov f64s, fields undecoded) + u16 + utf16 description +
33-byte tail. **Scene/page records** (`CSketchUpPage`/`CViewPage`): name
string + empty description + flags `7F 00 00 00` + an embedded CCamera —
extractable by that signature without decoding the page managers.
**CRelationship** (schema 0): preamble + u32 (value semantics unknown);
appears in definition tails [§4t].

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
- **2017 (v17.x)**: this specification.
- **Post-2017**: the container is a ZIP archive after the leading string
  records (`PK\x03\x04` at 0x41 with entries like `meta/model_th…`).
  Version identification still works; everything else is out of scope.

## 15. Known unknowns

None of these block reading geometry — each is either positionally
skipped with exact extent or lies outside the walk. Anchors refer to the
owning section.

Unknown bytes inside otherwise-exact records:

- CFaceTextureCoords: the leading u32; the per-side trailing f64 triples;
  the two flag u32s (§9.1).
- CComponentDefinition: the 22-byte base; the ~42–47-byte block between
  timestamp and thumbnail (§7.1).
- CCamera: the 137-byte head and 33-byte tail fields (§10.6).
- CLayer: the u16 and the 21-byte tail (§8.3).
- Drawbase bytes [3..5) and [7] (§5.2).
- CCurve's u8; CRelationship's u32 (§6.4, §10.6).
- CConstructionLine's 7-byte tail (§10.3).
- CDimensionLinear: the semantic fields inside the 165-byte tail (§10.1).
- CText: the leader-variant geometry fields (§10.2).
- Materials: the u32 after JPEG texture payloads; the second average
  color's role and the u32 + 4 bytes before the textured opacity (§8.1).
- CDib subtypes other than 1 and 4, if any exist (§8.2).

Candidate extents (single observed instance): CSectionPlane (and its
trailing back-ref word), CImage (§10.3, §10.4).

Never observed as entity-list elements: CDimensionRadial, CPolyline3d,
CComponentBehavior, CComponent, and the pre-model manager records
(CRenderingOptions, CShadowInfo, style and page managers) — scene names
and option dictionaries are recoverable by signature scans without them.

Narrow-oracle semantics: the horizontal-face frame rule (§9.2) is
validated on exact ±Z normals; behavior within ~1e-9 of ±Z is untested.
Instance-material inheritance beyond observed child-wins cases is
unproven.

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
| CSectionPlane | 2 | entity lists | Skip | candidate extent (§10.3) |
| CImage | 1 | entity lists | Skip | candidate extent (§10.4) |
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
| CMaterial | 12 | material manager | manager | extractor-decoded (§8.1) |
| CMaterialManager | 4 | manager | manager | count-prefixed list (§8.4) |
| CTexture | 6 | inside materials | manager | applied size decoded (§8.1) |
| CDib | 3 | textures, thumbnails | Full | subtype + length + payload (§8.2) |
| CLayer | 2 | layer list + definitions | Full | §8.3 |
| CLayerManager | 4 | manager | manager | |
| CFontManager | 0 | manager | manager | |
| CSkFont | 1 | dimension/text bodies | Full | name + fixed tail (§10.1) |
| CSkpStyle | 1 | style region | manager | |
| CSkpStyleManager | 2 | manager | manager | |
| CTextStyle | 5 | style region | manager | |
| CDimensionStyle | 4 | style region | manager | |
| CPageList | 1 | manager | manager | scene names by signature (§10.6) |
| CSketchUpPage | 1 | pages | manager | §10.6 |
| CViewPage | 12 | pages | manager | §10.6 |
| CCamera | 5 | views, thumbnails | Skip | exact 176-byte extent (§10.6) |
| CRenderingOptions | 36 | document options | manager | |
| CShadowInfo | 7 | document options | manager | |
| CBackgroundImage | 10 | document/style | manager | |
| CWatermark | 1 | style region | manager | |
| CWatermarkManager | 2 | manager | manager | |
| CRelationship | 0 | definition tails | Full | u32 value; semantics unknown (§10.6) |
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
| §4l | root tail; single-instance candidates | 4, 10.3, 10.4 |
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
