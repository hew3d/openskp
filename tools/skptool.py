#!/usr/bin/env python3
"""skptool — clean-room analysis helpers for the SketchUp .skp format.

No Trimble SDK or SDK-derived knowledge is used. Everything here is derived
from observing files we authored ourselves. See ../docs/SKP_FORMAT.md.

Subcommands:
  id      <file>            magic + version string + model GUID
  classes <file>           enumerate MFC class names (CArchive new-class tags)
  diff    <a> <b>          byte diff with common prefix/suffix bounding
  window  <file> <off> <n> hex dump a window (off/n accept 0x.. hex)
  strings16 <file>         dump UTF-16LE length-prefixed string records
  rendering <file>         the document's display settings (SKP_FORMAT §10.7)
  styles  <file>           style records and their watermarks (SKP_FORMAT §10.8)
  scenes  <file>           shadow info and scene records (SKP_FORMAT §10.9, §10.10)
"""
import sys, re, struct


def read(path):
    with open(path, "rb") as f:
        return f.read()


def cmd_id(path):
    d = read(path)
    print(f"file: {path}  size={len(d)} (0x{len(d):x})")
    print(f"magic: {d[:4].hex()}  ({'UTF-16LE BOM FFFE + tag' if d[:2]==b'\xff\xfe' else 'unknown'})")
    # leading string records: FF FE FF <len:u8> <utf16le>
    off = 0
    for _ in range(2):
        if d[off:off+3] == b"\xff\xfe\xff":
            n = d[off+3]
            s = d[off+4:off+4+n*2].decode("utf-16le", "replace")
            print(f"  string @0x{off:x} len={n}: {s!r}")
            off += 4 + n*2
    # 16-byte value after the two strings = per-model GUID (see SKP_FORMAT §3)
    print(f"  model GUID @0x{off:x}: {d[off:off+16].hex()}")


def cmd_classes(path):
    d = read(path)
    # MFC new-class tag: 0xFFFF, schema u16, name-len u16, ascii name
    seen = {}
    for m in re.finditer(rb"\xff\xff(..)(..)", d):
        schema = struct.unpack("<H", m.group(1))[0]
        nlen = struct.unpack("<H", m.group(2))[0]
        if 3 <= nlen <= 40:
            name = d[m.end():m.end()+nlen]
            if re.fullmatch(rb"C[A-Za-z][A-Za-z0-9_]+", name):
                seen.setdefault(name.decode(), (schema, m.start()))
    for name, (schema, off) in sorted(seen.items()):
        print(f"  {name:24} schema={schema:<4} first@0x{off:x}")


def cmd_diff(a, b):
    da, db = read(a), read(b)
    n = min(len(da), len(db))
    i = 0
    while i < n and da[i] == db[i]:
        i += 1
    j = 0
    while j < n - i and da[-1-j] == db[-1-j]:
        j += 1
    print(f"sizes: {len(da)} vs {len(db)} (delta {len(db)-len(da)})")
    print(f"common prefix: {i} (0x{i:x})")
    print(f"common suffix: {j} (0x{j:x})")
    print(f"changed window A: [0x{i:x}..0x{len(da)-j:x}] len {len(da)-j-i}")
    print(f"changed window B: [0x{i:x}..0x{len(db)-j:x}] len {len(db)-j-i}")
    for tag, dd in (("A", da), ("B", db)):
        for m in re.finditer(b"\x89PNG\r\n\x1a\n", dd[i:len(dd)-j]):
            print(f"  PNG in {tag} window @ 0x{i+m.start():x}")


INCH = 39.37007874015748  # 1 metre in inches (SketchUp internal unit)
# Class tags validated against the box oracle (8 V / 12 E / 6 F). map-index based,
# stable across this corpus because the default-template preamble is identical.
ENTITY_CLASS = {0x8020: "CVertex", 0x801e: "CEdge", 0x8089: "CFace"}
TEMPLATE_BASELINE_PID = 0x1281  # empty.skp max pid; drawn entities exceed this


