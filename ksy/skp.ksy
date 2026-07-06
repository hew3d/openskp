meta:
  id: skp
  title: SketchUp 2017 model (.skp) — clean-room spec
  file-extension: skp
  endian: le
  encoding: UTF-16LE
doc: |
  SketchUp 2017 (v17.3.116) .skp = an uncompressed Microsoft MFC `CArchive`
  serialization stream. Reverse-engineered clean-room from authored files +
  their COLLADA exports (see ../docs/SKP_FORMAT.md). No Trimble SDK used.

  THE SINGLE-ARCHIVE MODEL (SKP_FORMAT §4s): the entire payload from offset
  0x50 is ONE MFC CArchive with a single shared 1-based store map (classes
  and objects share one running index). The pre-model region (thumbnails,
  cameras, attribute dictionaries, materials + their CDibs) varies per file,
  so the number of pre-model slots is CALIBRATED per file (walk2.rs §4s:
  the definition-list header opens with an object pointer to the default
  layer = CLayer-class slot + 1). The model section then reads:

    <count:u32>  layer list            count × CLayer
    <layer-ptr:u2> <count:u32>         definition list (CComponentDefinition
                                       objects; purged slots = null tags)
    CComponentDefinition ×N            trailing defs, back-to-back class-refs
    <count:u32>  ROOT entity list      the model's own entities
    58 79 f0 6a 00 …                   the §4l root-record tail

  Escalations seen only at scale (§4t): object/class refs past 0x7FFF slots
  use the MFC big-tag escape (0x7FFF + u32); string records escalate at 255
  chars (see str_rec); the §4h decl field is u2 with 0x7FFF→u4 escalation.

  WHAT THIS SPEC PARSES: the fixed header (string records, format GUID, doc
  id) and the first class-definition record, then exposes the remainder raw.
  The body-record TYPES below formally document every byte layout decoded to
  date — the §4s entity preamble, CVertex/CEdge/CFace/CLoop/CEdgeUse, the
  §4v CFaceTextureCoords projective blocks + pin lists, CCurve,
  CRelationship, CConstructionLine, CLayer, materials/CDib, the 13-f64
  instance transform.

  WHAT KAITAI CANNOT DO HERE: the entity stream is a stateful MFC object
  graph — objects carry an implicit running map index and refer to earlier
  objects by that index (back-references), classes are defined lazily inline
  at first use, and child pointers are either inline objects or back-refs
  depending on serialization history. Sequencing it requires a stateful
  `ReadObject` walker, which is outside Kaitai's declarative model. That
  walk lives in ../crates/openskp (walk2.rs, the shipped reader) with
  ../tools/contwalk.py as the RE prototype.
seq:
  - id: model_tag
    type: str_rec
    doc: 'always "SketchUp Model"'
  - id: version
    type: str_rec
    doc: 'e.g. "{17.3.116}"'
  - id: format_guid
    size: 16
    doc: stable per format/version (not per-document)
  - id: doc_name
    type: str_rec
    doc: usually empty
  - id: doc_id
    type: u4
    doc: per-save id/seed (the only varying field this early)
  - id: first_class
    type: new_class_def
    doc: first MFC class definition; always CVersionMap
  - id: rest
    size-eos: true
    doc: remainder of the MFC CArchive object stream (walk with crates/openskp)
