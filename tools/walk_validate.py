#!/usr/bin/env python3
import re
"""Validate the CArchive walker: (1) class inventory works on every corpus file,
(2) the geometry walk resolves the inline-new / back-reference / null pointer
recursion correctly on the lone-edge files (no faces, so fully walkable today)."""
import sys, os, glob
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import skpwalk, carchive

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
fails = checks = 0

def check(name, cond, detail=""):
    global fails, checks
    checks += 1
    if not cond:
        fails += 1
    print(f"  {'OK ' if cond else 'FAIL'}  {name}{'' if cond else '  '+detail}")

# 1) inventory on every file
# note: CVersionMap is the container and does not list itself
CORE = {"CSketchUpModel", "CVertex", "CEdge", "CFace",
        "CComponentInstance", "CViewPage", "CLayer", "CMaterial"}
for skp in sorted(glob.glob(os.path.join(root, "corpus", "2017", "*.skp")) + glob.glob(os.path.join(root, "corpus", "legacy", "*.skp"))):
    inv = dict(skpwalk.inventory(skp))
    check(f"inventory {os.path.basename(skp):28} ({len(inv)} classes)",
          CORE <= set(inv), f"missing {CORE - set(inv)}")

# 2) geometry walk — single-line: 1 edge, two inline vertices
objs, _ = skpwalk.geom_walk(os.path.join(root, "corpus", "2017", "single-line.skp"))
e = objs[0] if objs else {}
verts = {v["m"] for v in (e.get("v0"), e.get("v1")) if isinstance(v, dict)}
check("single-line: 1 CEdge", len(objs) == 1 and e.get("__class__") == "CEdge")
check("single-line: endpoints (0,0,0)+(1,0,0) inline",
      verts == {(0.0, 0.0, 0.0), (1.0, 0.0, 0.0)}, str(verts))
check("single-line: null curve", e.get("curve") is None)

# 3) two-lines: 2 edges; edge2 = back-ref(shared) + new vertex (1,1,0)
objs, _ = skpwalk.geom_walk(os.path.join(root, "corpus", "2017", "two-lines.skp"))
check("two-lines: 2 CEdge", len(objs) == 2 and all(o["__class__"] == "CEdge" for o in objs))
inline = [v["m"] for o in objs for v in (o["v0"], o["v1"]) if isinstance(v, dict)]
refs = [v for o in objs for v in (o["v0"], o["v1"]) if isinstance(v, carchive.Ref)]
check("two-lines: 3 distinct inline vertices", set(inline) == {(0,0,0),(1,0,0),(1,1,0)}, str(set(inline)))
check("two-lines: exactly 1 shared-vertex back-ref", len(refs) == 1, str(refs))

# 4) triangle-face: full face topology (edges + loop + edge-uses), recursively
from collections import Counter
def deep(o, c, seen=None):
    seen = set() if seen is None else seen
    if not isinstance(o, dict) or id(o) in seen:
        return
    seen.add(id(o))
    c[o["__class__"]] += 1
    for L in o.get("loops", []): deep(L, c, seen)
    for e in o.get("edge_uses", []): deep(e, c, seen)
    for k in ("v0", "v1", "curve"):
        if isinstance(o.get(k), dict): deep(o[k], c, seen)
objs, _ = skpwalk.geom_walk(os.path.join(root, "corpus", "2017", "triangle-face.skp"))
c = Counter()
for o in objs: deep(o, c)
check("triangle-face: full topology 3E/3V/1F/1L/3EU",
      c == {"CEdge": 3, "CVertex": 3, "CFace": 1, "CLoop": 1, "CEdgeUse": 3}, dict(c))
face = next((o for o in objs if o["__class__"] == "CFace"), None)
check("triangle-face: face plane is a unit normal",
      face is not None and abs(sum(x*x for x in face["plane"][:3]) ** 0.5 - 1.0) < 1e-6)