def _scan_entities(d):
    """Find entity headers: <classtag:u16 0x8000-set> 00 00 03 <pid:u16>."""
    out = []
    off = 0
    while off < len(d) - 7:
        v = struct.unpack_from("<H", d, off)[0]
        if (v & 0x8000) and d[off+2:off+5] == b"\x00\x00\x03":
            pid = struct.unpack_from("<H", d, off+5)[0]
            out.append((off, v, pid))
            off += 7
            continue
        off += 1
    return out


def cmd_geom(path, baseline=None):
    """List user-drawn entities (pid above the default-template baseline)."""
    d = read(path)
    base = int(baseline, 0) if baseline else TEMPLATE_BASELINE_PID
    ents = sorted((e for e in _scan_entities(d) if e[2] > base), key=lambda e: e[2])
    print(f"{path}: {len(ents)} drawn entities (pid > {base:#x})")
    for k, (off, tag, pid) in enumerate(ents):
        nxt = ents[k+1][0] if k+1 < len(ents) else off + 60
        body = d[off+7:nxt if nxt > off else off+60]
        name = ENTITY_CLASS.get(tag, f"class{tag:#06x}")
        extra = ""
        if tag == 0x8020 and len(body) >= 24:
            x, y, z = struct.unpack_from("<ddd", body, 0)
            extra = f"  xyz=({x/INCH:.3f}, {y/INCH:.3f}, {z/INCH:.3f}) m"
        else:
            extra = f"  body[{len(body)}]={body[:24].hex()}"
        print(f"  pid={pid:#06x} {name:8} @0x{off:06x}{extra}")


def cmd_window(path, off, n):
    d = read(path)
    off = int(off, 0); n = int(n, 0)
    chunk = d[off:off+n]
    for k in range(0, len(chunk), 16):
        row = chunk[k:k+16]
        hexs = " ".join(f"{x:02x}" for x in row)
        asc = "".join(chr(x) if 32 <= x < 127 else "." for x in row)
        print(f"{off+k:08x}: {hexs:<48} {asc}")


def cmd_strings16(path):
    d = read(path)
    for m in re.finditer(b"\xff\xfe\xff(.)", d, re.DOTALL):
        n = m.group(1)[0]
        s = d[m.end():m.end()+n*2]
        try:
            txt = s.decode("utf-16le")
        except Exception:
            continue
        if txt.isprintable():
            print(f"0x{m.start():08x} len={n:3} {txt!r}")


def _newclass_defs(d):
    """Find MFC new-class definitions: FFFF <schema:u16> <namelen:u16> <ascii>.
    High confidence: the name must be valid and its length must match. Returns
    [(offset, schema, name)] in file order = order MFC first emitted each class."""
    out = []
    i = 0
    while True:
        i = d.find(b"\xff\xff", i)
        if i < 0:
            break
        if i + 6 <= len(d):
            schema = struct.unpack_from("<H", d, i + 2)[0]
            nlen = struct.unpack_from("<H", d, i + 4)[0]
            if 2 <= nlen <= 40 and i + 6 + nlen <= len(d):
                name = d[i + 6:i + 6 + nlen]
                if re.fullmatch(rb"C[A-Za-z][A-Za-z0-9_]+", name):
                    out.append((i, schema, name.decode()))
                    i += 6 + nlen
                    continue
        i += 2
    return out


def cmd_walk(path):
    """List new-class definitions in file order (the CArchive class table as it
    is built). NOTE: only the FIRST instance of each class carries a name tag;
    later instances use back-references whose body layout we haven't mapped yet."""
    d = read(path)
    defs = _newclass_defs(d)
    print(f"{path}: {len(defs)} class definitions")
    for off, schema, name in defs:
        print(f"  0x{off:06x}  schema={schema:<3} {name}")


def cmd_classdelta(a, b):
    """Which object *types* does B introduce that A lacks (and vice versa)."""
    ca = {n for _, _, n in _newclass_defs(read(a))}
    cb = {n for _, _, n in _newclass_defs(read(b))}
    print(f"{a}: {len(ca)} classes   {b}: {len(cb)} classes")
    add = sorted(cb - ca); rem = sorted(ca - cb)
    print(f"  + introduced by {b}: {add or '(none)'}")
    print(f"  - only in {a}:      {rem or '(none)'}")


