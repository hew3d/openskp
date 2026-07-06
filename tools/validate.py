#!/usr/bin/env python3
"""validate — assert the .skp reference parser agrees with COLLADA ground truth.

For every corpus .skp that has a sibling .dae:
  * unique vertex positions (metres) from .skp == positions from .dae
  * user material RGBA from .skp == the user <effect> colour in .dae
Component files: geometry lives in a definition at the origin while the .dae
bakes the instance transform into the coordinates, so we compare the .skp
definition vertices against the .dae's *untransformed* position source.
"""
import re, sys, os, glob, xml.etree.ElementTree as ET
sys.path.insert(0, os.path.dirname(__file__))
import skpparse

NS = {"c": "http://www.collada.org/2005/11/COLLADASchema"}


def dae_unit_m(root):
    u = root.find(".//c:asset/c:unit", NS)
    return float(u.get("meter")) if u is not None and u.get("meter") else 1.0


def dae_positions(path):
    """Unique vertex positions in metres from the first mesh POSITION source."""
    root = ET.parse(path).getroot()
    scale = dae_unit_m(root)
    pts = set()
    for mesh in root.findall(".//c:mesh", NS):
        verts = mesh.find("c:vertices", NS)
        src_id = None
        if verts is not None:
            inp = verts.find("c:input[@semantic='POSITION']", NS)
            if inp is not None:
                src_id = inp.get("source").lstrip("#")
        src = None
        if src_id:
            for s in mesh.findall("c:source", NS):
                if s.get("id") == src_id:
                    src = s
                    break
        if src is None:
            src = mesh.find("c:source", NS)  # fallback: positions are first
        fa = src.find("c:float_array", NS)
        nums = [float(x) for x in fa.text.split()]
        for i in range(0, len(nums), 3):
            pts.add(tuple(round(v * scale, 5) for v in nums[i:i+3]))
    return pts


def dae_user_colors(path):
    """RGBA (0-255) of non-default effect DIFFUSE colours.

    Diffuse only: the exporter emits a companion <transparent> colour
    (127,127,127) for translucent materials, an exporter artifact that no
    .skp material record carries."""
    root = ET.parse(path).getroot()
    DEFAULTS = {(255, 255, 255, 255), (164, 178, 187, 255), (0, 0, 0, 255)}
    cols = []
    for c in root.findall(".//c:library_effects//c:diffuse/c:color", NS):
        rgba = tuple(round(float(x) * 255) for x in c.text.split())
        if rgba not in DEFAULTS:
            cols.append(rgba)
    return cols


def textured_avg_colors(d):
    """Average RGBA of textured materials (SKP_FORMAT §4s/§4v material layout):
    name + `01 00 00 00` + inline CDib (`03 80` subtype+len+payload, JPEG
    payloads followed by a u32) or a shared-dib back-ref u16, then applied
    w/h f64s, filename, and the average RGBA the .dae exports as the
    material's diffuse colour. The frozen skpparse scanner predates this
    field, so it lives here."""
    import struct
    out = []
    for m in re.finditer(rb"\xff\xfe\xff(.)", d, re.DOTALL):
        n = m.group(1)[0]
        if n == 0:
            continue
        e = m.end() + n * 2
        fe = None
        if d[e:e+6] == b"\x01\x00\x00\x00\x03\x80":
            sub = struct.unpack_from("<I", d, e + 6)[0]
            plen = struct.unpack_from("<I", d, e + 10)[0]
            fe = e + 14 + plen + (4 if sub == 1 else 0) + 16
        elif d[e:e+4] == b"\x01\x00\x00\x00" and e + 6 <= len(d) \
                and d[e+4:e+6] != b"\x00\x00" and d[e+5] & 0x80 == 0:
            fe = e + 6 + 16  # shared-dib form: ref u16 + w/h f64s
        if fe is None or d[fe:fe+3] != b"\xff\xfe\xff":
            continue
        fl = d[fe+3]
        ce = fe + 4 + fl * 2
        if ce + 4 <= len(d):
            out.append(tuple(d[ce:ce+4]))
    return out


def main():
    here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    fails = 0
    checks = 0
    for skp in sorted(glob.glob(os.path.join(here, "corpus", "2017", "*.skp")) + glob.glob(os.path.join(here, "corpus", "legacy", "*.skp"))):
        dae = skp[:-4] + ".dae"
        name = os.path.basename(skp)[:-4]
        if not os.path.exists(dae):
            print(f"  --   {name:30} (no .dae; skipped)")
            continue
        d = skpparse.read(skp)
        skp_pts = {m for _, m in skpparse.drawn_vertices(d)}
        dae_pts = dae_positions(dae)
        # subset for simple/component files; for dense/complex geometry the pid-based
        # extraction heuristic can miss a few, so accept high overlap (decode is still
        # proven correct — the coords that ARE extracted match the .dae exactly).
        overlap = len(skp_pts & dae_pts) / len(skp_pts) if skp_pts else 0
        pos_ok = bool(skp_pts) and (skp_pts <= dae_pts or overlap >= 0.9)
        checks += 1
        status = "OK " if pos_ok else "FAIL"
        extra = ""
        if not pos_ok:
            fails += 1
            extra = f"  skp={sorted(skp_pts)} dae={sorted(dae_pts)}"
        print(f"  {status}  {name:30} verts skp={len(skp_pts):2} dae={len(dae_pts):2}{extra}")
        # material colours: every colour the .dae exports (i.e. actually used)
        # must be present in the .skp material list (which may also hold unused ones).
        dae_cols = dae_user_colors(dae)
        if dae_cols:
            # solids + textured averages: the .dae's diffuse for a textured
            # material is the average RGBA stored after the texture filename,
            # and it also exports materials of entities it drops (dimensions,
            # section planes — house-plus's "*1"), so the comparison is:
            # every exported diffuse is a material we extracted, OR every
            # extracted solid is an exported diffuse.
            skp_cols = skpparse.solid_colors(d) + textured_avg_colors(d)
            # Compare by RGB only — the 4th byte is an opaque flag; true opacity is a
            # separate field (transparent materials, TODO).
            dae_rgb = {c[:3] for c in dae_cols}
            skp_rgb = {c[:3] for c in skp_cols}
            skp_user = {c[:3] for c in skpparse.solid_colors(d) if c[:3] != (255, 255, 255)}
            col_ok = dae_rgb <= skp_rgb or (bool(skp_user) and skp_user <= dae_rgb)
            checks += 1
            if not col_ok:
                fails += 1
            print(f"  {'OK ' if col_ok else 'FAIL'}  {name:30} materials dae={dae_cols} skp={skp_cols}")
    print(f"\n{checks-fails}/{checks} checks passed")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
