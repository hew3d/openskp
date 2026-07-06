#!/usr/bin/env python3
"""skpparse — reference parser for SketchUp 2017 .skp (clean-room).

Extracts header + user-drawn geometry/materials by walking the MFC CArchive
entity stream. See ../docs/SKP_FORMAT.md for the format. Geometry is returned in
METRES (kernel-friendly); .skp stores f64 INCHES.

This is a *semantic* extractor (counts, coordinates, materials) used to validate
the format model against the COLLADA ground truth — not a full byte-exact
round-tripping serializer (the MFC object-map back-references would need a
stateful ReadObject; see validate.py for what we assert)."""
import struct, re

INCH = 39.37007874015748          # 1 metre in inches
TEMPLATE_BASELINE_PID = 0x1281    # max entity pid in the 2017 default template


def read(path):
    with open(path, "rb") as f:
        return f.read()


def parse_header(d):
    """version string + format GUID. Header = FF FE FF <len:u8> <utf16> records."""
    out = {}
    off = 0
    names = ["model_tag", "version"]
    for key in names:
        assert d[off:off+3] == b"\xff\xfe\xff", f"bad string record @{off:#x}"
        n = d[off+3]
        out[key] = d[off+4:off+4+n*2].decode("utf-16le")
        off += 4 + n*2
    out["format_guid"] = d[off:off+16].hex()
    return out


def scan_entities(d):
    """All entity headers: <classtag:u16, 0x8000 set> 00 00 03 <pid:u16>.
    Finds top-level AND inline-nested entities (e.g. vertices embedded in edges)."""
    out = []
    off = 0
    n = len(d)
    while off < n - 7:
        v = d[off] | (d[off+1] << 8)
        if (v & 0x8000) and d[off+2] == 0 and d[off+3] == 0 and d[off+4] == 3:
            pid = d[off+5] | (d[off+6] << 8)
            out.append((off, v, pid))
            off += 7
            continue
        off += 1
    return out


def drawn_vertices(d, baseline=TEMPLATE_BASELINE_PID):
    """User-drawn CVertex positions in METRES, de-duplicated.

    A CVertex body is 3 f64 coords. We classify by content (not class tag, which
    shifts per file): the 24-byte body must decode to 3 coords each == 0 or with
    1e-4 < |inches| < 1e4 (model-scale), which excludes edge/face bodies (whose
    leading bytes form denormal/huge doubles)."""
    verts = []
    seen = set()
    for off, tag, pid in scan_entities(d):
        if pid <= baseline:
            continue
        body = d[off+7:off+7+24]
        if len(body) < 24:
            continue
        xyz = struct.unpack("<ddd", body)
        ok = all(v == 0.0 or (1e-4 < abs(v) < 1e4) for v in xyz)
        if not ok:
            continue
        m = tuple(round(v / INCH, 5) for v in xyz)
        if m not in seen:
            seen.add(m)
            verts.append((pid, m))
    return verts


def materials(d):
    """All materials as dicts. Two on-disk shapes distinguish them:
      solid:    <name> 00 00 <RGBA:4> <empty texture-path string>
      textured: <name> 01 00 00 00 <CDib tag 0x8003> <image> ... <filename>
    Returns [{name, kind, rgba?, texture?}] (includes SketchUp's 'Default')."""
    tex_re = re.compile(rb"\xff\xfe\xff(.)", re.DOTALL)
    out = []
    for m in re.finditer(b"\xff\xfe\xff(.)", d, re.DOTALL):
        n = m.group(1)[0]
        if n == 0:
            continue
        try:
            name = d[m.end():m.end()+n*2].decode("utf-16le")
        except Exception:
            continue
        if not name.isprintable():
            continue
        e = m.end() + n*2
        if d[e:e+2] == b"\x00\x00" and d[e+6:e+10] == b"\xff\xfe\xff\x00":
            # opacity is a separate f64 (name + 00 00 + RGBA + empty texpath + 8B + f64);
            # the RGBA 4th byte is an opaque flag, real transparency lives here.
            opacity = round(struct.unpack_from("<d", d, e+18)[0], 4)
            out.append({"name": name, "kind": "solid", "rgba": tuple(d[e+2:e+6]),
                        "opacity": opacity})
        elif d[e:e+6] == b"\x01\x00\x00\x00\x03\x80":
            tex = None
            fm = re.search(rb"\xff\xfe\xff(.)", d[e:e+40000])
            for fm in re.finditer(rb"\xff\xfe\xff(.)", d[e:e+40000]):
                ln = fm.group(1)[0]
                try:
                    s = d[e+fm.end():e+fm.end()+ln*2].decode("utf-16le")
                except Exception:
                    continue
                if s.lower().endswith((".jpg", ".jpeg", ".png", ".tif", ".bmp")):
                    tex = s
                    break
            # applied texture size (inches): CDib len at e+10, image at e+14,
            # then (after the image + a u32) two f64 = width, height.
            asize = None
            try:
                ln = struct.unpack_from("<I", d, e+10)[0]
                je = e + 14 + ln
                asize = (round(struct.unpack_from("<d", d, je+4)[0], 3),
                         round(struct.unpack_from("<d", d, je+12)[0], 3))
            except Exception:
                pass
            out.append({"name": name, "kind": "textured", "texture": tex,
                        "applied_size_in": asize})
    return out