# SKP_FORMAT §10.7: offset from the field block's start -> (type, name).
# The block starts 3 bytes after the 33-byte tail of the file's first
# CCamera (the current view); validated against the 2026 conversion
# of every corpus model and 51 one-change pairs.
RENDERING = [
    (0, "u32", "face_style"), (4, "u8", "xray"), (5, "u8", "transparency"),
    (6, "u8", "jitter"), (7, "u32", "edges"), (11, "rgba", "background"),
    (15, "rgba", "edge_color"), (19, "rgba", "selected_color"),
    (23, "rgba", "guides_color"), (27, "u8", "shadows_displayed"),
    (29, "u8", "color_by_layer"), (30, "u8", "textures"),
    (31, "u32", "edge_color_mode"), (35, "u8", "extension"), (36, "u32", "extension_length"),
    (40, "u8", "profiles"), (41, "u32", "profiles_width"), (45, "u8", "depth_cue"),
    (46, "u32", "depth_cue_width"), (50, "u8", "endpoints"), (51, "u32", "endpoints_length"),
    (56, "u8", "hidden_geometry"), (61, "rgba", "front_color"), (65, "rgba", "back_color"),
    (69, "f64", "0x736c"), (77, "f64", "0x736d"), (85, "u8", "hide_rest_of_model"),
    (86, "u8", "hide_similar"), (87, "u8", "fog"), (88, "rgba", "fog_color"),
    (92, "u8", "fog_uses_background"), (93, "f64", "fog_start"), (101, "f64", "fog_end"),
    (109, "u32", "0x7364"), (117, "u8", "model_axes"), (118, "u8", "0x7344"),
    (119, "u8", "0x7345"), (120, "u8", "guides_hidden"), (121, "rgba", "sky_color"),
    (125, "u32", "0x7366"), (129, "rgba", "ground_color"), (133, "u8", "sky"),
    (134, "u8", "ground"), (135, "u8", "ground_from_below"), (136, "u32", "ground_transparency"),
    (140, "rgba", "active_section_color"), (144, "rgba", "inactive_section_color"),
    (148, "rgba", "section_cut_color"), (152, "u32", "section_cut_width"),
    (156, "u32", "section_bits"), (160, "u32", "transparency_quality"), (164, "u8", "0x7379"),
    (165, "f64", "0x737a"), (173, "u8", "0x737b"), (174, "rgba", "locked_color"),
    (178, "u8", "watermarks"), (179, "f64", "0x7378"), (187, "u8", "back_edges"),
    (188, "u8", "background_photo"), (189, "f64", "background_photo_opacity"),
    (197, "u8", "foreground_photo"), (198, "f64", "foreground_photo_opacity"),
]


def rendering_block(d):
    """Offset of the §10.7 field block, or None."""
    i = d.find(b"CCamera")
    if i < 0:
        return None
    after = i + 7 + 137                      # camera body
    if d[after + 2:after + 5] != b"\xff\xfe\xff":   # u16, then the description string
        return None
    return after + 6 + 2 * d[after + 5] + 33 + 3


def cmd_rendering(path):
    d = read(path)
    base = rendering_block(d)
    if base is None:
        print(f"{path}: no CCamera"); return
    fmt = {"u8": ("<B", 1), "u32": ("<I", 4), "f64": ("<d", 8)}
    for off, ty, name in RENDERING:
        at = base + off
        v = d[at:at + 4].hex() if ty == "rgba" else struct.unpack(fmt[ty][0], d[at:at + fmt[ty][1]])[0]
        print(f"  0x{at:06x} +{off:<3} {name:26s} {v}")


def _str16(d, i):
    """MFC CString at i (FF FE FF, u8 length or FF + u16) -> (text, end)."""
    if d[i:i + 3] != b"\xff\xfe\xff":
        raise ValueError(f"no string at 0x{i:x}")
    n, i = d[i + 3], i + 4
    if n == 0xFF:
        n, i = struct.unpack_from("<H", d, i)[0], i + 2
    return d[i:i + 2 * n].decode("utf-16le"), i + 2 * n


