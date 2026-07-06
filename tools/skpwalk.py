#!/usr/bin/env python3
"""skpwalk — SketchUp .skp class readers on top of the CArchive engine.

Two entry points:
  inventory(path) -> [(classname, schema), ...]   (from CVersionMap; works on ANY
                     file incl. a full house — tells you every class present)
  geom_walk(path) -> parsed user-geometry objects, resolving the inline-new vs.
                     back-reference pointer recursion (CEdge -> CVertex, etc.)

Coverage so far: CVersionMap, CVertex, CEdge. CFace/CLoop/CEdgeUse are partially
modelled; classes without a reader raise carchive.Stall with the resume point
(the worklist for further decoding). See ../docs/SKP_FORMAT.md.
"""
import sys, os, struct, re
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import carchive
import skpparse
import extract_images as _ei

INCH = 39.37007874015748
TEMPLATE_BASELINE_PID = 0x1281


def object_stream_start(d):
    """Offset just past the fixed header (2 str recs, GUID, str rec, doc id)."""
    o = 0
    for _ in range(2):                       # model_tag, version
        o += 4 + d[o+3] * 2
    o += 16                                  # format GUID
    o += 4 + d[o+3] * 2                      # doc_name string
    o += 4                                   # doc_id u32
    return o


# ---- class body readers ----
def r_cversionmap(ar, o):
    """list of (classname, schema) until the 'End-Of-Version-Map' sentinel."""
    entries = []
    while True:
        name = ar.utf16()
        schema = ar.u4()
        if name == "End-Of-Version-Map":
            break
        entries.append((name, schema))
    o["entries"] = entries


def _entity_preamble(ar):
    """CEntity preamble: 00 00 03 + persistent id (u16)."""
    tag = ar.take(3)                         # 00 00 03
    return ar.u2()                           # pid


def r_cvertex(ar, o):
    o["pid"] = _entity_preamble(ar)
    o["x"], o["y"], o["z"] = ar.f8(), ar.f8(), ar.f8()
    o["m"] = tuple(round(v / INCH, 5) for v in (o["x"], o["y"], o["z"]))


def r_cedge(ar, o):
    o["pid"] = _entity_preamble(ar)
    o["drawbase"] = ar.take(10)              # CDrawingElement base (matref + flags)
    o["soft"] = bool(o["drawbase"][5])       # soft edge  (drawbase[5])
    o["smooth"] = bool(o["drawbase"][6])     # smooth edge (drawbase[6])
    o["v0"] = ar.read_object()               # endpoint: inline CVertex or back-ref
    o["v1"] = ar.read_object()
    o["curve"] = ar.read_object()            # associated CCurve (null for a straight edge)
    # NOTE: edges in a solid additionally carry a winged-edge use/face list here;
    # not yet decoded (needs the solid-geometry corpus). Open/lone edges stop here.


def r_cedgeuse(ar, o):
    ar.take(3)                               # 00 00 00
    o["edge"] = ar.read_object()             # CEdge (back-ref)
    o["dir"] = ar.u1()                       # loop traversal sense
    o["parent"] = ar.read_object()           # owning CLoop (back-ref)


def r_cloop(ar, o):
    ar.take(5)                               # loop base (00 00 00 01 01)
    o["edge_uses"] = []
    while True:
        eu = ar.read_object()                # null-terminated CEdgeUse list
        if eu is None:
            break
        o["edge_uses"].append(eu)


def r_cface(ar, o):
    o["pid"] = _entity_preamble(ar)
    o["drawbase"] = ar.take(10)              # front-material u16 + flags
    o["front_material"] = struct.unpack_from("<H", o["drawbase"], 0)[0]  # 0 = none
    o["plane"] = [ar.f8() for _ in range(4)]  # A,B,C,D (unit normal + offset)
    o["loops"] = [ar.read_object() for _ in range(ar.u4())]
    o["back_material"] = ar.u2()             # back-face material u16 (0 = none)


def r_carccurve(ar, o):
    o["pid"] = _entity_preamble(ar)          # 00 00 03 + pid (5 bytes)
    o["header"] = ar.take(5)                  # 00 0c 00 00 00
    o["params"] = [ar.f8() for _ in range(14)]  # arc geometry (center/axes/radius/angles)


GEOM_READERS = {
    "CVertex": r_cvertex, "CEdge": r_cedge, "CFace": r_cface,
    "CLoop": r_cloop, "CEdgeUse": r_cedgeuse, "CArcCurve": r_carccurve,
}