# 5) triangle-face: resolve the back-reference graph to concrete topology
r = skpwalk.resolve_geometry(os.path.join(root, "corpus", "2017", "triangle-face.skp"))
check("triangle-face: unique global map base solved", r["base"] is not None, str(r["base"]))
# every edge-use resolves to a CEdge and they all share ONE loop
eus = [e for o in r["objects"] if o["__class__"] == "CFace"
       for L in o["loops"] for e in L["edge_uses"]]
def tgt(ref):
    return r["resolved"].get(ref.index) if isinstance(ref, carchive.Ref) else ref
edge_targets = [tgt(e["edge"]) for e in eus]
loop_targets = [tgt(e["parent"]) for e in eus]
check("triangle-face: 3 edge-uses each resolve to a CEdge",
      len(eus) == 3 and all(t and t["__class__"] == "CEdge" for t in edge_targets))
check("triangle-face: all edge-uses share one CLoop",
      len({id(t) for t in loop_targets}) == 1 and loop_targets[0]["__class__"] == "CLoop")

# 6) solids & complex faces (P1 corpus) — full face topology via resync
def gcounts(name):
    r = skpwalk.resolve_geometry(os.path.join(root, "corpus", "2017", name))
    c = Counter()
    for o in r["objects"]:
        deep(o, c)
    return c, r

c, r = gcounts("box.skp")                       # 1 m cube
check("box: cube = 6 faces / 6 loops / 24 edge-uses",
      c["CFace"] == 6 and c["CLoop"] == 6 and c["CEdgeUse"] == 24, dict(c))
check("box: back-reference graph resolved (unique base)", r["base"] is not None)

c, _ = gcounts("ngon-face.skp")                 # hexagon
check("ngon-face: 1 face / 1 loop / 6 edge-uses",
      c["CFace"] == 1 and c["CLoop"] == 1 and c["CEdgeUse"] == 6, dict(c))

c, _ = gcounts("face-with-hole.skp")            # outer loop + inner hole loop
check("face-with-hole: 1 face / 2 loops / 8 edge-uses",
      c["CFace"] == 1 and c["CLoop"] == 2 and c["CEdgeUse"] == 8, dict(c))

# 7) component / group definitions — geometry inside the CComponentDefinition
#    entity-list container (lazy class defs) walks to the full cube.
for name in ("group.skp", "box-component.skp", "two-components.skp", "nested-component.skp"):
    c, r = gcounts(name)
    check(f"{name[:-4]}: definition geometry walks (cube = 6F/6L/24EU)",
          c["CFace"] == 6 and c["CLoop"] == 6 and c["CEdgeUse"] == 24, dict(c))
    # definition-local back-ref base resolved, ≥90% of constraints satisfied
    check(f"{name[:-4]}: back-refs resolved at definition-local base (>=90%)",
          r["base"] is not None and r["satisfied"] >= 0.9 * r["constraints"],
          f"base={r['base']} {r['satisfied']}/{r['constraints']}")

# top-level geometry resolves 100% at the global base
for name in ("box.skp", "ngon-face.skp", "face-with-hole.skp"):
    r = skpwalk.resolve_geometry(os.path.join(root, "corpus", "2017", name))
    check(f"{name[:-4]}: 100% back-refs resolved (global base)",
          r["satisfied"] == r["constraints"], f"{r['satisfied']}/{r['constraints']}")

# 8) component/group definition enumeration (name + GUID), vs COLLADA names
import xml.etree.ElementTree as ET
NS = {"c": "http://www.collada.org/2005/11/COLLADASchema"}
def dae_def_names(path):
    root = ET.parse(path).getroot()
    names = set()
    for ln in root.findall(".//c:library_nodes/c:node", NS):
        if ln.get("name"):
            names.add(ln.get("name"))
    return names