def _obj_ref(d, i):
    """Object reference at i -> (kind, end): 'null', 'new' (class defined
    or referenced here, body follows) or 'back' (earlier object)."""
    t = struct.unpack_from("<H", d, i)[0]
    if t == 0:
        return "null", i + 2
    if t == 0xFFFF:                              # new class: schema, name
        n = struct.unpack_from("<H", d, i + 4)[0]
        return "new", i + 6 + n
    if t == 0x7FFF:                              # big tag: u32 follows
        v = struct.unpack_from("<I", d, i + 2)[0]
        return ("new" if v & 0x80000000 else "back"), i + 6
    return ("new" if t & 0x8000 else "back"), i + 2


def _watermark(d, i):
    """CWatermark body (§10.8) -> (fields, end)."""
    w = {"lead": struct.unpack_from("<I", d, i)[0]}
    w["path"], i = _str16(d, i + 4)
    w["time"], w["position"] = struct.unpack_from("<II", d, i)
    for k, name in enumerate(("tiled", "stretched", "keep_aspect", "background", "mask")):
        w[name] = d[i + 8 + k]
    w["blend"], w["scale"] = struct.unpack_from("<dd", d, i + 13)
    w["name"], i = _str16(d, i + 29)
    kind, i = _obj_ref(d, i)                     # the CDib
    if kind == "new":
        sub, n = struct.unpack_from("<II", d, i)
        w["image"] = f"subtype {sub}, {n} bytes"
        i += 8 + n
    else:
        w["image"] = "shared"
    return w, i


STYLE_SIG = re.compile(rb"\x00\x00\x00.{16}\xff\xfe\xff\x00\x03\x00\x00\x00\xff\xfe\xff", re.DOTALL)


def style_records(d):
    """Every CSkpStyle body (§10.8) -> list of (offset, record, end)."""
    out = []
    for m in STYLE_SIG.finditer(d):
        i = m.start()
        rec = {"guid": d[i + 3:i + 19].hex()}
        try:
            _, j = _str16(d, i + 19)
            rec["name"], j = _str16(d, j + 4)
            rec["description"], j = _str16(d, j)
            n, j = struct.unpack_from("<I", d, j)[0], j + 4
            items, wms = {}, []
            for _ in range(n):
                key, count = struct.unpack_from("<II", d, j)
                j += 8
                vals = []
                for _ in range(count):
                    if key == 5001:
                        kind, j = _obj_ref(d, j)
                        if kind == "null":
                            vals.append("<model>")
                        elif kind == "new":
                            w, j = _watermark(d, j)
                            wms.append(w)
                            vals.append(w["name"])
                        else:
                            vals.append("<back-reference>")
                        continue
                    ty = struct.unpack_from("<I", d, j)[0]
                    fmt, size = {1: ("<B", 1), 4: ("<I", 4), 7: ("<d", 8)}[ty]
                    vals.append(struct.unpack_from(fmt, d, j + 4)[0])
                    j += 4 + size
                items[key] = vals
        except (ValueError, KeyError, struct.error):
            continue
        rec["items"], rec["watermarks"] = items, wms
        out.append((i, rec, j))
    return out


def cmd_styles(path):
    d = read(path)
    recs = style_records(d)
    for k, (off, rec, end) in enumerate(recs):
        print(f"0x{off:06x} {rec['name']!r} guid={rec['guid']} items={len(rec['items'])}")
        for key, vals in rec["items"].items():
            print(f"    {key:5} {vals}")
        for w in rec["watermarks"]:
            print(f"    watermark {w}")
        if k == len(recs) - 1:
            print(f"  unsaved changes: {struct.unpack_from('<I', d, end)[0]}")


def shadow_record(d, i):
    """§10.9 shadow record at i -> (fields, end)."""
    r = {"time": struct.unpack_from("<I", d, i + 3)[0], "u8": d[i + 7]}
    r["city"], j = _str16(d, i + 8)
    r["country"], j = _str16(d, j)
    r["longitude"], r["latitude"], r["tz"] = struct.unpack_from("<3d", d, j)
    r["north"] = struct.unpack_from("<3d", d, j + 24)
    j += 48
    for k, name in enumerate(("north_shown", "on_faces", "on_ground", "from_edges")):
        r[name] = d[j + k]
    r["light"], r["dark"] = struct.unpack_from("<II", d, j + 4)
    r["use_sun"] = d[j + 12]
    return r, j + 14


