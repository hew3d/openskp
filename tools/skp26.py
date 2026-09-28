#!/usr/bin/env python3
"""skp26 — reverse-engineering reader for the 2026 (.skp) container.

Clean-room: derived only from observing SketchUp 2026 files (corpus/2026,
corpus/third-party/theater-2026.skp) against their decoded 2017 originals.

Container: two UTF-16 header string records ("SketchUp Model", version),
then a ZIP archive. `model.dat` is a tree of records

    u16 tag | u32 byte length | payload

where a payload is either a leaf value or a sequence of child records.
Tags are type-scoped (one tag means one field wherever it appears), so the
reader descends only into tags known to be containers.

Usage:
    python3 tools/skp26.py tree     <file.skp> [maxdepth]
    python3 tools/skp26.py json     <file.skp>
    python3 tools/skp26.py settings <file.skp>
"""
import io
import json
import struct
import sys
import zipfile
import xml.etree.ElementTree as ET

# ---------------------------------------------------------------- container

def header_strings(d):
    """The leading FF FE FF <len:u8> UTF-16LE string records."""
    out, off = [], 0
    while d[off:off + 3] == b"\xff\xfe\xff":
        n = d[off + 3]
        out.append(d[off + 4:off + 4 + 2 * n].decode("utf-16le"))
        off += 4 + 2 * n
    return out, off


def open_zip(d):
    i = d.find(b"PK\x03\x04")
    if i < 0:
        raise ValueError("no ZIP archive: not a 2026 container")
    return zipfile.ZipFile(io.BytesIO(d[i:]))

# ---------------------------------------------------------------- records

def records(b):
    """Split a payload into (tag, payload) child records; raises if it does not
    tile exactly."""
    out, o = [], 0
    while o < len(b):
        if o + 6 > len(b):
            raise ValueError(f"truncated record header at {o}")
        t, n = struct.unpack_from("<HI", b, o)
        if o + 6 + n > len(b):
            raise ValueError(f"record {t:#06x} overruns its parent")
        out.append((t, b[o + 6:o + 6 + n]))
        o += 6 + n
    return out


class Node:
    """A record whose payload is a sequence of child records."""
    __slots__ = ("tag", "kids")

    def __init__(self, tag, payload):
        self.tag = tag
        self.kids = records(payload)

    def all(self, tag):
        return [p for t, p in self.kids if t == tag]

    def get(self, tag, default=None):
        for t, p in self.kids:
            if t == tag:
                return p
        return default

    def node(self, tag):
        p = self.get(tag)
        return Node(tag, p) if p is not None else None

    def nodes(self, tag):
        return [Node(tag, p) for p in self.all(tag)]


def uint(b):
    """Little-endian unsigned integer of the payload's width (ids are 1-3
    bytes wide, just enough for their value)."""
    return int.from_bytes(b, "little") if b is not None else None


def f64s(b):
    return list(struct.unpack(f"<{len(b) // 8}d", b))


def id_list(b):
    """A packed id list: repeated <width:u8> <id: width bytes LE>."""
    out, o = [], 0
    while o < len(b):
        w = b[o]
        out.append(int.from_bytes(b[o + 1:o + 1 + w], "little"))
        o += 1 + w
    return out

# ---------------------------------------------------------------- entities

def entity_id(base):
    """0x05dc entity base -> its persistent id (0x05de)."""
    return uint(Node(0x05DC, base).get(0x05DE))


def drawing(n):
    """0x07d0 drawing-element header: id, material ref (a face's FRONT
    material), layer ref, flags (0x01 hidden, 0x08 soft, 0x10 smooth)."""
    h = n.node(0x07D0)
    return {
        "id": entity_id(h.get(0x05DC)) if h.get(0x05DC) is not None else None,
        "material": uint(h.get(0x07D1)),
        "layer": uint(h.get(0x07D2)),
        "flags": uint(h.get(0x07D3)),
        "attrs": Node(0x05DC, h.get(0x05DC)).get(0x05DD) if h.get(0x05DC) is not None else None,
    }