def _make_archive(d, pos):
    ar = carchive.CArchive(d, pos)
    ar.register("CVersionMap", r_cversionmap)
    for name, fn in GEOM_READERS.items():
        ar.register(name, fn)
    return ar


def component_definitions(path):
    """Enumerate component & group definitions: {name, guid}. A `CComponentDefinition`
    header stores a 16-byte GUID + id then the definition name string, followed by
    two empty string records (description + file path). That trailing
    `FF FE FF 00 FF FE FF 00` is a robust, geometry-free signature."""
    d = open(path, "rb").read()
    out = []
    for m in re.finditer(b"\xff\xfe\xff(.)", d, re.DOTALL):
        n = m.group(1)[0]
        if n == 0:
            continue
        try:
            name = d[m.end():m.end()+n*2].decode("utf-16le")
        except Exception:
            continue
        after = m.end() + n*2
        if name.isprintable() and d[after:after+8] == b"\xff\xfe\xff\x00\xff\xfe\xff\x00":
            out.append({"name": name, "guid": d[m.start()-18:m.start()-2].hex(),
                        "off": m.start()})
    return out


def component_instances(path):
    """`CComponentInstance` entities placed in the model. Layout:
    `00 00 03 <pid>` + 10-byte CDrawingElement base + **def-ref (u16 = the
    referenced CComponentDefinition's map index)** + 13-f64 transform (3x3
    rotation row-major, translation, w=1.0). Found by the w=1.0 + drawbase
    signature; def-ref distinguishes which definition each instance places."""
    d = open(path, "rb").read()
    out, n, o = [], len(d), 16
    while o < n - 104:
        if (d[o+96:o+104] == b"\x00\x00\x00\x00\x00\x00\xf0\x3f"
                and d[o-12:o-2] == b"\x00\x00\x00\x01\x01\x00\x00\x00\x00\x00"):
            defref = struct.unpack_from("<H", d, o-2)[0]
            if defref != 0:
                v = struct.unpack_from("<13d", d, o)
                if all(abs(sum(c*c for c in v[i:i+3]) ** 0.5 - 1) < 1e-9 for i in (0, 3, 6)):
                    out.append({"pid": struct.unpack_from("<H", d, o-14)[0],
                                "defref": defref, "off": o,
                                "translation_m": tuple(round(t/INCH, 5) for t in v[9:12]),
                                "transform": v})
                    o += 104
                    continue
        o += 1
    return out


def component_tree(path):
    """The component hierarchy: definitions (name+GUID) and the instances that
    place them, each linked to its definition NAME. The instance def-ref is the
    definition's map index; sorted def-refs correspond to definitions in
    serialization (file-offset) order, so we zip them to resolve names."""
    defs = sorted(component_definitions(path), key=lambda d: d["off"])
    insts = component_instances(path)
    drefs = sorted({i["defref"] for i in insts})
    link = {dr: defs[k] for k, dr in enumerate(drefs)} if len(drefs) == len(defs) else {}
    for i in insts:
        d = link.get(i["defref"])
        i["definition"] = d["name"] if d else None
    return {"definitions": defs, "instances": insts}


def inventory(path):
    d = open(path, "rb").read()
    ar = _make_archive(d, object_stream_start(d))
    vm = ar.read_object()
    assert vm["__class__"] == "CVersionMap"
    return vm["entries"]


def _candidate_headers(d, baseline):
    res, off, n = [], 0, len(d)
    while off < n - 7:
        v = d[off] | (d[off+1] << 8)
        if (v & 0x8000) and d[off+2] == 0 and d[off+3] == 0 and d[off+4] == 3:
            pid = d[off+5] | (d[off+6] << 8)
            if pid > baseline:
                res.append((off, v, pid))
            off += 7
            continue
        off += 1
    return res


def user_entities(d, baseline=TEMPLATE_BASELINE_PID, gap=2048):
    """User-drawn entity headers, robust to stray false positives in thumbnail
    data: take the densest cluster by file-offset proximity (real geometry is
    contiguous; spurious matches in PNG blobs are isolated and far away)."""
    hs = sorted(_candidate_headers(d, baseline))
    if not hs:
        return []
    clusters, cur = [], [hs[0]]
    for h in hs[1:]:
        if h[0] - cur[-1][0] <= gap:
            cur.append(h)
        else:
            clusters.append(cur); cur = [h]
    clusters.append(cur)
    return max(clusters, key=len)


def first_user_entity(d, baseline=TEMPLATE_BASELINE_PID):
    ents = user_entities(d, baseline)
    return ents[0][0] if ents else None