def shadow_records(d):
    """Offsets of every §10.9 record: 00 00 00, a plausible UNIX time, a
    u8, then two strings. The first is the document's."""
    out = []
    for m in re.finditer(rb"\x00\x00\x00(....).\xff\xfe\xff", d, re.DOTALL):
        if 1e9 < struct.unpack("<I", m.group(1))[0] < 3e9:
            try:
                shadow_record(d, m.start())
            except (ValueError, struct.error):
                continue
            out.append(m.start())
    return out


def _ref_list(d, i):
    n, i = struct.unpack_from("<I", d, i)[0], i + 4
    refs = []
    for _ in range(n):
        _, j = _obj_ref(d, i)
        refs.append(d[i:j].hex())
        i = j
    return refs, i


def scene_record(d, i, style_ends):
    """§10.10 CViewPage body at i -> fields."""
    s = {}
    s["name"], i = _str16(d, i + 3)
    s["description"], i = _str16(d, i)
    f = s["flags"] = struct.unpack_from("<I", d, i)[0]
    i += 4
    if f & 1:                                      # CCamera object
        _, i = _obj_ref(d, i)
        s["eye"] = struct.unpack_from("<3d", d, i)
        s["target"] = struct.unpack_from("<3d", d, i + 24)
        _, i = _str16(d, i + 139)
        i += 33
    if f & 2:                                      # rendering fields, style
        s["face_style"] = struct.unpack_from("<I", d, i + 3)[0]
        i += 3 + 206
        kind, j = _obj_ref(d, i)
        i = style_ends[j] if kind == "new" else j
        s["style"] = kind
    if f & 4:
        s["shadow"], i = shadow_record(d, i)
    if f & 8:
        vals = struct.unpack_from("<12d", d, i + 13)
        s["axes"] = [vals[k:k + 3] for k in range(0, 12, 3)]
        i += 110
    for bit, name in ((16, "hidden"), (32, "layers"), (64, "section_planes")):
        if f & bit:
            s[name], i = _ref_list(d, i)
    s["animation"] = d[i]
    s["transition"] = struct.unpack_from("<2d", d, i + 1)
    s["thumbnail"] = d[i + 19]
    if s["thumbnail"]:
        s["thumbnail_bytes"] = struct.unpack_from("<I", d, i + 22)[0]
    return s


def scene_offsets(d):
    """Offsets of CViewPage bodies (§10.10): a string record preceded by
    the 3-byte zero preamble and either the CViewPage new-class record or
    a class reference; the name only appears once, so later scenes are
    found by this shape."""
    out = []
    for m in re.finditer(rb"\x00\x00\x00\xff\xfe\xff", d):
        i = m.start()
        new_class = d[i - 15:i] == b"\xff\xff\x0c\x00\x09\x00CViewPage"
        tag = struct.unpack_from("<H", d, i - 2)[0] if i >= 2 else 0
        big = i >= 6 and d[i - 6:i - 4] == b"\xff\x7f" and d[i - 1] & 0x80
        if new_class or big or (tag & 0x8000 and tag != 0xFFFF):
            out.append(i)
    return out


def cmd_scenes(path):
    d = read(path)
    shadows = shadow_records(d)
    if shadows:
        print(f"document shadow info 0x{shadows[0]:06x}: {shadow_record(d, shadows[0])[0]}")
    style_ends = {off: end for off, _, end in style_records(d)}
    for i in scene_offsets(d):
        try:
            s = scene_record(d, i, style_ends)
        except (ValueError, KeyError, struct.error, UnicodeDecodeError):
            continue
        if s["flags"] <= 0xFFFF and s["animation"] <= 1 and s["thumbnail"] <= 1:
            print(f"0x{i:06x} {s}")


if __name__ == "__main__":
    args = sys.argv[1:]
    if not args:
        print(__doc__); sys.exit(1)
    cmd, rest = args[0], args[1:]
    {
        "id": cmd_id, "classes": cmd_classes, "diff": cmd_diff,
        "window": cmd_window, "strings16": cmd_strings16,
        "walk": cmd_walk, "classdelta": cmd_classdelta, "geom": cmd_geom,
        "rendering": cmd_rendering, "styles": cmd_styles, "scenes": cmd_scenes,
    }[cmd](*rest)
