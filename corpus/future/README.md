# corpus/future — post-2017 container probes

Files here are **not** part of the 2017 validation corpus. They record how
the container changed after 2017 and are kept as evidence for future work.

- `box-v2026.skp` — the 1 m box saved by SketchUp 2026 (version id
  `{26.2.0}`). The two leading UTF-16 string records still parse, so
  header identification works, but at offset 0x41 sits `PK\x03\x04` — the
  ZIP local-file-header magic — followed by an entry named
  `meta/model_th…`: **the post-2017 container is a ZIP archive** (string
  record preamble + zip payload), not an MFC `CArchive` stream. Reading it
  would be a new container backend behind the `detect_container` seam, not
  an extension of the CArchive walk. The reader identifies such files and
  refuses them cleanly.