_GEOM_CLASSES = {b"CEdge", b"CVertex", b"CFace", b"CLoop", b"CEdgeUse", b"CCurve"}


def _lazy_def_start(d, body_off, back=48):
    """If the entity body at body_off (the 00 00 03 <pid>) is introduced by a
    lazy class definition (FFFF schema namelen <C-class>), return that FFFF
    offset; else None. Used to find the true start inside a CComponentDefinition."""
    for p in range(body_off - 6, max(0, body_off - back) - 1, -1):
        if d[p] == 0xFF and d[p+1] == 0xFF:
            nlen = d[p+4] | (d[p+5] << 8)
            if 3 <= nlen <= 40 and p + 6 + nlen == body_off and d[p+6:p+6+nlen] in _GEOM_CLASSES:
                return p
    return None


def geometry_start(d, baseline=TEMPLATE_BASELINE_PID):
    """Start of the user geometry entity stream. Handles top-level geometry
    (class-ref entities) and component-definition geometry (which begins with
    lazy class definitions the class-ref scan misses). Scans every entity body
    marker `00 00 03 <pid>`, resolves each to its tag start (lazy def or
    class-ref), clusters by file offset, and returns the earliest start in the
    densest cluster (real geometry, not stray thumbnail matches)."""
    markers, off, n = [], 0, len(d)
    while off < n - 5:
        if d[off] == 0 and d[off+1] == 0 and d[off+2] == 3:
            pid = d[off+3] | (d[off+4] << 8)
            if baseline < pid < 0x4000:
                lazy = _lazy_def_start(d, off)
                if lazy is not None:
                    markers.append((lazy, off))
                elif off >= 2 and (d[off-2] | (d[off-1] << 8)) & 0x8000:
                    markers.append((off - 2, off))
        off += 1
    if not markers:
        return None
    markers.sort()
    clusters, cur = [], [markers[0]]
    for m in markers[1:]:
        if m[0] - cur[-1][0] <= 2048:
            cur.append(m)
        else:
            clusters.append(cur); cur = [m]
    clusters.append(cur)
    return max(clusters, key=len)[0][0]


def detect_geometry_tags(d, baseline=TEMPLATE_BASELINE_PID):
    """Map per-file class-ref tags -> classname by content. Vertices are the
    entities whose post-preamble body is 3 plausible coords; edges carry the
    CDrawingElement base then point at vertices. Tags shift per file, so we
    classify by bytes, not by hardcoded values."""
    from collections import Counter
    verts, edges = Counter(), Counter()
    tags = {}
    for off, v, pid in user_entities(d, baseline):
        idx = v & 0x7FFF
        body = d[off+7:off+7+24]
        if len(body) < 24:
            continue
        if all(c == 0.0 or 1e-4 < abs(c) < 1e4 for c in struct.unpack("<ddd", body)):
            verts[idx] += 1
            continue
        # face? the plane must be a real unit normal, AND
        # header(7)+drawbase(10)+plane(32)+loopcount(4) must be followed by a
        # CLoop class-ref tag (CLoop tag(2)+base(5) then a CEdgeUse tag).
        nrm = struct.unpack_from("<ddd", d, off + 7 + 10)
        unit = all(abs(c) <= 1.0001 for c in nrm) and \
            abs((nrm[0]**2 + nrm[1]**2 + nrm[2]**2) ** 0.5 - 1.0) < 1e-6
        p = off + 7 + 10 + 32 + 4
        ct = d[p] | (d[p+1] << 8)
        if unit and (ct & 0x8000) and idx not in verts:
            tags[idx] = "CFace"
            tags[ct & 0x7FFF] = "CLoop"
            et = d[p+7] | (d[p+8] << 8)
            if et & 0x8000:
                tags[et & 0x7FFF] = "CEdgeUse"
        else:
            edges[idx] += 1
    for idx in verts:
        tags.setdefault(idx, "CVertex")
    for idx in edges:
        tags.setdefault(idx, "CEdge")
    return tags


def _is_header(d, p):
    if p + 7 > len(d):
        return False
    v = d[p] | (d[p+1] << 8)
    if (v & 0x8000) and d[p+2] == 0 and d[p+3] == 0 and d[p+4] == 3:
        return True                            # class-ref entity
    if v == 0xFFFF:                             # lazy class def of a geometry class
        nlen = d[p+4] | (d[p+5] << 8)
        if 3 <= nlen <= 40 and d[p+6:p+6+nlen] in _GEOM_CLASSES:
            bp = p + 6 + nlen
            return bp + 5 <= len(d) and d[bp] == 0 and d[bp+1] == 0 and d[bp+2] == 3
    return False


