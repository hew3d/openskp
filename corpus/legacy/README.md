# corpus/legacy — SketchUp 2013–2016 saves

The same 1 m box saved by four earlier SketchUp releases. These share the
2017 outer container (header string records, version string, format GUID,
MFC `CArchive` object stream) but their class schemas and body layouts
differ, so they are **not** part of the zero-desync 2017 corpus.

| file | version string |
|---|---|
| `box-v2013.skp` | `{13.0.1}` |
| `box-v2014.skp` | `{14.0.1}` |
| `box-v2015.skp` | `{15.0.1}` |
| `box-v2016.skp` | `{16.0.1}` |

What they pin down:

- Header identification (`version` + format GUID) works across 2013–2017.
- The reader's bar for pre-2017 bodies is *parse without panic, degrade
  loudly* — these files exercise the legacy fallback path and its
  degradation diagnostics.
- Class schema numbers vary per release (e.g. `CArcCurve` declares
  schema 2 in 2013/14 vs 3 in 2017), which is why body readers are keyed
  by `(class, schema range)` and never assume 2017's numbers.