def uv_sides(attrs):
    """Texture projection for a face, from its attribute container
    (0x05dd > 0x36b1 > 0x36b2 > 0x2710): per side (0x2711 front, 0x2712
    back) a 3x3 matrix (0x2715), 3 f64 (0x2716) and a pin list (0x2717)."""
    if attrs is None:
        return None
    a = Node(0x05DD, attrs).node(0x36B1)
    b = a.node(0x36B2) if a else None
    t = b.node(0x2710) if b else None
    if t is None:
        return None
    out = {}
    for side, tag in (("front", 0x2711), ("back", 0x2712)):
        s = t.node(tag)
        s = s.node(0x2713) if s else None
        if s is None:
            continue
        pins = []
        pl = s.get(0x2717)
        if pl:
            for pn in Node(0x2717, pl).nodes(0x2718):
                pins.append([f64s(pn.get(0x2719)), f64s(pn.get(0x271A))])
        out[side] = {
            "u32": uint(s.get(0x2714)),
            "matrix": f64s(s.get(0x2715)),
            "extra": f64s(s.get(0x2716)),
            "pins": pins,
        }
    return out


def container(n):
    """0x1388 entity container (a definition's or the model's entities)."""
    out = {"id": drawing(n)["id"], "vertices": {}, "edges": {}, "faces": {},
           "instances": [], "order": id_list(n.get(0x138E, b""))}
    vp = n.node(0x1389)
    for v in (vp.nodes(0x09C4) if vp else []):
        out["vertices"][entity_id(v.get(0x05DC))] = f64s(v.get(0x09C5))
    ep = n.node(0x138A)
    for e in (ep.nodes(0x0BB8) if ep else []):
        h = drawing(e)
        out["edges"][h["id"]] = {"v": [uint(e.get(0x0BB9)), uint(e.get(0x0BBA))],
                                 "curve": uint(e.get(0x0BBB)), "flags": h["flags"],
                                 "layer": h["layer"], "material": h["material"]}
    fp = n.node(0x138B)
    for f in (fp.nodes(0x0DAC) if fp else []):
        h = drawing(f)
        loops = []
        for lp in Node(0x0DAE, f.get(0x0DAE)).nodes(0x1194):
            uses = lp.node(0x1195).nodes(0x0FA0)
            loops.append([(uint(u.get(0x0FA1)), uint(u.get(0x0FA2))) for u in uses])
        out["faces"][h["id"]] = {"plane": f64s(f.get(0x0DAD)), "loops": loops,
                                 "front": h["material"], "back": uint(f.get(0x0DAF)),
                                 "flags": h["flags"], "layer": h["layer"],
                                 "uv": uv_sides(h["attrs"])}
    # Annotation and construction pools, each a list of records (counts
    # mirror the 2017 topology counters).
    out["counts"] = {name: len(Node(pool, n.get(pool)).kids) if n.get(pool) else 0
                     for name, pool in (("curves", 0x1397), ("plain_curves", 0x1396),
                                        ("images_placed", 0x1390), ("guides", 0x1391),
                                        ("construction_points", 0x1392),
                                        ("section_planes", 0x1393), ("texts", 0x1398),
                                        ("dimensions", 0x1399))}
    out["guides"] = []
    gp = n.node(0x1391)
    for g in (gp.nodes(0x4269) if gp else []):
        v = f64s(g.get(0x426A))
        out["guides"].append({"point_in": v[0:3], "direction": v[3:6], "bounds": v[6:8]})
    out["texts"] = [t.get(0x55F1).decode("utf-8", "replace") for t in (n.node(0x1398).nodes(0x55F0) if n.get(0x1398) else [])]
    out["section_planes"] = [f64s(x.get(0x445D)) for x in (n.node(0x1393).nodes(0x445C) if n.get(0x1393) else [])]
    out["construction_points"] = [f64s(x.get(0x426D)) for x in (n.node(0x1392).nodes(0x426C) if n.get(0x1392) else [])]
    for pool, item in ((0x138C, None), (0x138D, 0x1D4C)):
        pn = n.node(pool)
        if pn is None:
            continue
        for x in pn.nodes(item or 0x1964):
            inst = x if item is None else x.node(0x1964)
            if inst is None:
                continue
            h = drawing(inst)
            out["instances"].append({
                "id": h["id"], "group": item is not None,
                "name": inst.get(0x1965, b"").decode("utf-8", "replace"),
                "transform": f64s(inst.get(0x1966)),
                "definition": uint(inst.get(0x1967)),
                "guid": inst.get(0x1968).hex(), "flags": h["flags"],
                "layer": h["layer"], "material": h["material"],
            })
    return out


