#!/usr/bin/env python3
"""skp26_check — compare a 2026 save against its decoded 2017 original.

The 2017 side comes from the shipped reader (`openskp mesh`), the 2026 side
from tools/skp26.py. Every entity container (a definition's or the model's
entities) is reduced to a content signature — its faces' vertex rings (with
winding), holes and hidden flag, and its edges' endpoints with soft, smooth
and hidden flags — and the multisets of container signatures must be equal.
Persistent ids mostly survive the conversion but are renumbered where the
2017 file held duplicates, so content, not pid, is the pairing.

Usage: python3 tools/skp26_check.py <2017.skp> <2026.skp> [openskp-binary]
"""
import collections
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(__file__))
import skp26  # noqa: E402

INCH = 0.0254


def key(p):
    return tuple(round(c, 6) + 0.0 for c in p)


def ring_key(pts):
    """Rotation-invariant key of a closed vertex ring (winding kept)."""
    ks = [key(p) for p in pts]
    i = ks.index(min(ks))
    return tuple(ks[i:] + ks[:i])


def ring_2026(c, loop):
    """Vertex ring (metres) of a 2026 loop: each edge use contributes its
    start vertex; the orientation flag says which end that is."""
    out = []
    for eid, orient in loop:
        v0, v1 = c["edges"][eid]["v"]
        out.append([x * INCH for x in c["vertices"][v0 if orient else v1]])
    return out


def flags26(b):
    """2026 drawing-element flags byte -> (soft, smooth, hidden)."""
    return (bool(b & 0x08), bool(b & 0x10), bool(b & 0x01))


def sig_2017(r):
    V = r["vertices_m"]
    faces = sorted((ring_key([V[i] for i in f["outer"]]),
                    tuple(sorted(ring_key([V[i] for i in h]) for h in f["holes"])),
                    f["hidden"]) for f in r["faces"])
    edges = sorted((tuple(sorted((key(V[e["v0"]]), key(V[e["v1"]])))),
                    e["soft"], e["smooth"], e["hidden"]) for e in r["edges"])
    return (tuple(faces), tuple(edges))


def sig_2026(c):
    faces = []
    for f in c["faces"].values():
        rings = [ring_2026(c, lp) for lp in f["loops"]]
        # Inner loops are stored with the opposite sense to the 2017 mesh's
        # hole rings (which follow the COLLADA export).
        faces.append((ring_key(rings[0]), tuple(sorted(ring_key(r[::-1]) for r in rings[1:])),
                      flags26(f["flags"])[2]))
    edges = []
    for e in c["edges"].values():
        p, q = ([x * INCH for x in c["vertices"][v]] for v in e["v"])
        edges.append((tuple(sorted((key(p), key(q)))),) + flags26(e["flags"]))
    return (tuple(sorted(faces)), tuple(sorted(edges)))


def mat_of(t):
    """13-f64 instance transform -> row-major 4x4, translation in metres
    (the same convention as the 2017 reader's scene)."""
    return [t[0], t[1], t[2], t[9] * INCH, t[3], t[4], t[5], t[10] * INCH,
            t[6], t[7], t[8], t[11] * INCH, 0.0, 0.0, 0.0, 1.0]


def mat_mul(a, b):
    return [sum(a[r * 4 + k] * b[k * 4 + c] for k in range(4)) for r in range(4) for c in range(4)]


IDENTITY = [1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0]


def scene_2026(m26, sigs):
    """(container signature, world matrix) for every placed container,
    root first, composing instance transforms depth-first."""
    by_id = {d["id"]: d for d in m26["definitions"]}
    out = []

    def rec(c, world, depth):
        if id(c) in sigs:
            out.append((sigs[id(c)], tuple(round(x, 6) + 0.0 for x in world)))
        if depth > 64:
            return
        for i in c["instances"]:
            rec(by_id[i["definition"]], mat_mul(world, mat_of(i["transform"])), depth + 1)
    rec(m26["root"], IDENTITY, 0)
    return collections.Counter(out)


def r6(xs):
    return tuple(round(x, 6) + 0.0 for x in xs)


def lname(x):
    """Default-layer entities carry no layer ref in either format."""
    return x or "Layer0"


def tex_2017(t):
    if t is None:
        return None
    return (r6(t["front"]), r6(t["back"]), r6(t["front_extra"]), r6(t["back_extra"]),
            tuple(r6(p) for p in t["front_pins"]), tuple(r6(p) for p in t["back_pins"]))


