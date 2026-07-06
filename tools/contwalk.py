#!/usr/bin/env python3
"""contwalk — continuous single-archive walk of the model section (SKP_FORMAT §4s).

The whole .skp payload is ONE MFC archive from 0x50.
CDib class = slot 3, CMaterial = 16, CLayer = 31 (house.skp). The model
section = layer list + count-prefixed definition list (each definition body
embeds its own entity list) + root entity list. Back-refs are GLOBAL.

This prototype starts at the layer list, pads the map to the calibrated
base (derived from the first repeated class-ref tag), and walks forward,
decoding bodies exactly. Unknown structure => Stall + hexdump so the layout
can be decoded by hand and added.
"""
import struct, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from carchive import CArchive, Stall, Ref

TRACE = os.environ.get("TRACE")

def hexdump(d, start, n=96):
    for row in range(start, start + n, 16):
        bs = d[row:row+16]
        hx = " ".join(f"{b:02x}" for b in bs)
        pr = "".join(chr(b) if 32 <= b < 127 else "." for b in bs)
        print(f"  {row:08x}: {hx:<48} {pr}")

# ---------- entity preamble, REINTERPRETED ----------
# §4i's "00 00 <mask>" lead bytes are actually a NULLABLE OBJECT POINTER
# (the entity's attribute container — null for plain entities) followed by
# the pid presence-mask field. Textured faces carry `05 80` (inline
# CAttributeContainer holding CFaceTextureCoords) exactly there.
def preamble(ar, o=None):
    attrs = ar.read_object_expect("CAttributeContainer")
    if o is not None and attrs is not None:
        o["attrs"] = attrs
    mask = ar.u1()
    assert mask <= 0x0f, f"pid mask {mask:#x} @{ar.pos-1:#x}"
    raw = ar.take(bin(mask).count("1"))
    pid, i = 0, 0
    for k in range(4):
        if mask & (1 << k):
            pid |= raw[i] << (8 * k); i += 1
    return pid

# ---------- body readers ----------
def r_clayer(ar, o):
    o["pid"] = preamble(ar, o)
    o["name"] = ar.utf16()
    o["hidden"] = ar.u4()
    o["internal"] = ar.utf16()
    o["w16"] = ar.u2()
    o["rgba"] = tuple(ar.take(4))
    o["s2"] = ar.utf16()
    o["tail"] = ar.take(21).hex()

def r_cvertex(ar, o):
    o["pid"] = preamble(ar, o)
    o["xyz"] = (ar.f8(), ar.f8(), ar.f8())

def r_cedge(ar, o):
    o["pid"] = preamble(ar, o)
    db = ar.take(10)
    o["hidden"] = db[2] != 0
    o["soft"], o["smooth"] = db[5] != 0, db[6] != 0
    o["layer"] = struct.unpack("<H", db[8:10])[0]
    o["v0"] = ar.read_object()
    o["v1"] = ar.read_object()
    o["curve"] = ar.read_object()

def r_cedgeuse(ar, o):
    ar.take(3)
    o["edge"] = ar.read_object()
    o["dir"] = ar.u1()
    o["parent"] = ar.read_object()

def r_cloop(ar, o):
    o["flags"] = ar.take(5).hex()
    o["uses"] = []
    while True:
        eu = ar.read_object()
        if eu is None:
            break
        o["uses"].append(eu)

def r_cface(ar, o):
    o["pid"] = preamble(ar, o)
    db = ar.take(10)
    o["frontmat"] = struct.unpack("<H", db[0:2])[0]
    o["hidden"] = db[2] != 0
    o["layer"] = struct.unpack("<H", db[8:10])[0]
    o["plane"] = tuple(ar.f8() for _ in range(4))
    n = ar.u4()
    o["loops"] = [ar.read_object() for _ in range(n)]
    o["backmat"] = ar.u2()

def r_cconstructionline(ar, o):
    """CConstructionLine schema 1 (guide.skp root list, @0x11b63):
    preamble + drawbase(10) + point(3 f64 in) + unit direction(3 f64) +
    line-parameter bounds(2 f64, ±1e30 = infinite guide) + 7B tail
    (zeros observed, TBD). Extent pinned by the §4l root tail resuming
    byte-exactly after the single root element."""
    o["pid"] = preamble(ar, o)
    db = ar.take(10)
    o["hidden"] = db[2] != 0
    o["layer"] = struct.unpack("<H", db[8:10])[0]
    o["point"] = (ar.f8(), ar.f8(), ar.f8())
    o["direction"] = (ar.f8(), ar.f8(), ar.f8())
    o["bounds"] = (ar.f8(), ar.f8())
    o["tail"] = ar.take(7).hex()

