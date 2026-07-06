#!/usr/bin/env python3
"""carchive — a faithful Microsoft MFC `CArchive` object-stream reader.

This is the engine the .skp format needs: it implements MFC's object tagging
protocol (null / object back-reference / new-class / existing-class, with the
big-tag escapes) and the shared 1-based map index that classes and objects
both consume. Body parsing is pluggable per class via `register()`.

Reference: MFC CArchive::ReadObject / ReadClass. Constants:
  wNullTag=0x0000  wNewClassTag=0xFFFF  wClassTag=0x8000
  wBigObjectTag=0x7FFF  dwBigClassTag=0x80000000
A class-ref tag is (0x8000 | mapIndex); an object back-ref tag is the object's
mapIndex with the high bit clear. Both indices live in one running counter.
"""
import struct


class Stall(Exception):
    """Raised when the walker reaches a class with no registered body reader.
    Carries the exact resume point — this is the decode-as-you-go worklist."""
    def __init__(self, classname, pos, mapindex):
        self.classname, self.pos, self.mapindex = classname, pos, mapindex
        super().__init__(f"no reader for {classname!r} @0x{pos:x} (map#{mapindex})")


class Ref:
    """An object back-reference whose target isn't in our (partial) map yet."""
    __slots__ = ("index",)
    def __init__(self, index): self.index = index
    def __repr__(self): return f"Ref(#{self.index})"


class CArchive:
    WNULL, WNEWCLASS, WCLASS, WBIGOBJ, DWBIGCLASS = 0, 0xFFFF, 0x8000, 0x7FFF, 0x80000000

    def __init__(self, data, pos=0):
        self.d = data
        self.pos = pos
        self.map = [None]        # 1-based; map[i] = ("class",name) or an object dict
        self.readers = {}        # classname -> fn(archive, obj_dict)
        self.classes = []        # names in definition order (diagnostics)
        self.seed_classes = {}   # map_index -> classname, for sub-stream walks
                                 # (lets class-ref tags resolve without a full walk)

    # ---- primitives ----
    def u1(self):
        v = self.d[self.pos]; self.pos += 1; return v
    def u2(self):
        v = struct.unpack_from("<H", self.d, self.pos)[0]; self.pos += 2; return v
    def u4(self):
        v = struct.unpack_from("<I", self.d, self.pos)[0]; self.pos += 4; return v
    def f8(self):
        v = struct.unpack_from("<d", self.d, self.pos)[0]; self.pos += 8; return v
    def take(self, n):
        v = self.d[self.pos:self.pos+n]; self.pos += n; return v
    def utf16(self):
        assert self.d[self.pos:self.pos+3] == b"\xff\xfe\xff", f"str rec @{self.pos:#x}"
        n = self.d[self.pos+3]
        s = self.d[self.pos+4:self.pos+4+n*2].decode("utf-16le")
        self.pos += 4 + n*2
        return s

    def register(self, name, fn):
        self.readers[name] = fn

    # ---- the MFC object protocol ----
    def read_object(self):
        """Read one tagged object, with a recursion-depth guard so a desynced
        walk stalls instead of crashing with RecursionError."""
        self._depth = getattr(self, "_depth", 0) + 1
        try:
            if self._depth > 400:
                raise Stall("<recursion limit>", self.pos, self._depth)
            return self._read_object()
        finally:
            self._depth -= 1

    def _read_object(self):
        start = self.pos
        wtag = self.u2()
        if wtag == self.WNULL:
            return None
        if wtag == self.WBIGOBJ:
            obtag = self.u4()
            is_class = bool(obtag & self.DWBIGCLASS)
            idx = obtag & ~self.DWBIGCLASS
            new_class = False
        elif wtag == self.WNEWCLASS:
            is_class, new_class, idx = True, True, None
        else:
            is_class = bool(wtag & self.WCLASS)
            idx = wtag & 0x7FFF
            new_class = False

        if not is_class:                      # object back-reference
            if idx == 0:
                return None
            if idx < len(self.map) and not (isinstance(self.map[idx], tuple)):
                return self.map[idx]
            return Ref(idx)

        if new_class:                          # define a new class, then its object
            schema = self.u2()
            namelen = self.u2()
            raw = self.d[self.pos:self.pos + namelen]
            if namelen > 64 or not raw.isascii():   # garbage => we've desynced
                self.pos = start
                raise Stall("<bad class name>", start, len(self.map))
            self.pos += namelen
            name = raw.decode("ascii")
            self.map.append(("class", name))
            self.classes.append(name)
        else:                                  # object of an already-seen class
            entry = self.map[idx] if idx < len(self.map) else None
            if isinstance(entry, tuple) and entry[0] == "class":
                name = entry[1]
            elif idx in self.seed_classes:      # seeded for a sub-stream walk
                name = self.seed_classes[idx]
            else:
                raise Stall(f"<class-ref #{idx}>", start, idx)

        obj = {"__class__": name}
        self.map.append(obj)                   # register BEFORE body (cycle-safe)
        rdr = self.readers.get(name)
        if rdr is None:
            self.pos = start                   # rewind so caller sees the tag
            raise Stall(name, start, len(self.map) - 1)
        rdr(self, obj)
        return obj