def solid_colors(d):
    """RGBA tuples of the solid materials (convenience for cross-checks)."""
    return [m["rgba"] for m in materials(d) if m["kind"] == "solid"]


def scenes(d):
    """Scene/page names (CSketchUpPage/CViewPage). A scene record = name string +
    empty description string + scene flags `7f 00 00 00` + an embedded CCamera
    (a class-ref tag). That flags+camera tail distinguishes scenes from the many
    other name+empty-string records (groups, collections, ...)."""
    out = []
    for m in re.finditer(b"\xff\xfe\xff(.)", d, re.DOTALL):
        n = m.group(1)[0]
        if n == 0:
            continue
        try:
            name = d[m.end():m.end()+n*2].decode("utf-16le")
        except Exception:
            continue
        e = m.end() + n*2
        if (name.isprintable() and d[e:e+8] == b"\xff\xfe\xff\x00\x7f\x00\x00\x00"
                and (struct.unpack_from("<H", d, e+8)[0] & 0x8000)):
            out.append(name)
    return out


def guides(d):
    """Construction lines (CConstructionLine): {point_m, direction}. Layout after
    the class name: entity preamble + CDrawingElement drawbase(10) + point(3 f64,
    inches) + direction(3 f64, unit vector)."""
    out = []
    for m in re.finditer(b"CConstructionLine", d):
        p = m.end() + 5 + 10          # skip preamble (00 00 03 + pid) + drawbase
        try:
            pt = struct.unpack_from("<3d", d, p)
            dr = struct.unpack_from("<3d", d, p + 24)
        except Exception:
            continue
        if all(abs(x) < 1e5 for x in pt) and abs(sum(x*x for x in dr) ** 0.5 - 1) < 1e-6:
            out.append({"point_m": tuple(round(x / INCH, 5) for x in pt),
                        "direction": tuple(round(x, 5) for x in dr)})
    return out


def attributes(d):
    """(key, type, value) from attribute dictionaries — model options, geo-location,
    and dynamic-component parameters. Anchors on a UTF-16 key string immediately
    followed by a typed value: `0x04`=int32, `0x06`=f64, `0x07`=bool. Skips
    `C[A-Z]…` class names (whose schema byte collides with a type tag) and
    out-of-range values, so it needs no container/nesting model."""
    out = []
    for m in re.finditer(b"\xff\xfe\xff(.)", d, re.DOTALL):
        n = m.group(1)[0]
        if n == 0:
            continue
        try:
            key = d[m.end():m.end()+n*2].decode("utf-16le")
        except Exception:
            continue
        if not key.isprintable() or re.match(r"C[A-Z]", key):
            continue
        e = m.end() + n*2
        t = d[e] if e < len(d) else 0
        if t == 0x04:
            v = struct.unpack_from("<i", d, e+1)[0]
            if abs(v) < 1_000_000:
                out.append((key, "int", v))
        elif t == 0x06:
            v = struct.unpack_from("<d", d, e+1)[0]
            if v == v and abs(v) < 1e6:
                out.append((key, "f64", round(v, 6)))
        elif t == 0x07 and d[e+1] in (0, 1):
            out.append((key, "bool", bool(d[e+1])))
    return out


def layers(d):
    """Model layers (CLayer): {name, rgba, flags}. A layer stores its internal
    name `Layer_<name>` followed by 2 flag bytes + RGBA. Deduped by name (the
    internal name is echoed by each definition's layer pointer)."""
    out, seen = [], set()
    for m in re.finditer(b"\xff\xfe\xff(.)", d, re.DOTALL):
        n = m.group(1)[0]
        if n == 0:
            continue
        try:
            s = d[m.end():m.end()+n*2].decode("utf-16le")
        except Exception:
            continue
        if s.startswith("Layer_") and s[6:] not in seen and m.start() >= 4:
            e = m.end() + n*2
            seen.add(s[6:])
            # the u32 just before the internal 'Layer_<name>' string is the hidden
            # flag (1 = hidden); the 2 bytes after it are a colour flag, not visibility.
            hidden = struct.unpack_from("<I", d, m.start()-4)[0]
            out.append({"name": s[6:], "visible": hidden == 0,
                        "rgba": tuple(d[e+2:e+6])})
    return out


if __name__ == "__main__":
    import sys
    d = read(sys.argv[1])
    h = parse_header(d)
    vs = drawn_vertices(d)
    print(f"version {h['version']}  guid {h['format_guid']}")
    print(f"{len(vs)} unique drawn vertices (m):")
    for pid, m in sorted(vs):
        print(f"  pid={pid:#06x} {m}")
    for mat in materials(d):
        extra = mat.get("rgba") or mat.get("texture")
        print(f"material {mat['name']!r} ({mat['kind']}) {extra}")