MAT_NS = "{http://sketchup.google.com/schemas/sketchup/1.0/material}"


def material(z, rec):
    """A 0x32c8 material record plus its materials/<name>/material.xml.
    The record name is the archive folder name; the XML's own name
    attribute keeps the original (2017 files' unnamed '*N' become folder
    '_N' but stay '*N' in the XML)."""
    m = Node(0x32C8, rec)
    folder = m.get(0x32CC).decode("utf-8", "replace")
    out = {"id": entity_id(m.get(0x05DC)), "folder": folder, "name": folder}
    try:
        x = ET.fromstring(z.read(f"materials/{folder}/material.xml"))
    except KeyError:
        return out
    e = x.find(f"{MAT_NS}material")
    a = e.attrib
    out["name"] = a.get("name", folder)
    use_trans = a.get("useTrans") == "1"
    opacity = float(a.get("trans", "1")) if use_trans else 1.0
    rgb = [int(a.get(k, "0")) for k in ("colorRed", "colorGreen", "colorBlue")]
    out["rgba"] = rgb + [int(opacity * 255)]
    out["opacity"] = opacity
    t = e.find(f"{MAT_NS}texture")
    if a.get("hasTexture") == "1" and t is not None:
        out["kind"] = "textured"
        out["texture"] = t.get("textureFilename")
        out["applied_size_in"] = [float(t.get("xScale")), float(t.get("yScale"))]
        # The displayed colour: the texture's average (avgColor, packed
        # 0xAABBGGRR), or the tint colour for a colourised texture (type 2).
        if a.get("type") == "2":
            out["avg_rgba"] = rgb + [255]
        else:
            avg = int(t.get("avgColor", "0"))
            out["avg_rgba"] = [avg & 0xFF, (avg >> 8) & 0xFF, (avg >> 16) & 0xFF, (avg >> 24) & 0xFF]
        # The image path is folder-relative ("./x.jpg"), or archive-rooted
        # ("materials/<other>/x.jpg") when another material owns the bytes.
        img = t.find(f"{MAT_NS}images/{MAT_NS}image")
        path = img.get("path") if img is not None else "./" + (out["texture"] or "")
        path = f"materials/{folder}/{path[2:]}" if path.startswith("./") else path
        try:
            out["image"] = z.read(path)
        except KeyError:
            out["image"] = None
    else:
        out["kind"] = "solid"
    return out


def typed_value(b):
    """0x38a4 typed value: 0x38a7 i32, 0x38a9 f64, 0x38aa bool, 0x38ad
    string, 0x38ae array."""
    for t, p in records(b):
        if t == 0x38A7:
            return ("int", struct.unpack("<i", p)[0])
        if t == 0x38A9:
            return ("f64", struct.unpack("<d", p)[0])
        if t == 0x38AA:
            return ("bool", p[0] != 0)
        if t == 0x38AD:
            return ("str", p.decode("utf-8", "replace"))
        if t == 0x38AE:
            return ("array", None)
    return ("empty", None)


def key_values(b, key_tag):
    """Alternating <key_tag> key / 0x38a4 typed value records."""
    out, key = [], None
    for t, p in records(b):
        if t == key_tag:
            key = p.decode("utf-8", "replace")
        elif t == 0x38A4 and key is not None:
            out.append((key,) + typed_value(p))
            key = None
    return out


def all_attributes(top_payload):
    """Every typed key/value in the document: attribute dictionaries
    (0x36b3: 0x36b4 name, 0x36b5 key 0x36b6 / value pairs) wherever they
    sit, and the model option sets (0x61aa: 0x61ab name, 0x61ac key 0x61ad /
    value pairs)."""
    out = []

    def rec(b):
        try:
            kids = records(b)
        except ValueError:
            return
        for t, p in kids:
            if t == 0x36B5:
                out.extend(key_values(p, 0x36B6))
            elif t == 0x61AC:
                out.extend(key_values(p, 0x61AD))
            elif len(p) >= 6 and t not in LEAVES:
                rec(p)
    rec(top_payload)
    return out


# Fixed-size numeric leaves that can happen to tile as records.
LEAVES = {0x09C5, 0x0DAD, 0x1966, 0x2715, 0x2716, 0x157D, 0x1968, 0x13A0, 0x426A,
          0x426D, 0x426E, 0x445D, 0x4C2D, 0x38A9, 0x38A7}