def check_defs(name, expected):
    defs = skpwalk.component_definitions(os.path.join(root, "corpus", "2017", name))
    got = {d["name"].replace(" ", "_").replace("#", "_") for d in defs}
    all_guid = all(len(d["guid"]) == 32 for d in defs)
    check(f"{name[:-4]}: definitions {sorted(d['name'] for d in defs)}",
          got == expected and all_guid, f"got {got} want {expected}")

check_defs("two-components.skp", {"Box_1", "Box_2"})
check_defs("nested-component.skp", {"Box_1", "Box_2"})
check_defs("box-component.skp", {"Box_Component"})
check("box.skp: no component definitions",
      skpwalk.component_definitions(os.path.join(root, "corpus", "2017", "box.skp")) == [])
# names cross-check vs the .dae library_nodes (underscored)
for nm in ("two-components.skp", "nested-component.skp"):
    sk = {d["name"].replace(" ", "_") for d in skpwalk.component_definitions(os.path.join(root, "corpus", "2017", nm))}
    da = dae_def_names(os.path.join(root, "corpus", "2017", nm[:-4] + ".dae"))
    check(f"{nm[:-4]}: def names match COLLADA", sk <= da, f"{sk} vs {da}")

# 9) component instances (transform + def-ref), counts vs COLLADA instance nodes
def dae_instance_count(path):
    root = ET.parse(path).getroot()
    return len({n.get("name") for n in root.findall(".//c:node", NS)
               if n.get("name", "").startswith("instance_")})
def inst(name):
    return skpwalk.component_instances(os.path.join(root, "corpus", "2017", name))

check("box.skp: 0 instances (plain geometry)", len(inst("box.skp")) == 0)
i = inst("box-component-two-instances.skp")
check("box-component-two-instances: 2 instances of the SAME definition",
      len(i) == 2 and len({x["defref"] for x in i}) == 1, str(i))
check("box-component-two-instances: translations 0 and ~1.243 m",
      sorted(round(x["translation_m"][0], 2) for x in i) == [0.0, 1.24], str([x["translation_m"] for x in i]))
i = inst("two-components.skp")
check("two-components: 2 instances of DISTINCT definitions",
      len(i) == 2 and len({x["defref"] for x in i}) == 2, str(i))
i = inst("component-move.skp")
check("component-move: 2 instances of one def, translations 0 and 2.0 m",
      len(i) == 2 and len({x["defref"] for x in i}) == 1
      and sorted(round(x["translation_m"][0], 2) for x in i) == [0.0, 2.0], str(i))
# instance count matches COLLADA
for nm in ("two-components.skp", "nested-component.skp", "box-component-two-instances.skp"):
    check(f"{nm[:-4]}: instance count matches COLLADA",
          len(inst(nm)) == dae_instance_count(os.path.join(root, "corpus", "2017", nm[:-4] + ".dae")),
          f"skp={len(inst(nm))} dae={dae_instance_count(os.path.join(root,'corpus','2017',nm[:-4]+'.dae'))}")

# 10) full hierarchy: instance -> definition NAME -> transform, vs COLLADA
def dae_hierarchy(path):
    r = ET.parse(path).getroot()
    u = r.find(".//c:asset/c:unit", NS)
    sc = float(u.get("meter")) if u is not None and u.get("meter") else 1.0
    libname = {ln.get("id"): ln.get("name") for ln in r.findall(".//c:library_nodes/c:node", NS)}
    out = set()
    for node in r.findall(".//c:visual_scene//c:node", NS):
        if not node.get("name", "").startswith("instance_"):
            continue
        inn = node.find("c:instance_node", NS)
        mat = node.find("c:matrix", NS)
        if inn is None or mat is None:
            continue
        tx = [float(x) for x in mat.text.split()]
        name = libname.get(inn.get("url", "").lstrip("#"), "").replace("_", " ")
        out.add((name, round(tx[3]*sc, 3)))   # (def name, X translation in m)
    return out