def tex_2026(uv):
    if not uv:
        return None
    side = lambda s: uv.get(s) or {"matrix": [0.0] * 9, "extra": [0.0] * 3, "pins": []}  # noqa: E731
    f, b = side("front"), side("back")
    return (r6(f["matrix"]), r6(b["matrix"]), r6(f["extra"]), r6(b["extra"]),
            tuple(r6(p[0] + p[1]) for p in f["pins"]), tuple(r6(p[0] + p[1]) for p in b["pins"]))


def appearance(oracle, m26):
    """Per-face (ring, front, back, layer, texture) and per-edge (ends,
    layer) multisets from both sides, plus the material and layer tables."""
    a17, a26 = collections.Counter(), collections.Counter()
    for r in oracle["runs"]:
        for f in r["faces"]:
            a17[("face", ring_key(f["outer"]), f["front"], f["back"], lname(f["layer"]), tex_2017(f["texture"]))] += 1
        for e in r["edges"]:
            a17[("edge", tuple(sorted((key(e["a"]), key(e["b"])))), lname(e["layer"]))] += 1
    mats = m26["materials"]
    lays = m26["layers"]
    mn = lambda i: mats[i]["name"] if i in mats else None  # noqa: E731
    ln = lambda i: lays[i]["name"] if i in lays else None  # noqa: E731
    for c in [m26["root"]] + m26["definitions"]:
        for f in c["faces"].values():
            ring = ring_2026(c, f["loops"][0])
            a26[("face", ring_key(ring), mn(f["front"]), mn(f["back"]), lname(ln(f["layer"])), tex_2026(f["uv"]))] += 1
        for e in c["edges"].values():
            p, q = ([x * INCH for x in c["vertices"][v]] for v in e["v"])
            a26[("edge", tuple(sorted((key(p), key(q)))), lname(ln(e["layer"])))] += 1
    return a17, a26