def read(path):
    d = open(path, "rb").read()
    strings, _ = header_strings(d)
    z = open_zip(d)
    top = Node(0, records(z.read("model.dat"))[0][1])
    model = {"version": strings[1] if len(strings) > 1 else None}
    # materials, keyed by persistent id
    mats = {}
    mm = top.node(0x01F7)
    for rec in (mm.node(0x30D4).node(0x30D5).all(0x32C8) if mm else []):
        m = material(z, rec)
        mats[m["id"]] = m
    model["materials"] = mats
    # layers (0x3c8e is the hidden byte; 0x3c8f embeds the layer's colour
    # material, Layer_<name>)
    layers = {}
    lm = top.node(0x01F8)
    for l in (lm.node(0x3A98).node(0x3A99).nodes(0x3C8C) if lm else []):
        lmat = material(z, l.get(0x3C8F)[6:]) if l.get(0x3C8F) else {}
        layers[entity_id(l.get(0x05DC))] = {
            "name": l.get(0x3C8D).decode("utf-8", "replace"),
            "visible": uint(l.get(0x3C8E)) == 0,
            "rgba": lmat.get("rgba"),
        }
    model["layers"] = layers
    # definitions
    defs = []
    dm = top.node(0x01F9)
    if dm:
        for dl in dm.nodes(0x1770):
            for dd in dl.node(0x1771).nodes(0x157C):
                c = container(dd.node(0x1388))
                c["name"] = dd.get(0x157E).decode("utf-8", "replace")
                c["guid"] = dd.get(0x157D).hex()
                defs.append(c)
    model["definitions"] = defs
    model["root"] = container(top.node(0x01F6).node(0x1388))
    pages = top.node(0x0207)
    model["scenes"] = [pg.get(0x7153).decode("utf-8", "replace")
                       for pg in (pages.node(0x6D60).node(0x6D61).nodes(0x7148) if pages else [])]
    model["attributes"] = all_attributes(records(z.read("model.dat"))[0][1])
    model["archive"] = [(i.filename, i.file_size) for i in z.infolist()]
    return model


# ---------------------------------------------------------------- settings
# Field tables for the settings records (SKP_FORMAT §16.9–16.15): tag ->
# (name, type). Tags absent from a table print as "0xNNNN" with raw hex.

CAMERA = {0x34BD: ("eye", "v3"), 0x34BE: ("target", "v3"), 0x34BF: ("up", "v3"),
          0x34C2: ("perspective", "u8"), 0x34C3: ("view_height", "f64"),
          0x34C4: ("fov_deg", "f64"), 0x34CA: ("two_point", "u8")}

RENDER = {0x733D: ("face_style", "u32"), 0x733E: ("xray", "u8"),
          0x733F: ("transparency", "u8"), 0x7340: ("textures", "u8"),
          0x7341: ("edges", "u32"), 0x7343: ("model_axes", "u8"),
          0x7346: ("guides_hidden", "u8"), 0x7347: ("color_by_layer", "u8"),
          0x7348: ("edge_color_mode", "u32"), 0x7380: ("hidden_geometry", "u8"),
          0x7381: ("hidden_objects", "u8"), 0x734B: ("jitter", "u8"),
          0x734D: ("extension", "u8"), 0x734E: ("extension_length", "u32"),
          0x734F: ("profiles", "u8"), 0x7350: ("profiles_width", "u32"),
          0x7351: ("depth_cue", "u8"), 0x7352: ("depth_cue_width", "u32"),
          0x7353: ("endpoints", "u8"), 0x7354: ("endpoints_length", "u32"),
          0x7356: ("back_edges", "u8"), 0x7357: ("background", "rgba"),
          0x7358: ("edge_color", "rgba"), 0x7359: ("selected_color", "rgba"),
          0x735A: ("locked_color", "rgba"), 0x735B: ("guides_color", "rgba"),
          0x735C: ("front_color", "rgba"), 0x735D: ("back_color", "rgba"),
          0x735E: ("watermarks", "u8"), 0x735F: ("fog", "u8"),
          0x7360: ("fog_color", "rgba"), 0x7361: ("fog_uses_background", "u8"),
          0x7362: ("fog_start", "f64"), 0x7363: ("fog_end", "f64"),
          0x7365: ("sky_color", "rgba"), 0x7367: ("ground_color", "rgba"),
          0x7368: ("sky", "u8"), 0x7369: ("ground", "u8"),
          0x736A: ("ground_from_below", "u8"), 0x736B: ("ground_transparency", "u32"),
          0x736E: ("hide_rest_of_model", "u8"), 0x736F: ("hide_similar", "u8"),
          0x7370: ("active_section_color", "rgba"), 0x7371: ("inactive_section_color", "rgba"),
          0x7372: ("section_cut_color", "rgba"), 0x7373: ("section_fill_color", "rgba"),
          0x7374: ("section_cut_width", "u32"), 0x7375: ("section_bits", "u32"),
          0x7376: ("section_fill", "u8"), 0x7377: ("transparency_quality", "u32"),
          0x737C: ("background_photo", "u8"), 0x737D: ("background_photo_opacity", "f64"),
          0x737E: ("foreground_photo", "u8"), 0x737F: ("foreground_photo_opacity", "f64"),
          0x7382: ("paint_tool_active", "u8")}