def skp_hierarchy(name):
    t = skpwalk.component_tree(os.path.join(root, "corpus", "2017", name))
    return {(i["definition"], round(i["translation_m"][0], 3)) for i in t["instances"]}
for nm in ("two-components.skp", "box-component-two-instances.skp"):
    sk, da = skp_hierarchy(nm), dae_hierarchy(os.path.join(root, "corpus", "2017", nm[:-4] + ".dae"))
    check(f"{nm[:-4]}: instance->definition->transform matches COLLADA", sk == da,
          f"skp={sorted(sk)} dae={sorted(da)}")

# 11) per-definition geometry: walk EVERY definition's cube (not just densest)
def def_geoms(name):
    out = []
    for _, objs, _ in skpwalk.walk_definitions(os.path.join(root, "corpus", "2017", name)):
        c = Counter()
        for o in objs:
            deep(o, c)
        out.append(c)
    return out
for name, ndefs in (("two-components.skp", 2), ("nested-component.skp", 2),
                    ("box.skp", 1), ("group.skp", 1)):
    gs = def_geoms(name)
    cubes = [c for c in gs if c["CFace"] == 6 and c["CLoop"] == 6]
    check(f"{name[:-4]}: walks all {ndefs} definition geometr{'ies' if ndefs>1 else 'y'} (each a cube)",
          len(cubes) == ndefs, f"{len(cubes)} cubes of {[dict(c) for c in gs]}")

# 12) embedded image extraction (CDib carving)
import extract_images as EI
for name in ("box.skp", "empty.skp", "material-one-face.skp"):
    imgs = EI.extract_images(open(os.path.join(root, "corpus", "2017", name), "rb").read())
    check(f"{name[:-4]}: embedded images all complete ({len(imgs)})",
          len(imgs) >= 1 and all(i["complete"] for i in imgs), str([(i["kind"], i["len"]) for i in imgs]))
mof = EI.extract_images(open(os.path.join(root, "corpus", "2017", "material-one-face.skp"), "rb").read())
check("material-one-face: contains the embedded JPEG texture",
      any(i["kind"] == "jpg" for i in mof))

# 13) per-definition back-ref resolution — every definition resolves 100%
for name in ("box.skp", "triangle-face.skp", "two-components.skp", "nested-component.skp", "group.skp"):
    rd = skpwalk.resolve_definitions(os.path.join(root, "corpus", "2017", name))
    ok = all(dd["base"] is not None and dd["satisfied"] == dd["constraints"] and dd["constraints"] > 0
             for dd in rd)
    check(f"{name[:-4]}: all {len(rd)} definition(s) resolve 100% of back-refs",
          ok, str([(dd["base"], f"{dd['satisfied']}/{dd['constraints']}") for dd in rd]))

# 14) unified parse_model() — runs on every corpus file, geometry resolves 100%
import glob
for skp in sorted(glob.glob(os.path.join(root, "corpus", "2017", "*.skp")) + glob.glob(os.path.join(root, "corpus", "legacy", "*.skp"))):
    try:
        m = skpwalk.parse_model(skp)
        base = os.path.basename(skp)
        if re.match(r"^box-v20\d\d\.skp$", base):
            # Tier D pre-2017 saves: this FROZEN reference validates them at
            # header level only (version string parses in its own {N.N.N}
            # shape; parse itself doesn't raise). Full pre-2017 geometry
            # walking is the Rust SDK's job (plan Phase 5.3), not the frozen
            # Python's — its walk constants are 2017-template-derived.
            ok = bool(re.match(r"^\{\d+[\d.]*\}$", m["version"]))
            check(f"parse_model {base:30} pre-2017: header-level only "
                  f"(version {m['version']})", ok)
            continue
        # smoke test: parse_model runs clean on every file, reports version + images,
        # and every retained (confident) geometry run resolves >=75% of its back-refs.
        # (Exact per-file resolution is checked in section 13.)
        ok = (m["version"] == "{17.3.116}" and len(m["images"]) >= 1
              and all(r["resolved"][0] >= 0.75 * r["resolved"][1]
                      for r in m["geometry_runs"] if r["resolved"][1] > 0))
        check(f"parse_model {base:30} {len(m['geometry_runs'])} run(s), "
              f"{len(m['definitions'])} defs, {len(m['instances'])} insts", ok)
    except Exception as e:
        check(f"parse_model {os.path.basename(skp)}", False, f"{type(e).__name__}: {e}")