def r_carccurve(ar, o):
    o["pid"] = preamble(ar, o)
    ar.take(5)
    o["params"] = tuple(ar.f8() for _ in range(14))

def r_ccurve(ar, o):
    """CCurve schema 4 (theater-2017 @0x56df8d): preamble + u8 + u32
    member-edge count. NO owned children — member edges serialize as
    ordinary list/loop elements and each back-refs the curve via its
    curve pointer (same association direction as CArcCurve)."""
    o["pid"] = preamble(ar, o)
    o["flag"] = ar.u1()
    o["count"] = ar.u4()

def r_crelationship(ar, o):
    """CRelationship schema 0 (theater-2017 def tails): preamble + u32."""
    o["pid"] = preamble(ar, o)
    o["val"] = ar.u4()

def r_cattrcontainer(ar, o):
    o["pid"] = preamble(ar, o)
    o["children"] = []
    while True:
        c = ar.read_object_expect("CAttributeNamed")
        if c is None:
            break
        o["children"].append(c)

def attr_value(ar, t):
    if t == 0x00:
        return None            # nil (theater-2017 dynamic components)
    if t == 0x0a:
        return ar.utf16()
    if t == 0x07:
        return ar.u1()
    if t == 0x04:
        return struct.unpack("<i", ar.take(4))[0]
    if t == 0x06:
        return ar.f8()
    if t == 0x0b:              # typed array: u32 count + elem-type + values
        n = ar.u4(); et = ar.u1()
        return [attr_value(ar, et) for _ in range(n)]
    raise Stall(f"<attr type {t:#x}>", ar.pos, len(ar.map))

def r_cattrnamed(ar, o):
    o["pid"] = preamble(ar, o)
    o["u"] = ar.take(4).hex()
    o["name"] = ar.utf16()
    o["entries"] = []
    while True:
        key = ar.utf16()
        if key == "":
            break
        t = ar.u1()
        o["entries"].append((key, t, attr_value(ar, t)))
    o["tail32"] = ar.u4()

def r_cftc(ar, o):
    """SOLVED (SKP_FORMAT §4u): affine block + front/back pin lists."""
    o["pid"] = preamble(ar, o)
    o["u4a"] = ar.u4()
    o["k"] = tuple(ar.f8() for _ in range(24))
    for side in ("front_pins", "back_pins"):
        n = ar.u4()
        assert n <= 4, f"ftc pin count {n} @{ar.pos:#x}"
        o[side] = [tuple(ar.f8() for _ in range(4)) for _ in range(n)]
    o["flags"] = (ar.u4(), ar.u4())

def r_ccompdef(ar, o):
    o["pre"] = preamble(ar, o)
    o["drawbase"] = ar.take(10).hex()
    o["mid"] = ar.take(12).hex()
    nlay = ar.u4()
    o["layers"] = [ar.read_object() for _ in range(nlay)]
    # §4h prelude REVISED (theater-2017): <decl:u16, 0x7FFF-escalated to
    # u32> <u32 0> <count:u32> — for decl < 0x7fff byte-identical to the
    # old "<decl:u32> 00 00 <count>" reading.
    decl = ar.u2()
    if decl == 0x7FFF:
        decl = ar.u4()
    z = ar.take(4); cnt = ar.u4()
    assert z == b"\x00\x00\x00\x00", f"def prelude @{ar.pos:#x}: {z.hex()}"
    o["decl"], o["count"] = decl, cnt
    o["entities"] = []
    for i in range(cnt):
        o["entities"].append(ar.read_object())
    r_deftail(ar, o)

