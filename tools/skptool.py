#!/usr/bin/env python3
"""skptool — clean-room analysis helpers for the SketchUp .skp format.

No Trimble SDK or SDK-derived knowledge is used. Everything here is derived
from observing files we authored ourselves. See ../docs/SKP_FORMAT.md.

Subcommands:
  id      <file>            magic + version string + format GUID
  classes <file>           enumerate MFC class names (CArchive new-class tags)
  diff    <a> <b>          byte diff with common prefix/suffix bounding
  window  <file> <off> <n> hex dump a window (off/n accept 0x.. hex)
  strings16 <file>         dump UTF-16LE length-prefixed string records
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
    # 16-byte value after the two strings = stable format GUID (see SKP_FORMAT)
    print(f"  format GUID @0x{off:x}: {d[off:off+16].hex()}")


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


if __name__ == "__main__":
    args = sys.argv[1:]
    if not args:
        print(__doc__); sys.exit(1)
    cmd, rest = args[0], args[1:]
    {
        "id": cmd_id, "classes": cmd_classes, "diff": cmd_diff,
        "window": cmd_window, "strings16": cmd_strings16,
        "walk": cmd_walk, "classdelta": cmd_classdelta, "geom": cmd_geom,
    }[cmd](*rest)