# 15) material extraction — solid RGBA + textured (library) materials
import skpparse
def mats(name):
    return skpparse.materials(open(os.path.join(root, "corpus", "2017", name), "rb").read())
mm = mats("material-one-face.skp")
tex = [m for m in mm if m["kind"] == "textured"]
check("material-one-face: textured material '[Wood Floor Light]' + .jpg detected",
      any(m["name"] == "[Wood Floor Light]" and (m.get("texture") or "").lower().endswith(".jpg")
          for m in tex), str(mm))
b2m = open(os.path.join(root, "corpus", "2017", "box-two-materials.skp"), "rb").read()
check("box-two-materials: solid materials *1 and *2 with correct RGBA",
      {(251, 1, 6, 255), (63, 127, 2, 255)} <= set(skpparse.solid_colors(b2m)))
check("box: solid 'Default' white material present",
      any(m["name"] == "Default" and m["rgba"] == (255, 255, 255, 255) for m in mats("box.skp")))

# 16) layer enumeration — every file has the default 'Layer0', deduped
for name in ("box.skp", "two-components.skp", "material-one-face.skp"):
    lys = skpparse.layers(open(os.path.join(root, "corpus", "2017", name), "rb").read())
    check(f"{name[:-4]}: layer 'Layer0' enumerated (deduped, {len(lys)})",
          len(lys) == len({l['name'] for l in lys}) and any(l["name"] == "Layer0" for l in lys),
          str(lys))

# 17) multi-layer enumeration (layers.skp) — distinct named layers with colours
lys = skpparse.layers(open(os.path.join(root, "corpus", "2017", "layers.skp"), "rb").read())
check("layers.skp: 4 named layers with distinct colours",
      {l["name"] for l in lys} == {"Layer0", "BigLayer", "MediumLayer", "LittleLayer"}
      and len({l["rgba"] for l in lys}) == 4, str(lys))
vis = {l["name"]: l["visible"] for l in lys}
check("layers.skp: MediumLayer hidden, the rest visible",
      vis.get("MediumLayer") is False and all(vis[k] for k in ("Layer0", "BigLayer", "LittleLayer")),
      str(vis))

# 18) CFace front/back material (back-material.skp: front != back, both set)
def faces_of(name):
    r = skpwalk.resolve_geometry(os.path.join(root, "corpus", "2017", name))
    return [o for o in r["objects"] if isinstance(o, dict) and o["__class__"] == "CFace"]
bm = faces_of("back-material.skp")
check("back-material: face has distinct front & back materials (both set)",
      any(f["front_material"] and f["back_material"] and f["front_material"] != f["back_material"]
          for f in bm), str([(f["front_material"], f["back_material"]) for f in bm]))
pf = faces_of("paint-one-face.skp")
check("paint-one-face: painted face is front-only (back_material=0)",
      any(f["front_material"] and f["back_material"] == 0 for f in pf)
      and all(f["back_material"] == 0 for f in pf), str([(f["front_material"], f["back_material"]) for f in pf]))
check("box: all faces have no front/back material",
      all(f["front_material"] == 0 and f["back_material"] == 0 for f in faces_of("box.skp")))

# 19) scene/page enumeration (two-scenes.skp) vs COLLADA camera names
sc = skpparse.scenes(open(os.path.join(root, "corpus", "2017", "two-scenes.skp"), "rb").read())
dae_scene = set()
for m in re.finditer(r'name="skp_camera_(Scene_\d+)"', open(os.path.join(root, "corpus", "2017", "two-scenes.dae")).read()):
    dae_scene.add(m.group(1).replace("_", " "))
