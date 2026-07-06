# Contributing to OpenSKP

Thank you for your interest in OpenSKP. Contributions are welcome, but this
project has two unusual constraints — a clean-room evidence policy and a
licensing grant — that every contribution must satisfy. Please read this
whole document before opening a pull request.

## The clean-room rule (non-negotiable)

OpenSKP documents and reads the `.skp` format **without** the Trimble
SketchUp SDK. Every format fact in this repository derives from one of:

- observing `.skp` files (authored samples or contributed third-party files),
- their COLLADA (`.dae`) exports, or
- general public knowledge of Microsoft MFC `CArchive` serialization.

Contributions must never introduce knowledge taken from the SketchUp C SDK,
the Ruby API implementation, SDK headers, SDK constants, decompiled
SketchUp binaries, or any material derived from those sources. If you have
studied the Trimble SDK's serialization internals, please do not contribute
format findings here — that protection is what makes this project
distributable.

Practical consequences:

- **Cite evidence.** A format claim needs a corpus file and byte offsets
  (or a reproducible scan) backing it. Unverified inferences are marked as
  candidates, never stated as fact.
- **The corpus is the oracle.** No parser change merges unless
  `cargo test --workspace --release` and `scripts/verify.sh` pass on the
  whole corpus.
- **New corpus files** should be minimal pairs: drawn from the origin,
  along axes, with typed exact dimensions, and accompanied by a COLLADA
  export of the same basename. Note what the file isolates relative to an
  existing sample.

## Licensing and the contributor grant

OpenSKP is licensed under the **GNU General Public License v3.0 only**
(`GPL-3.0-only`, see `LICENSE`). The project additionally reserves the
right to offer the code under separate commercial license terms in the
future (dual licensing).

To keep that possible, contributions are accepted under the following
terms, which you agree to by submitting a contribution:

1. **Inbound license.** You license your contribution under GPL-3.0-only.
2. **Relicensing grant.** You additionally grant the project maintainer
   (Kurt Granroth) a perpetual, worldwide, non-exclusive, royalty-free
   right to relicense your contribution, in whole or in part, under other
   license terms, including proprietary or commercial terms. You retain
   copyright in your contribution and all rights to use it elsewhere.
3. **Developer Certificate of Origin.** Every commit must be signed off
   (`git commit -s`), certifying the [Developer Certificate of Origin
   v1.1](https://developercertificate.org/): that you wrote the change or
   otherwise have the right to submit it under these terms.

If you cannot agree to the relicensing grant, please open an issue to
discuss instead of submitting code.

## Commit conventions

- Conventional Commits titles with an optional scope: `feat(sdk): …`,
  `docs(format): …`, `chore: …`.
- Write commit bodies for people: what changed and why, not a byte-level
  changelog — the diff carries the details.
- Sign off every commit (`git commit -s`).

## Style

- Documentation and comments are concise, impersonal, and independent of
  dates: state facts about the format and the code, not the narrative of
  discovering them.
- Rust code carries no runtime dependencies in the core crate; test-only
  dependencies belong under `dev-dependencies`.
- Python tooling is standard-library only.