SHADOW = {0x6591: ("time", "u32"), 0x6593: ("city", "str"), 0x6594: ("country", "str"),
          0x6595: ("longitude", "f64"), 0x6596: ("latitude", "f64"),
          0x6597: ("tz_offset_h", "f64"), 0x6599: ("shadows", "u8"),
          0x659A: ("north_shown", "u8"), 0x659B: ("on_faces", "u8"),
          0x659C: ("on_ground", "u8"), 0x659D: ("from_edges", "u8"),
          0x659E: ("light", "u32"), 0x659F: ("dark", "u32"), 0x65A0: ("use_sun", "u8")}

FONT = {0x05DC: ("id", "base"), 0x5015: ("family", "str"), 0x5016: ("bold", "u8"),
        0x5017: ("italic", "u8"), 0x5018: ("size_pt", "u32")}

ANCHOR = {0x5209: ("kind", "u32"), 0x520A: ("point", "v3")}

TEXT = {0x07D0: ("drawing", "drawing"), 0x55F1: ("text", "str"),
        0x55F2: ("screen_x", "f64"), 0x55F3: ("screen_y", "f64"),
        0x55F4: ("anchor", "anchor"), 0x55F5: ("leader_offset", "v3"),
        0x55F9: ("font", "id"), 0x55FA: ("arrow", "u32"), 0x55FC: ("leader_shown", "u8"),
        0x55FD: ("leader", "u32"), 0x55FE: ("leader_hidden", "u32")}

DIMENSION = {0x5BCD: ("start", "anchor"), 0x5BCE: ("end", "anchor"),
             0x5BCF: ("normal", "v3"), 0x5BD0: ("x_axis", "v3"),
             0x5BD2: ("offset", "f64"), 0x5BD4: ("text_position", "u32")}
DIM_BASE = {0x07D0: ("drawing", "drawing"), 0x59D9: ("text_override", "str"),
            0x59DA: ("font", "id"), 0x59DB: ("aligned_to_line", "u8"), 0x59DC: ("arrow", "u32")}

SECTION = {0x07D0: ("drawing", "drawing"), 0x445D: ("plane", "f64x4"),
           0x445E: ("name", "str"), 0x445F: ("symbol", "str")}

BEHAVIOUR = {0x1B5B: ("glues", "u8"), 0x1B59: ("glue_plane", "u32"),
             0x1B5C: ("cuts_opening", "u8"), 0x1B5D: ("faces_camera", "u8"),
             0x1B5E: ("shadows_face_sun", "u8")}

SCENE = {0x7149: ("properties", "u32"), 0x714A: ("camera", "camera"),
         0x714C: ("style", "id"), 0x714D: ("rendering", "rendering"),
         0x714E: ("shadow", "shadow"), 0x7151: ("section_planes", "ids"),
         0x7152: ("in_animation", "u8"), 0x7153: ("display_name", "str")}