check("two-scenes: scenes [Scene 1, Scene 2] match COLLADA cameras",
      sc == ["Scene 1", "Scene 2"] and set(sc) == dae_scene, f"skp={sc} dae={dae_scene}")
check("box: no scenes", skpparse.scenes(open(os.path.join(root, "corpus", "2017", "box.skp"), "rb").read()) == [])

# 20) material opacity (attributes.skp has 50% transparent materials) vs .dae alpha
am = skpparse.materials(open(os.path.join(root, "corpus", "2017", "attributes.skp"), "rb").read())
solids = [m for m in am if m["kind"] == "solid"]
check("attributes: has a 0.5-opacity (transparent) material",
      any(abs(m["opacity"] - 0.5) < 1e-6 for m in solids), str([(m["name"], m["opacity"]) for m in solids]))
check("box: all solid materials fully opaque (opacity 1.0)",
      all(m["opacity"] == 1.0 for m in skpparse.materials(open(os.path.join(root, "corpus", "2017", "box.skp"), "rb").read())
          if m["kind"] == "solid"))
# opacity * 255 matches a .dae alpha (0.5 -> ~128)
_ad = ET.parse(os.path.join(root, "corpus", "2017", "attributes.dae")).getroot()
dae_alphas = {round(float(c.text.split()[3]) * 255) for c in _ad.iter() if c.tag.endswith("color")}
check("attributes: 0.5 opacity -> ~128 alpha present in COLLADA",
      128 in dae_alphas or 127 in dae_alphas, str(sorted(dae_alphas)))

# 21) PNG-texture material (png-texture.skp) — textured material + PNG file
pm = skpparse.materials(open(os.path.join(root, "corpus", "2017", "png-texture.skp"), "rb").read())
check("png-texture: textured material with a .png texture file",
      any(m["kind"] == "textured" and (m.get("texture") or "").lower().endswith(".png") for m in pm),
      str([(m["name"], m.get("texture")) for m in pm]))

# 22) CArcCurve (arc/circle) — the edge->curve pointer is decoded (122-byte record)
for name, min_res in (("arc.skp", 0.9), ("circle.skp", 1.0)):
    r = skpwalk.resolve_geometry(os.path.join(root, "corpus", "2017", name))
    cc = Counter()
    for o in r["objects"]:
        deep(o, cc)
    check(f"{name[:-4]}: CArcCurve decoded + back-refs resolved (>={int(min_res*100)}%)",
          cc["CArcCurve"] >= 1 and r["satisfied"] >= min_res * r["constraints"] and r["constraints"] > 0,
          f"CArcCurve={cc['CArcCurve']} {r['satisfied']}/{r['constraints']}")
ccnt = Counter()
for o in skpwalk.resolve_geometry(os.path.join(root, "corpus", "2017", "circle.skp"))["objects"]:
    deep(o, ccnt)
check("circle: closed curve => 1 face + 24 edge-uses + 1 CArcCurve",
      ccnt["CFace"] == 1 and ccnt["CEdgeUse"] == 24 and ccnt["CArcCurve"] == 1, dict(ccnt))

# 23) polyline (stored as edges) + textured applied size
r = skpwalk.resolve_geometry(os.path.join(root, "corpus", "2017", "polyline.skp"))
pc = Counter()
for o in r["objects"]:
    deep(o, pc)
check("polyline: stored as 4 edges / 5 vertices (CPolyline3d unused)",
      pc["CEdge"] == 4 and pc["CVertex"] == 5, dict(pc))
tm = [m for m in skpparse.materials(open(os.path.join(root, "corpus", "2017", "material-one-face.skp"), "rb").read())
      if m["kind"] == "textured"]