types:
  # ---- framing primitives ----
  str_rec:
    doc: |
      UTF-16LE string record: FF FE FF <len> then len UTF-16LE chars.
      The length ESCALATES (§4t): a u1 below 0xFF; 0xFF = read a u2;
      u2 0xFFFF = read a u4 (observed: >255-char definition names and
      warehouse figure descriptions).
    seq:
      - id: magic
        contents: [0xff, 0xfe, 0xff]
      - id: len8
        type: u1
      - id: len16
        type: u2
        if: len8 == 0xff
      - id: len32
        type: u4
        if: len8 == 0xff and len16 == 0xffff
      - id: value
        type: str
        size: n_chars * 2
    instances:
      n_chars:
        value: 'len8 < 0xff ? len8 : (len16 < 0xffff ? len16 : len32)'
  new_class_def:
    doc: 'MFC new-class tag: FF FF <schema:u2> <name_len:u2> <ascii name>'
    seq:
      - id: magic
        contents: [0xff, 0xff]
      - id: schema
        type: u2
      - id: name_len
        type: u2
      - id: name
        type: str
        size: name_len
        encoding: ASCII
  entity_preamble:
    doc: |
      Every entity body starts with this (§4s, superseding §4i's "00 00
      lead bytes"): a NULLABLE MFC OBJECT POINTER (the entity's attribute
      container — 00 00 null tag for plain entities; textured faces carry
      an inline CAttributeContainer here holding the CFaceTextureCoords),
      then the pid presence mask + the stored pid bytes. Mask bit k set =
      pid byte k stored (low-to-high); clear bits are zero and omitted.
      THIS TYPE COVERS THE NULL-POINTER CASE ONLY — a non-null pointer is
      an inline object or back-ref the stateful walker must read first.
    seq:
      - id: attr_ptr_null
        contents: [0x00, 0x00]
      - id: pid_mask
        type: u1
      - id: pid_bytes
        size: >
          (pid_mask & 1) + ((pid_mask >> 1) & 1)
          + ((pid_mask >> 2) & 1) + ((pid_mask >> 3) & 1)
  drawing_element_base:
    doc: |
      The 10-byte CDrawingElement base after the preamble (§4q). The
      matref is a GLOBAL archive slot on the §4s continuous map; on
      INSTANCES (CGroup/CComponentInstance) it is a material painted on
      the instance, inherited by default-material faces below it (§4v
      addendum).
    seq:
      - id: material_ref
        type: u2
        doc: front material slot, 0 = none
      - id: hidden
        type: u1
        doc: per-entity hidden flag (§4q)
      - id: unknown1
        size: 2
      - id: soft
        type: u1
        doc: CEdge soft flag (edges only)
      - id: smooth
        type: u1
        doc: CEdge smooth flag (edges only)
      - id: unknown2
        size: 1
      - id: layer_ref
        type: u2
        doc: layer slot, 0 = default layer
  # ---- value types ----
  vec3_inches:
    doc: 3 f64 in inches (1 m = 39.37007874015748 in)
    seq:
      - id: x
        type: f8
      - id: y
        type: f8
      - id: z
        type: f8
  rgba:
    doc: material colour, one byte each, direct order (NOT BGRA)
    seq:
      - id: r
        type: u1
      - id: g
        type: u1
      - id: b
        type: u1
      - id: a
        type: u1
  transform:
    doc: |
      Instance transform = 13 f64: 3x3 rotation/scale ROW-major, then
      translation (inches), then homogeneous w (=1.0). Column-vector
      convention p' = M·p; reproduces the COLLADA 4x4 as
      M[r][c] = rot[r*3+c], T = translation. Scale and mirroring
      (negative determinant) live in the 3x3 (§4t Tier C2).
    seq:
      - id: rot
        type: f8
        repeat: expr
        repeat-expr: 9
      - id: translation
        type: vec3_inches
      - id: w
        type: f8
  # ---- entity bodies (each follows a class tag + entity_preamble) ----
  cvertex_body:
    doc: CVertex schema 0 — final.
    seq:
      - id: pos
        type: vec3_inches
  cedge_body:
    doc: |
      CEdge schema 2 — base, then THREE MFC object pointers the stateful
      walker reads: endpoint v0, endpoint v1 (inline CVertex or back-ref),
      and the associated curve (CCurve/CArcCurve; null for a straight
      edge — the member-edge association direction for curves, §4t).
    seq:
      - id: base
        type: drawing_element_base
  cface_body:
    doc: |
      CFace schema 3 — base (front material in `material_ref`), plane
      (unit normal A,B,C + offset D), loop_count, then loop_count MFC
      loop pointers, then a trailing u2 BACK material slot. A textured
      face's preamble attr pointer holds its CFaceTextureCoords.
    seq:
      - id: base
        type: drawing_element_base
      - id: plane
        type: f8
        repeat: expr
        repeat-expr: 4
      - id: loop_count
        type: u4
  cloop_body:
    doc: |
      CLoop schema 1 = 5-byte base then a null-terminated list of CEdgeUse
      pointers (ends at wNullTag 0x0000).
    seq:
      - id: base
        size: 5
  cedgeuse_body:
    doc: |
      CEdgeUse schema 1 (after a 3-byte pid-less preamble) = edge pointer
      (back-ref u2) + direction u1 (loop traversal sense) + parent
      pointer (the owning CLoop — every edge-use is a loop-alignment
      oracle, §4t).
    seq:
      - id: edge_ref
        type: u2
      - id: direction
        type: u1
      - id: parent_ref
        type: u2
  cftc_pin:
    doc: |
      One §4u texture pin: the texture-space anchor (inches) and the
      face-local point it is pinned to (inches, §4v frame). The side's
      projective matrix reproduces every pin exactly:
      [anchor_u, anchor_v, 1] · K = face (homogeneous).
    seq:
      - id: anchor_u
        type: f8
      - id: anchor_v
        type: f8
      - id: face_x
        type: f8
      - id: face_y
        type: f8
  cftc_body:
    doc: |
      CFaceTextureCoords schema 4 — SOLVED (§4u layout, §4v semantics).
      After the entity preamble: u4, then TWO 12-f64 side blocks. Each
      block = a row-major 3×3 PROJECTIVE matrix mapping texture space
      (inches) to the face-local frame ([t_u, t_v, 1] · K, row-vector) +
      3 trailing f64s (zero on user faces, TBD). Then the front and back
      pin lists and two flag u4s ((1,0) affine-era / (0,1) pin-era
      observed, TBD).

      Face-local frame (§4v, shared by both sides, from the FRONT plane
      normal n): if |n_z| == 1 then v = +Y, else v = normalize(Z − (Z·n)n);
      u = v × n; local(P) = (P·u, P·v) inches. Per-vertex UV =
      [x_l, y_l, 1] · K⁻¹ → (t/w) / material applied size. No FTC =
      identity placement.
    seq:
      - id: unknown
        type: u4
        doc: 0 in every instance
      - id: front_matrix
        type: f8
        repeat: expr
        repeat-expr: 9
      - id: front_extra
        type: f8
        repeat: expr
        repeat-expr: 3
      - id: back_matrix
        type: f8
        repeat: expr
        repeat-expr: 9
      - id: back_extra
        type: f8
        repeat: expr
        repeat-expr: 3
      - id: front_pin_count
        type: u4
      - id: front_pins
        type: cftc_pin
        repeat: expr
        repeat-expr: front_pin_count
      - id: back_pin_count
        type: u4
      - id: back_pins
        type: cftc_pin
        repeat: expr
        repeat-expr: back_pin_count
      - id: flags
        type: u4
        repeat: expr
        repeat-expr: 2
  ccurve_body:
    doc: |
      CCurve schema 4 (§4t) — welded/freehand curve: u1 flag + u4
      member-edge count. NO owned children: member edges are ordinary
      list/loop elements whose curve pointers back-ref this record.
    seq:
      - id: flag
        type: u1
      - id: member_count
        type: u4
  crelationship_body:
    doc: 'CRelationship schema 0 (§4t, definition tails): a single u4.'
    seq:
      - id: value
        type: u4
  cconstructionline_body:
    doc: |
      CConstructionLine schema 1 (§4w, guide.skp): after preamble +
      drawing_element_base — anchor point, unit direction, and the two
      line-parameter bounds (±1e30 exactly = infinite guide), + 7B tail
      (zeros observed, TBD).
    seq:
      - id: point
        type: vec3_inches
      - id: direction
        type: vec3_inches
      - id: bound_lo
        type: f8
      - id: bound_hi
        type: f8
      - id: tail
        size: 7
  cconstructionpoint_body:
    doc: |
      CConstructionPoint schema 0 (§4l): position + reference vector + u4.
    seq:
      - id: position
        type: vec3_inches
      - id: reference
        type: vec3_inches
      - id: tail
        type: u4
  clayer_body:
    doc: |
      CLayer schema 2 (§4s): after the preamble — display name, hidden
      flag, internal "Layer_<name>" string, u2, display RGBA, string,
      21-byte tail (holds an f64; 0.0/0.5 observed).
    seq:
      - id: name
        type: str_rec
      - id: hidden
        type: u4
      - id: internal_name
        type: str_rec
      - id: w16
        type: u2
      - id: color
        type: rgba
      - id: s2
        type: str_rec
      - id: tail
        size: 21
  ccamera_body:
    doc: |
      CCamera schema 5 (§4s): NO preamble — 137B raw (f64 eye/target/up…,
      fields TBD) + u2 + description string + 33B tail.
    seq:
      - id: head
        size: 137
      - id: w
        type: u2
      - id: description
        type: str_rec
      - id: tail
        size: 33
  cdib:
    doc: |
      Embedded raster (schema 3) = u4 format (1=JPEG, 4=PNG) + u4 length +
      raw image bytes. Used for thumbnails and material textures.
    seq:
      - id: format
        type: u4
        doc: 1=JPEG, 4=PNG
      - id: length
        type: u4
      - id: image
        size: length
  material_solid:
    doc: 'solid CMaterial: name, 2 bytes, RGBA, texture-path (empty)'
    seq:
      - id: name
        type: str_rec
      - id: gap
        size: 2
      - id: color
        type: rgba
      - id: texture_path
        type: str_rec
  material_textured:
    doc: |
      Textured CMaterial, inline-image form: name + has-texture u4(1) +
      the CDib class tag + inline CDib. A JPEG (format 1) payload is
      followed by a u4 (70/99 observed, TBD — §4v); a PNG (format 4)
      payload is not. Then the applied size (2 f64 inches — the §4v UV
      denominator), the source filename, and the average RGBA (what .dae
      exports emit as the material's diffuse), u1, second RGBA.

      SHARED-image form (§4s addendum 2): has-texture u4(1) + a u2
      OBJECT BACK-REF to an earlier material's CDib + f64 w/h + filename
      + average RGBA (house.skp's "[Wood Floor Light]1").
    seq:
      - id: name
        type: str_rec
      - id: has_texture
        type: u4
      - id: dib_class_tag
        type: u2
        doc: class-ref to the CDib class slot
      - id: dib
        type: cdib
      - id: jpeg_quirk
        type: u4
        if: dib.format == 1
        doc: 70/99 observed; TBD (quality?)
      - id: applied_width_in
        type: f8
      - id: applied_height_in
        type: f8
      - id: filename
        type: str_rec
      - id: avg_color
        type: rgba
        doc: the .dae diffuse for this material
      - id: sep
        type: u1
      - id: avg_color2
        type: rgba
        doc: near-duplicate of avg_color, ±1/level; TBD
  component_instance_body:
    doc: |
      CComponentInstance schema 5 / CGroup schema 1 — IDENTICAL layouts
      (§4s: a group is an instance of an anonymous definition). After
      preamble + drawing_element_base (whose material_ref is the
      INSTANCE-painted, inheritable material): an MFC OBJECT POINTER to
      the definition (byte-identical to a u2 back-ref below 0x8000 slots;
      big maps escalate 7F FF + u4, and an instance whose definition is
      not yet serialized INLINES it here — §4t), the 13-f64 transform,
      the instance name string, and a 16-byte GUID.
    seq:
      - id: base
        type: drawing_element_base
      - id: def_ref
        type: u2
        doc: the definition's GLOBAL map slot (small-map case)
      - id: xform
        type: transform
      - id: name
        type: str_rec
      - id: guid
        size: 16
  component_definition:
    doc: |
      CComponentDefinition schema 10 (§4s/§4t) — the stateful shape:
      preamble (attr pointer may be non-null: GSU/GeoReference dicts) +
      22B base + u4 layer-count + that many layer objects (each def
      carries its own Layer0 copy) + the §4h prelude
      <decl:u2, 0x7FFF→u4 escalated> <u4 0> <count:u4> (decl−1 = the
      declared GLOBAL slot; the READ-side slot is what def-refs carry —
      purged def-list slots shift the writer's numbering) + count
      entities + tail: u4 relationship-count + CRelationship objects +
      u2 + GUID(16) + name + description + string + u4 UNIX timestamp +
      42–47B mid-tail (not pinned) + CThumbnail object (camera +
      nullable CDib preview).
    seq:
      - id: opaque
        size: 0
        doc: fully stateful — documented above, decoded in crates/openskp
  cversionmap_entry:
    doc: |
      CVersionMap body = a list of these (class name + schema) terminated
      by the sentinel entry name "End-Of-Version-Map". Enumerates every
      class in the file (58 in the 2017 corpus).
    seq:
      - id: class_name
        type: str_rec
      - id: schema
        type: u4