def field(ty, p):
    if ty == "u8":
        return p[0] if p else None
    if ty == "u32":
        return struct.unpack("<I", p)[0]
    if ty == "f64":
        return struct.unpack("<d", p)[0]
    if ty == "f64x4":
        return list(struct.unpack("<4d", p))
    if ty == "v3":
        return list(f64s(p))
    if ty == "rgba":
        return list(p)
    if ty == "str":
        return p.decode("utf-8", "replace")
    if ty == "id":
        return uint(p)
    if ty == "ids":
        return id_list(p)
    if ty == "base":
        return entity_id(p)
    if ty == "drawing":
        h = Node(0x07D0, p)
        return {"id": entity_id(h.get(0x05DC)) if h.get(0x05DC) is not None else None,
                "material": uint(h.get(0x07D1)), "layer": uint(h.get(0x07D2)),
                "flags": uint(h.get(0x07D3))}
    if ty == "anchor":
        return fields(records(p)[0][1], ANCHOR)
    if ty == "camera":
        return fields(records(p)[0][1], CAMERA)
    if ty == "rendering":
        return fields(records(p)[0][1], RENDER)
    if ty == "shadow":
        return fields(records(p)[0][1], SHADOW)
    raise ValueError(ty)


def fields(payload, table):
    """Decode one settings record's children through `table`."""
    out = {}
    for t, p in records(payload):
        if t in table:
            name, ty = table[t]
            out[name] = field(ty, p)
        else:
            out[f"{t:#06x}"] = p.hex()
    return out


def settings(path):
    d = open(path, "rb").read()
    top = Node(0, records(open_zip(d).read("model.dat"))[0][1])
    out = {"camera": fields(top.node(0x01FA).get(0x34BC), CAMERA),
           "rendering": fields(top.node(0x01FB).get(0x733C), RENDER),
           "shadow": fields(top.node(0x0204).get(0x6590), SHADOW)}
    fonts = top.node(0x01FD)
    out["fonts"] = [fields(f, FONT) for f in fonts.node(0x4E20).node(0x4E21).all(0x5014)] if fonts else []
    root = top.node(0x01F6).node(0x1388)
    texts, dims, sections = root.node(0x1398), root.node(0x1399), root.node(0x1393)
    out["texts"] = [fields(t, TEXT) for t in texts.all(0x55F0)] if texts else []
    out["dimensions"] = []
    for dp in (dims.all(0x5BCC) if dims else []):
        rec = fields(dp, DIMENSION)
        rec.update(fields(Node(0, dp).get(0x59D8), DIM_BASE))
        rec.pop("0x59d8", None)
        out["dimensions"].append(rec)
    out["section_planes"] = [fields(s, SECTION) for s in sections.all(0x445C)] if sections else []
    pages = top.node(0x0207)
    out["scenes"] = [fields(pg, SCENE) for pg in (pages.node(0x6D60).node(0x6D61).all(0x7148) if pages else [])]
    out["behaviours"] = []
    dm = top.node(0x01F9)
    for dl in (dm.nodes(0x1770) if dm else []):
        for dd in dl.node(0x1771).nodes(0x157C):
            if dd.get(0x1B58):
                b = fields(dd.get(0x1B58), BEHAVIOUR)
                b["definition"] = dd.get(0x157E).decode("utf-8", "replace")
                b["kind"] = uint(dd.get(0x1583)) if dd.get(0x1583) else None
                out["behaviours"].append(b)
    return out

# ---------------------------------------------------------------- CLI

def tree(b, depth=0, maxdepth=99):
    for t, p in records(b):
        try:
            sub = records(p) if len(p) >= 6 and depth < maxdepth else None
            if sub is not None and any(tt == 0 for tt, _ in sub):
                sub = None
        except ValueError:
            sub = None
        text = f"  {p[:40].decode()!r}" if p and all(32 <= c < 127 for c in p[:40]) else ""
        print(f"{'  ' * depth}{t:#06x} [{len(p)}]" + ("" if sub else f" = {p[:24].hex(' ')}{text}"))
        if sub:
            tree(p, depth + 1, maxdepth)


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    cmd, path = argv[1], argv[2]
    if cmd == "tree":
        d = open(path, "rb").read()
        tree(open_zip(d).read("model.dat"), 0, int(argv[3]) if len(argv) > 3 else 99)
    elif cmd == "settings":
        print(json.dumps(settings(path), indent=1))
    elif cmd == "json":
        m = read(path)
        print(json.dumps(m, default=lambda o: o.hex() if isinstance(o, bytes) else str(o)))
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