def main(argv):
    old, new = argv[1], argv[2]
    exe = argv[3] if len(argv) > 3 else os.path.join(os.path.dirname(__file__), "../target/release/openskp")
    mesh = json.loads(subprocess.run([exe, "mesh", old], capture_output=True, text=True, check=True).stdout)
    m26 = skp26.read(new)

    s17 = collections.Counter(sig_2017(r) for r in mesh["runs"] if r["faces"] or r["edges"])
    s26 = collections.Counter(sig_2026(c) for c in [m26["root"]] + m26["definitions"] if c["faces"] or c["edges"])
    sig17 = {ri: sig_2017(r) for ri, r in enumerate(mesh["runs"]) if r["faces"] or r["edges"]}
    sig26 = {id(c): sig_2026(c) for c in [m26["root"]] + m26["definitions"] if c["faces"] or c["edges"]}
    scene17 = collections.Counter((sig17[n["run"]], tuple(round(x, 6) + 0.0 for x in n["world"]))
                                  for n in mesh["scene"] if n.get("run") in sig17)
    scene26 = scene_2026(m26, sig26)
    print(f"placed leaves 2017={sum(scene17.values())} 2026={sum(scene26.values())}  "
          + ("scene equivalent" if scene17 == scene26 else
             f"SCENE DIFFERS: {sum((scene17 - scene26).values())} only in 2017, {sum((scene26 - scene17).values())} only in 2026"))
    oracle = json.loads(subprocess.run([os.path.join(os.path.dirname(exe), "examples/oracle17"), old],
                                       capture_output=True, text=True, check=True).stdout)
    a17, a26 = appearance(oracle, m26)
    ok_app = a17 == a26
    print("face materials/layers/UVs and edge layers: " + ("equivalent" if ok_app else
          f"DIFFER ({sum((a17 - a26).values())} only in 2017, {sum((a26 - a17).values())} only in 2026)"))
    if not ok_app:
        for k in list(a17 - a26)[:3]:
            print("   2017:", str(k)[:300])
        for k in list(a26 - a17)[:3]:
            print("   2026:", str(k)[:300])
    ntex = sum(n for k, n in a17.items() if k[0] == "face" and k[5] is not None)
    print(f"   (textured faces compared: {ntex})")
    # Instance appearance: the 2017 scene nodes' effective (inherited)
    # material, layer and own hidden flag, per definition and world matrix.
    mats, lays = m26["materials"], m26["layers"]
    by_id = {d["id"]: d for d in m26["definitions"]}
    n26 = collections.Counter()

    def rec(c, world, mat, depth):
        for i in c["instances"]:
            w = mat_mul(world, mat_of(i["transform"]))
            eff = i["material"] if i["material"] is not None else mat
            d = by_id[i["definition"]]
            n26[(d["name"], tuple(round(x, 6) + 0.0 for x in w),
                 mats[eff]["name"] if eff in mats else None,
                 lname(lays[i["layer"]]["name"] if i["layer"] in lays else None),
                 bool(i["flags"] & 1))] += 1
            if depth < 64:
                rec(d, w, eff, depth + 1)
    rec(m26["root"], IDENTITY, None, 0)
    n17 = collections.Counter((x["definition"], tuple(round(v, 6) + 0.0 for v in x["world"]), x["material"],
                               lname(x["layer"]), x["hidden"]) for x in oracle["nodes"])
    ok_inst = n17 == n26
    print(f"instances {sum(n17.values())}/{sum(n26.values())} (definition, world, inherited material, layer, hidden): "
          + ("equivalent" if ok_inst else "DIFFER"))
    if not ok_inst:
        for k in list(n17 - n26)[:3]:
            print("   2017:", str(k)[:300])
        for k in list(n26 - n17)[:3]:
            print("   2026:", str(k)[:300])
    t17 = sorted((m["name"], m["kind"], m.get("opacity"), tuple(m.get("applied_size_in") or ()), m.get("image_len"),
                  tuple(m.get("avg_rgba") or ())) for m in oracle["materials"])
    t26 = sorted((m["name"], m.get("kind"), round(m.get("opacity", 1.0), 6) if m.get("kind") else None,
                  tuple(round(x, 3) for x in m.get("applied_size_in") or ()), len(m["image"]) if m.get("image") else None,
                  tuple(m.get("avg_rgba") or ())) for m in m26["materials"].values())
    t17 = [(n, k, round(o, 6) if o is not None else None, tuple(round(x, 3) for x in a), il, av) for n, k, o, a, il, av in t17]
    ok_mat = t17 == t26
    print("material table: " + ("equivalent" if ok_mat else "DIFFERS"))
    if not ok_mat:
        for x, y in zip(t17, t26):
            if x != y:
                print("   2017:", x, "\n   2026:", y)
    l17 = sorted((l["name"], l["visible"], tuple(l["rgba"])) for l in oracle["layers"])
    l26 = sorted((l["name"], l["visible"], tuple(l["rgba"] or ())) for l in m26["layers"].values())
    ok_lay = l17 == l26
    print("layer table: " + ("equivalent" if ok_lay else f"DIFFERS\n   2017: {l17[:4]}\n   2026: {l26[:4]}"))
    only17, only26 = s17 - s26, s26 - s17
    nf = lambda s: sum(len(k[0]) * n for k, n in s.items())  # noqa: E731
    ne = lambda s: sum(len(k[1]) * n for k, n in s.items())  # noqa: E731
    print(f"containers 2017={sum(s17.values())} 2026={sum(s26.values())}  "
          f"faces {nf(s17)}/{nf(s26)}  edges {ne(s17)}/{ne(s26)}")
    if not only17 and not only26:
        print("equivalent: every container's faces, holes, windings and edge flags match")
        return 0 if scene17 == scene26 and ok_app and ok_mat and ok_lay and ok_inst else 1
    print(f"UNMATCHED: {sum(only17.values())} 2017 containers, {sum(only26.values())} 2026 containers")
    for a in list(only17)[:3]:
        # closest 2026 container by shared faces, to show what differs
        b = max(only26, key=lambda b: len(set(a[0]) & set(b[0])) + len(set(a[1]) & set(b[1])), default=None)
        print(f"  2017 container: {len(a[0])} faces, {len(a[1])} edges")
        if b:
            print(f"    faces only in 2017: {str(sorted(set(a[0]) - set(b[0]))[:2])[:600]}")
            print(f"    faces only in 2026: {str(sorted(set(b[0]) - set(a[0]))[:2])[:600]}")
            print(f"    edges only in 2017: {sorted(set(a[1]) - set(b[1]))[:3]}")
            print(f"    edges only in 2026: {sorted(set(b[1]) - set(a[1]))[:3]}")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
