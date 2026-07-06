#!/usr/bin/env python3
"""Parse every corpus .skp with the COMPILED Kaitai spec and cross-check the
header fields against the reference parser. Requires `tools/gen/skp.py`
(produce it with `node tools/compile_ksy.js ksy/skp.ksy tools/gen`)."""
import sys, glob, os
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "gen"))
sys.path.insert(0, HERE)
from skp import Skp
import skpparse

root = os.path.dirname(HERE)
ok = fail = 0
for f in sorted(glob.glob(os.path.join(root, "corpus", "2017", "*.skp")) + glob.glob(os.path.join(root, "corpus", "legacy", "*.skp"))):
    name = os.path.basename(f)
    try:
        m = Skp.from_file(f)
        ref = skpparse.parse_header(skpparse.read(f))
        assert m.model_tag.value == "SketchUp Model", m.model_tag.value
        assert m.version.value == ref["version"], (m.version.value, ref["version"])
        assert m.format_guid.hex() == ref["format_guid"]
        assert m.first_class.name == "CVersionMap", m.first_class.name
        print(f"  OK   {name:30} ver={m.version.value} cls0={m.first_class.name}")
        ok += 1
    except Exception as e:
        print(f"  FAIL {name:30} {type(e).__name__}: {e}")
        fail += 1
print(f"\nKaitai: {ok}/{ok+fail} corpus files parsed + header cross-checked")
sys.exit(1 if fail else 0)
