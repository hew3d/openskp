# OpenSKP — Agent Instructions

## What this project is

A **clean-room** specification and reader for the SketchUp 2017 `.skp`
binary format (v17.3.116 and nearby), for which no public spec exists.
Deliverables: the format spec (`docs/SKP_FORMAT.md` + `ksy/skp.ksy`), the
Rust SDK (`crates/openskp` + CLI + C ABI), and the evidence corpus.

## Hard rules (non-negotiable)

1. **Clean-room only.** Every format fact must derive from observing
   `.skp` files (authored or third-party), their COLLADA exports, or
   public knowledge of Microsoft MFC `CArchive` serialization. **Never**
   use Trimble/SketchUp code — the Ruby API, C SDK, headers, constants,
   or any SDK-derived knowledge; none of it may enter this repo.
2. **Evidence-based claims.** Record offsets and the corpus file they
   came from. Mark inferred-but-unverified findings as candidates. Do not
   overstate.
3. **The corpus is the oracle.** No parser change lands without
   `cargo test --workspace --release` and `scripts/verify.sh` green on
   the whole corpus. COLLADA exports are ground truth for coordinates,
   connectivity, material RGBA, and UVs (a 1 m box must yield 8 vertices
   / 12 edges / 6 faces).
4. **Public writing style.** Docs, comments, and commit messages are
   concise, impersonal, and independent of dates. Conventional Commits
   titles; no AI attribution or co-author trailers in commits.

## Layout

```
corpus/2017/         authored minimal pairs + .dae ground truth (README lists purposes)
corpus/legacy|future|third-party/   older saves, post-2017 probes, the benchmark model
docs/                SKP_FORMAT.md (the spec — read first), SDK.md, DEVELOPMENT.md
ksy/                 skp.ksy — Kaitai grammar (header + record catalogue)
crates/              openskp (core), openskp-cli, openskp-capi
tools/               Python instruments; frozen reference parser (differential oracle)
scripts/verify.sh    Kaitai + reference-parser verification
```

## Format facts to keep straight (details in docs/SKP_FORMAT.md)

- The whole payload is ONE MFC `CArchive` from 0x50 with a single shared
  1-based store map; back-references are GLOBAL and the pre-model slot
  base is calibrated per file from structural anchors (§4s / spec §3.2).
- Every entity body starts `<attr-container ptr> <pid mask> <pid>`.
- Coordinates are f64 inches; `1 m = 39.37007874015748"`.
- Class tags are per-file (lazy inline definition) — never hardcode them.
- Scale escalations exist and must stay exercised: `7F FF` big-tag past
  0x7FFF slots, u16→u32 prelude decl, 255-char string escalation, pid
  masks past 0xFFFF (spec §11).
- UVs: two row-major 3×3 projective matrices per face + pin lists; UV =
  inverse map ÷ material applied size; no FTC record = identity;
  instance-painted materials inherit down the scene (spec §9).
- Source comments cite `§4x` anchors (resolve via SKP_FORMAT.md
  Appendix B) and `Phase N` labels (resolve via DEVELOPMENT.md).

## Method

- Diff minimal pairs, but byte-diffs only align for same-length in-place
  edits; structural changes shift all later offsets — anchor on typed
  exact coordinates searched as f64 inches instead.
- Decode new structures in `tools/contwalk.py` first (hexdumps on
  stall), then port to `crates/openskp` (`walk2.rs` + `entity.rs`).
- `tools/carchive.py` / `skpwalk.py` / `skpparse.py` are FROZEN — they
  are the differential oracle (`crates/openskp/tests/oracle.json`), not a
  place for new capability.
- `tools/skptool.py` is the byte-level analysis CLI. CAUTION: its
  `window` command has printed wrong regions before — cross-check with
  `xxd -s <offset>`.

## Corpus authoring conventions

Draw from the origin, along axes, typing exact dimensions (line `1m`; box
`1m,1m,1m`) so endpoints are known constants. One isolated change per
file, named against its base pair; export a same-basename `.dae`. Keep
file names stable; document each file's purpose in `corpus/2017/README.md`.

## Acceptance state

Every 2017-era corpus file parses zero-desync on the continuous path;
`house.skp` ≡ `house.dae` and `house-plus.skp` ≡ `house.skp` in world
space; the third-party benchmark matches its export exactly (vertices,
face rings, materials, all textured faces' UVs); the spec's class
catalogue is machine-checked against `CVersionMap`. The legacy path
serves v2013–16 only. Remaining unknowns: `docs/SKP_FORMAT.md` §15.

## Commands

- Build/test: `cargo test --workspace --release`
- Verify grammar + reference parser: `./scripts/verify.sh`
- Analyze bytes: `python3 tools/skptool.py <cmd> corpus/2017/<file>.skp`
  (Python tooling is stdlib-only)