def _run_walk(d, max_objects=8192, resync_gap=32):
    """Core geometry walk. Resyncs over short runs of non-object filler between
    top-level entities (e.g. a push/pull face's trailing edge-list, which is
    back-references only, so skipping it doesn't disturb the map index).
    Returns (archive, objects, skipped_ranges)."""
    start = first_user_entity(d)
    if start is None:
        return None, [], []
    ar = _make_archive(d, start)
    ar.seed_classes = detect_geometry_tags(d)
    objs, skipped = [], []
    for _ in range(max_objects):
        if ar.pos >= len(d) - 7:
            break
        if not _is_header(d, ar.pos):
            nxt = next((p for p in range(ar.pos + 1, min(ar.pos + resync_gap, len(d) - 7))
                        if _is_header(d, p)), None)
            if nxt is None:
                break
            skipped.append((ar.pos, nxt))
            ar.pos = nxt
        try:
            objs.append(ar.read_object())
        except carchive.Stall:
            break
    return ar, objs, skipped


def geom_walk(path, max_objects=8192):
    """Walk the user-geometry sub-stream, resolving the pointer recursion.
    Returns (objects, end_pos)."""
    d = open(path, "rb").read()
    ar, objs, _ = _run_walk(d, max_objects)
    return objs, (ar.pos if ar else None)


def walk_definitions(path):
    """Walk EVERY definition's geometry (multi-definition files store each
    definition's entity list back-to-back, separated by the next definition's
    header). Each definition is walked with a FRESH archive — its back-references
    are definition-local, so an isolated map is correct and avoids cross-boundary
    desync. Returns [(start_offset, objects), ...] (one entry per geometry run)."""
    d = open(path, "rb").read()
    seed = detect_geometry_tags(d)
    results, pos, seen = [], geometry_start(d), set()
    while pos is not None and pos < len(d) - 7 and pos not in seen:
        seen.add(pos)
        ar = _make_archive(d, pos)
        ar.seed_classes = seed
        objs = []
        while ar.pos < len(d) - 7:
            if not _is_header(d, ar.pos):
                nx = next((p for p in range(ar.pos+1, ar.pos+32) if _is_header(d, p)), None)
                if nx is None:
                    break
                ar.pos = nx
            try:
                objs.append(ar.read_object())
            except carchive.Stall:
                break
        results.append((pos, objs, ar.map[1:]))
        pos = next((p for p in range(ar.pos+1, ar.pos+1500) if _is_header(d, p)), None)
    return results


def _resolve_refs(objs, pool):
    """Resolve MFC back-references to concrete objects. Each (back-ref,
    expected-type) pair votes for every base that would land it on an object of
    that type; the modal base wins (robust to resync-perturbed constraints, and
    finds the GLOBAL base for top-level geometry or the definition-LOCAL base for
    a component definition). Returns {base, resolved, satisfied, constraints}."""
    from collections import Counter
    cons = []
    stack, seen = list(objs), set()           # iterative + cycle-safe (desynced
    while stack:                              # walks can nest pathologically deep)
        o = stack.pop()
        if not isinstance(o, dict) or id(o) in seen:
            continue
        seen.add(id(o))
        c = o["__class__"]
        if c == "CEdge":
            for k in ("v0", "v1"):
                if isinstance(o[k], carchive.Ref):
                    cons.append((o[k], "CVertex"))
            # (edge.curve back-refs to the shared CArcCurve are left unconstrained:
            #  they'd out-vote the vertex/edge base since a curve groups many edges)
        elif c == "CEdgeUse":
            if isinstance(o["edge"], carchive.Ref):
                cons.append((o["edge"], "CEdge"))
            if isinstance(o["parent"], carchive.Ref):
                cons.append((o["parent"], "CLoop"))
        stack.extend(o.get("loops", []))
        stack.extend(o.get("edge_uses", []))
        for k in ("v0", "v1", "curve"):
            if isinstance(o.get(k), dict):
                stack.append(o[k])
    cls = lambda p: p["__class__"] if isinstance(p, dict) else None
    pos_by_cls = {}
    for k, p in enumerate(pool):
        pos_by_cls.setdefault(cls(p), []).append(k)
    votes = Counter()
    for r, t in cons:
        for k in pos_by_cls.get(t, ()):
            votes[r.index - k] += 1
    base = votes.most_common(1)[0][0] if votes else None
    resolved, satisfied = {}, 0
    if base is not None:
        for r, t in cons:
            j = r.index - base
            if 0 <= j < len(pool) and cls(pool[j]) == t:
                resolved[r.index] = pool[j]
                satisfied += 1
    return {"base": base, "resolved": resolved,
            "constraints": len(cons), "satisfied": satisfied}


