# openskp-cli

Command-line reader for SketchUp 2017 `.skp` files, built on
[openskp](https://crates.io/crates/openskp) — clean-room, no Trimble
SDK anywhere in its lineage. Installs as the `openskp` binary:

```
cargo install openskp-cli

openskp id    model.skp   # header (version + format GUID) and class count
openskp model model.skp   # human-readable decode summary
openskp json  model.skp   # full model as JSON (stdout)
openskp mesh  model.skp   # concrete mesh + composed scene as JSON (stdout)
```

The format specification and evidence corpus live in the
[repository](https://github.com/hew3d/openskp).

## License

GPL-3.0-only.