def r_deftail(ar, o):
    """Definition body tail after the entity list: GUID + name + desc…
    Iteratively decoded; stalls loudly at the first unknown byte run."""
    o["tail_at"] = ar.pos
    # def tail = u32 relationship-count + N CRelationship objects (theater
    # def "Group#119": count 1 + inline ffff CRelationship decl) + u16 +
    # GUID + name + desc + ...
    nrel = ar.u4()
    o["relationships"] = [ar.read_object() for _ in range(nrel)]
    o["tail_u16"] = ar.u2()
    o["guid"] = ar.take(16).hex()
    o["name"] = ar.utf16()
    o["desc"] = ar.utf16()
    o["s3"] = ar.utf16()
    o["timestamp"] = ar.u4()
    o["midtail_at"] = ar.pos
    # not yet pinned: block between timestamp and the thumbnail object
    # (varies 42/47 bytes between plain groups and warehouse comps).
    # Locate the thumbnail tag structurally for now and record the gap.
    p = ar.pos
    for q in range(p, p + 96):
        w = struct.unpack_from("<H", ar.d, q)[0]
        if w == 0xFFFF and ar.d[q+2:q+10] == b"\x01\x00\x0a\x00CThu":
            break
        if w & 0x8000 and w != 0xFFFF:
            idx = w & 0x7FFF
            e = ar.map[idx] if idx < len(ar.map) else None
            if isinstance(e, tuple) and e[0] == "class" and e[1] == "CThumbnail":
                break
    else:
        raise Stall("<def midtail unbounded>", p, len(ar.map))
    o["midtail"] = ar.take(q - p).hex()
    o["thumbnail"] = ar.read_object()

def r_cinstance(ar, o):
    """CComponentInstance (§4o) and CGroup share this layout exactly."""
    o["pid"] = preamble(ar)
    db = ar.take(10)
    o["hidden"] = db[2] != 0
    o["layer"] = struct.unpack("<H", db[8:10])[0]
    # §4o REVISED: the def-ref is an MFC OBJECT POINTER, not a raw u16 —
    # identical bytes for slots < 0x8000 (backref word), but big maps
    # (theater-2017) escalate via the 7F FF big-tag escape (u32 index).
    dref = ar.read_object()
    o["defref"] = dref.index if isinstance(dref, Ref) else dref
    o["transform"] = tuple(ar.f8() for _ in range(13))
    o["name"] = ar.utf16()
    o["guid"] = ar.take(16).hex()

def r_ccamera(ar, o):
    o["head"] = ar.take(137)
    o["w"] = ar.u2()
    o["desc"] = ar.utf16()
    o["tail"] = ar.take(33)

def r_cdib(ar, o):
    o["subtype"] = ar.u4()
    n = ar.u4()
    o["at"], o["len"] = ar.pos, n
    ar.take(n)

def r_cthumbnail(ar, o):
    ar.take(3)
    o["camera"] = ar.read_object_expect("CCamera")
    o["image"] = ar.read_object_expect("CDib")   # null or CDib