check("material-one-face: [Wood Floor Light] applied size 120x48 in",
      any(m["applied_size_in"] == (120.0, 48.0) for m in tm), str([m["applied_size_in"] for m in tm]))

# 24) attribute dictionaries — model options + geo-location + dynamic-component params
def attrs(name):
    return {k: v for k, t, v in skpparse.attributes(open(os.path.join(root, "corpus", "2017", name), "rb").read())}
ea = attrs("box.skp")
check("box: model-option attributes decoded (SnapAngle 15, LengthPrecision 4)",
      ea.get("SnapAngle") == 15.0 and ea.get("LengthPrecision") == 4, str({k: ea.get(k) for k in ("SnapAngle", "LengthPrecision")}))
check("box: geo-location attributes (Boulder CO ~40.02, -105.24)",
      abs(ea.get("Latitude", 0) - 40.018) < 0.01 and abs(ea.get("Longitude", 0) + 105.242) < 0.01,
      str({k: ea.get(k) for k in ("Latitude", "Longitude")}))
aa = attrs("attributes.skp")
check("attributes.skp: dynamic-component params present (_hasbehaviors, _lenx_nominal)",
      aa.get("_hasbehaviors") == 1.0 and "_lenx_nominal" in aa,
      str({k: aa.get(k) for k in ("_hasbehaviors", "_lenx_nominal")}))

# 25) CEdge soft/smooth flags (drawbase[5]=soft, [6]=smooth)
def edges_of(name):
    r = skpwalk.resolve_geometry(os.path.join(root, "corpus", "2017", name))
    return [o for o in r["objects"] if isinstance(o, dict) and o["__class__"] == "CEdge"]
ss = edges_of("soft-smooth-edges.skp")
check("soft-smooth-edges: all edges soft + smooth",
      len(ss) > 0 and all(e["soft"] and e["smooth"] for e in ss),
      str([(e["soft"], e["smooth"]) for e in ss]))
check("box: all edges hard (not soft/smooth)",
      all(not e["soft"] and not e["smooth"] for e in edges_of("box.skp")))

# 26) construction line / guide (guide.skp: 1 m above red axis, along red axis)
gl = skpparse.guides(open(os.path.join(root, "corpus", "2017", "guide.skp"), "rb").read())
check("guide.skp: construction line at y=1 m, direction along red axis (1,0,0)",
      len(gl) == 1 and abs(gl[0]["point_m"][1] - 1.0) < 1e-4 and gl[0]["direction"] == (1.0, 0.0, 0.0),
      str(gl))
check("box: no construction lines", skpparse.guides(open(os.path.join(root, "corpus", "2017", "box.skp"), "rb").read()) == [])

# 27) per-face UV mapping (uv-quad.skp): default UV = face_position / texture_size
uq = open(os.path.join(root, "corpus", "2017", "uv-quad.skp"), "rb").read()
tex = [m for m in skpparse.materials(uq) if m["kind"] == "textured"]
uvdae = re.search(r'<float_array id="ID20"[^>]*>([^<]+)</float_array>',
                  open(os.path.join(root, "corpus", "2017", "uv-quad.dae")).read())
if tex and uvdae and tex[0].get("applied_size_in"):
    tw, th = tex[0]["applied_size_in"]
    vals = [float(x) for x in uvdae.group(1).split()]
    u_ext = max(abs(v) for v in vals[0::2]); v_ext = max(vals[1::2])
    face_in = 39.37008  # 1 m quad
    check("uv-quad: UV extent = face_size / texture_size, matches COLLADA",
          abs(u_ext - face_in/tw) < 1e-4 and abs(v_ext - face_in/th) < 1e-4,
          f"UV=({u_ext:.5f},{v_ext:.5f}) vs face/tex=({face_in/tw:.5f},{face_in/th:.5f})")
else:
    check("uv-quad: textured material + UV data present", False)

print(f"\nWalker: {checks-fails}/{checks} checks passed")
sys.exit(1 if fails else 0)
