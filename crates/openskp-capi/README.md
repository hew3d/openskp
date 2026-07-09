# openskp-capi

C ABI for [openskp](https://crates.io/crates/openskp), the clean-room
reader for the SketchUp 2017 `.skp` binary format. Builds `libopenskp`
as a static and shared library with a plain-C header
([`include/openskp.h`](https://github.com/hew3d/openskp/blob/main/crates/openskp-capi/include/openskp.h)),
exposing header identification, model parsing, and JSON output — any
language with a C FFI and a JSON parser can read `.skp` files through
it.

No Trimble SDK anywhere in its lineage; the format specification and
evidence corpus live in the
[repository](https://github.com/hew3d/openskp).

## License

GPL-3.0-only.