READERS = {
    "CLayer": r_clayer,
    "CVertex": r_cvertex,
    "CEdge": r_cedge,
    "CEdgeUse": r_cedgeuse,
    "CLoop": r_cloop,
    "CFace": r_cface,
    "CArcCurve": r_carccurve,
    "CConstructionLine": r_cconstructionline,
    "CCurve": r_ccurve,
    "CRelationship": r_crelationship,
    "CComponentDefinition": r_ccompdef,
    "CCamera": r_ccamera,
    "CDib": r_cdib,
    "CThumbnail": r_cthumbnail,
    "CGroup": r_cinstance,
    "CAttributeContainer": r_cattrcontainer,
    "CAttributeNamed": r_cattrnamed,
    "CFaceTextureCoords": r_cftc,
    "CComponentInstance": r_cinstance,
}
def make_archive(d, pos, pad):
    ar = CArchive(d, pos)
    ar._stack = []
    orig_ro = ar._read_object
    ar._ring = []
    ar._expect = None
    ar.bound = []
    # Context-directed pad binding (§4s, ported back from walk2.rs): a
    # class-ref into PAD territory resolves by the read site's EXPECTED
    # class, binding the slot permanently. This replaces the old fixed
    # house-tuned SEED table — expectations hold for any file.
    def read_object_expect(expected):
        ar._expect = expected
        try:
            return ar.read_object()
        finally:
            ar._expect = None
    ar.read_object_expect = read_object_expect
    # §4t: string records ESCALATE at 255 chars (len byte 0xFF -> u16;
    # 0xFFFF -> u32) — the frozen carchive.utf16 reads a u8 length only,
    # which desyncs on >255-char strings (warehouse figure descriptions).
    def utf16():
        d, p = ar.d, ar.pos
        assert d[p:p+3] == b"\xff\xfe\xff", f"str rec @{p:#x}"
        n = d[p+3]; at = p + 4
        if n == 0xFF:
            n = struct.unpack_from("<H", d, at)[0]; at += 2
            if n == 0xFFFF:
                n = struct.unpack_from("<I", d, at)[0]; at += 4
        s = d[at:at+n*2].decode("utf-16le")
        ar.pos = at + n*2
        return s
    ar.utf16 = utf16
    def stacked():
        p = ar.pos
        w = struct.unpack_from("<H", ar.d, p)[0]
        # the expectation is consumed by THIS tag only — nested reads
        # inside a body must never inherit it
        exp, ar._expect = ar._expect, None
        if exp and (w & 0x8000) and w != 0xFFFF:
            idx = w & 0x7FFF
            e = ar.map[idx] if idx < len(ar.map) else None
            if isinstance(e, tuple) and e[0] == "pad":
                ar.map[idx] = ("class", exp)
                ar.bound.append((idx, exp))
        ar._stack.append((p, w))
        ar._ring.append((p, w))
        if len(ar._ring) > 40: ar._ring.pop(0)
        try:
            return orig_ro()
        except Stall:
            if not hasattr(ar, "_stall_stack"):
                ar._stall_stack = list(ar._stack)
            raise
        finally:
            ar._stack.pop()
    ar._read_object = stacked
    ar.map += [("pad", i) for i in range(pad)]
    for name, fn in READERS.items():
        ar.register(name, fn)
    if TRACE:
        orig = ar._read_object
        def traced():
            p = ar.pos
            w = struct.unpack_from("<H", ar.d, p)[0]
            r = orig()
            nm = r.get("__class__") if isinstance(r, dict) else r
            print(f"    ro @0x{p:x} tag=0x{w:04x} -> {nm if isinstance(nm,str) else type(r).__name__} (map {len(ar.map)-1})")
            return r
        ar._read_object = traced
    return ar