def resolve_geometry(path):
    """Walk the (densest) geometry and resolve its back-reference graph.
    Returns {objects, pool, base, resolved, satisfied, constraints, skipped}."""
    d = open(path, "rb").read()
    ar, objs, skipped = _run_walk(d)
    pool = ar.map[1:]
    return {"objects": objs, "pool": pool, "skipped": skipped, **_resolve_refs(objs, pool)}


def resolve_definitions(path):
    """Walk AND resolve every definition's geometry (each at its own local base).
    Returns [{start, objects, pool, base, resolved, satisfied, constraints}]."""
    out = []
    for start, objs, pool in walk_definitions(path):
        out.append({"start": start, "objects": objs, "pool": pool, **_resolve_refs(objs, pool)})
    return out


def _count_geom(objs):
    from collections import Counter
    c = Counter()
    stack, seen = list(objs), set()
    while stack:
        o = stack.pop()
        if not isinstance(o, dict) or id(o) in seen:
            continue
        seen.add(id(o))
        c[o["__class__"]] += 1
        stack.extend(o.get("loops", []))
        stack.extend(o.get("edge_uses", []))
        for k in ("v0", "v1"):
            if isinstance(o.get(k), dict):
                stack.append(o[k])
    return c


def parse_model(path):
    """Unified structured extraction of everything decoded: version + format GUID,
    component definitions (name+GUID), placed instances (definition name +
    transform), per-definition geometry (face/loop/edge/vertex counts + back-ref
    resolution), materials, and embedded images. The top-level entry point."""
    d = open(path, "rb").read()
    hdr = skpparse.parse_header(d)
    tree = component_tree(path)
    runs = []
    for dd in resolve_definitions(path):
        c = _count_geom(dd["objects"])
        sat, con = dd["satisfied"], dd["constraints"]
        if c["CVertex"] == 0 and c["CEdge"] == 0:
            continue                              # degenerate/thumbnail false-positive
        if con > 0 and sat < 0.75 * con:
            continue                              # low-confidence (desynced) fragment
        runs.append({"start": dd["start"], "faces": c["CFace"], "loops": c["CLoop"],
                     "edges": c["CEdge"], "vertices": c["CVertex"], "resolved": (sat, con)})
    return {
        "version": hdr["version"], "format_guid": hdr["format_guid"],
        "definitions": [{"name": x["name"], "guid": x["guid"]} for x in tree["definitions"]],
        "instances": [{"definition": i["definition"], "translation_m": i["translation_m"]}
                      for i in tree["instances"]],
        "geometry_runs": runs,
        "materials": skpparse.materials(d),
        "layers": skpparse.layers(d),
        "scenes": skpparse.scenes(d),
        "guides": skpparse.guides(d),
        "attributes": skpparse.attributes(d),
        "images": [{"kind": i["kind"], "bytes": i["len"]} for i in _ei.extract_images(d)],
    }


def _summary(path):
    m = parse_model(path)
    print(f"== {os.path.basename(path)} ==  version {m['version']}")
    print(f"inventory: {len(inventory(path))} classes")
    if m["definitions"]:
        print(f"definitions: {', '.join(repr(c['name']) for c in m['definitions'])}")
    for i in m["instances"]:
        print(f"  instance: {i['definition']!r} @ T={i['translation_m']} m")
    for r in m["geometry_runs"]:
        print(f"  geometry @0x{r['start']:x}: {r['faces']}F {r['loops']}L "
              f"{r['edges']}E {r['vertices']}V  (back-refs {r['resolved'][0]}/{r['resolved'][1]})")
    if m["materials"]:
        print(f"materials: {m['materials']}")
    if m["layers"]:
        print(f"layers: {[l['name'] for l in m['layers']]}")
    if m["scenes"]:
        print(f"scenes: {m['scenes']}")
    if m["attributes"]:
        geo = {k: v for k, t, v in m["attributes"] if k in ("Latitude", "Longitude")}
        print(f"attributes: {len(m['attributes'])} key/values" + (f"  geo={geo}" if geo else ""))
    print(f"images: {len(m['images'])} embedded ({', '.join(i['kind'] for i in m['images'])})")


if __name__ == "__main__":
    _summary(sys.argv[1])