def main(path):
    d = open(path, "rb").read()
    decl = d.find(b"\xff\xff\x02\x00\x06\x00CLayer")
    assert decl > 0
    count = struct.unpack_from("<I", d, decl - 4)[0]
    print(f"layer list: count={count} decl @0x{decl:x}")

    # pass 1: no pad — the walk2.rs deterministic anchor (§4s, twice-revised).
    # Multi-layer files: the 2nd list layer arrives as a class-ref naming the
    # TRUE CLayer class slot (base = idx - 1). Single-layer files: the very
    # next record is the def-list header, which OPENS with an object pointer
    # to the DEFAULT LAYER — always the slot right after the CLayer class,
    # so base = pointer - 2. (The old stall-driven fallback is REFUTED: the
    # first out-of-range class-ref can be a def's attribute pointer, or name
    # a class declared inside the throwaway walk whose index is base-shifted.)
    ar = make_archive(d, decl, 0)
    base = None
    first = ar.read_object()
    nxt = struct.unpack_from("<H", d, ar.pos)[0]
    if nxt & 0x8000 and nxt != 0xFFFF:
        base = (nxt & 0x7FFF) - 1
    else:
        ptr = struct.unpack_from("<I", d, ar.pos + 2)[0] if nxt == 0x7FFF else nxt
        if 3 <= ptr <= 1_000_000:
            base = ptr - 2
    assert base is not None, "calibration failed"
    print(f"calibrated archive base: {base} pre-existing slots")

    ar = make_archive(d, decl, base)
    layers = [ar.read_object() for _ in range(count)]
    for l in layers:
        print(f"  layer {l['name']!r} internal={l['internal']!r} rgba={l['rgba']} hidden={l['hidden']}")
    print(f"after layers pos=0x{ar.pos:x}")
    hexdump(d, ar.pos, 32)

    # definition list: u16 + u32 count observed
    w = ar.u2()
    ndef = ar.u4()
    print(f"def-list: u16={w:#x} count={ndef}")
    defs = []
    for i in range(ndef):
        at = ar.pos
        try:
            o = ar.read_object()
            defs.append(o)
            nm = o.get("name") if isinstance(o, dict) else o
            print(f"  def[{i}] @0x{at:x}: {nm!r} entities={len(o.get('entities', [])) if isinstance(o, dict) else '?'}")
        except Stall as s:
            print(f"  def[{i}] @0x{at:x}: STALL {s.classname} @0x{s.pos:x} (map#{s.mapindex})")
            for (sp, sw) in getattr(ar, "_ring", [])[-28:]:
                e = ar.map[sw & 0x7fff] if (sw & 0x8000) and (sw & 0x7fff) < len(ar.map) else None
                nm = e[1] if isinstance(e, tuple) else ""
                print(f"    stack: @0x{sp:x} tag=0x{sw:04x} {nm}")
            hexdump(d, s.pos, 96)
            break
        except AssertionError as e:
            print(f"  def[{i}] @0x{at:x}: ASSERT {e}")
            hexdump(d, ar.pos, 144)
            break
    else:
        print(f"ALL DEFS OK; pos=0x{ar.pos:x}")
        hexdump(d, ar.pos, 96)
        # trailing definitions (incl. the model root) follow back-to-back
        k = len(defs)
        while struct.unpack_from("<H", d, ar.pos)[0] == 0x8023:
            at = ar.pos
            try:
                o = ar.read_object()
                defs.append(o)
                print(f"  def[{k}]* @0x{at:x}: {o.get('name')!r} entities={len(o.get('entities',[]))}")
                for e in o.get("entities", []):
                    if isinstance(e, dict) and e.get("__class__") in ("CGroup","CComponentInstance"):
                        t = e["transform"]
                        print(f"     inst {e['__class__']} defref={e['defref']} T=({t[9]:.2f},{t[10]:.2f},{t[11]:.2f})in")
                k += 1
            except (Stall, AssertionError) as e:
                print(f"  def* @0x{at:x}: FAIL {e}")
                import traceback; traceback.print_exc()
                hexdump(d, max(0,ar.pos-16), 160)
                break
        else:
            print(f"ALL TRAILING DEFS OK; pos=0x{ar.pos:x}")
            rcount = ar.u4()
            print(f"root list count={rcount}")
            from collections import Counter
            kinds = Counter()
            for i in range(rcount):
                at = ar.pos
                try:
                    o = ar.read_object()
                    kinds[o.get("__class__") if isinstance(o, dict) else str(type(o).__name__)] += 1
                    if isinstance(o, dict) and o.get("__class__") in ("CGroup","CComponentInstance"):
                        t = o["transform"]
                        print(f"  root[{i}] {o['__class__']} defref={o['defref']} name={o.get('name','')!r} T=({t[9]:.1f},{t[10]:.1f},{t[11]:.1f})in")
                except (Stall, AssertionError) as e:
                    print(f"  root[{i}] @0x{at:x}: FAIL {e}")
                    hexdump(d, max(0,at-16), 160)
                    break
            else:
                print(f"ROOT LIST OK: {dict(kinds)}; pos=0x{ar.pos:x}")
                validate(ar, defs)

def validate(ar, defs):
    from collections import Counter
    cls = Counter()
    hidden = []
    refs_unresolved = 0
    for i, e in enumerate(ar.map):
        if isinstance(e, dict):
            c = e.get("__class__")
            cls[c] += 1
            if e.get("hidden"):
                hidden.append((i, e))
    # unresolved Ref children in edges
    def chk(v):
        return isinstance(v, Ref)
    for e in ar.map:
        if isinstance(e, dict) and e.get("__class__") == "CEdge":
            refs_unresolved += chk(e["v0"]) + chk(e["v1"])
        if isinstance(e, dict) and e.get("__class__") == "CEdgeUse":
            refs_unresolved += chk(e.get("edge")) + chk(e.get("parent"))
    print("class totals:", {k: v for k, v in cls.most_common()})
    print(f"unresolved refs in edges/edgeuses: {refs_unresolved}")
    M = 39.37007874015748
    for i, e in hidden:
        c = e["__class__"]
        if c == "CEdge":
            v0, v1 = e["v0"], e["v1"]
            a = v0["xyz"] if isinstance(v0, dict) else "?"
            b = v1["xyz"] if isinstance(v1, dict) else "?"
            print(f"HIDDEN {c} pid={e['pid']:#x} map#{i} {a} -> {b} (inches)")
        elif c == "CFace":
            print(f"HIDDEN {c} pid={e['pid']:#x} map#{i} plane={e['plane']}")
        else:
            print(f"HIDDEN {c} pid={e.get('pid',0):#x} map#{i}")

if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "corpus/2017/house.skp")
