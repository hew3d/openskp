//! Version/container context (Phase 0.1; see `docs/DEVELOPMENT.md`).
//!
//! `detect_container` is the seam where any future outer-container change
//! lands: the post-2017 releases abandoned the 2013–2017 layout (evidence:
//! `corpus/future/box-v2026.skp` — the doc-name string-record magic at the
//! header's tail is absent), so an arbitrary file must be classified before
//! the CArchive walk is attempted at all.
//!
//! `Ctx` carries the file's own version string and its `CVersionMap`
//! class→schema table; it rides on every `CArchive` (see `carchive.rs`) so
//! body readers can consult the schema the FILE declares rather than
//! assuming 2017's (Phase 0.2 builds the registry lookup on top of this).

use std::collections::HashMap;

use crate::{header, inventory};

/// The outer container of a `.skp` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// SketchUp ~2013–2017: fixed UTF-16 string-record header, then an
    /// uncompressed MFC `CArchive` object stream opening with the
    /// `CVersionMap` new-class record. The only container this crate reads.
    Carchive2017,
    /// Anything else — including the post-2017 layout.
    Unknown,
}

/// Classify the outer container. Evidence-based, no version-number
/// assumptions: the 2013–2017 discriminator is that the fixed header's
/// string-record chain parses AND the object stream opens with an MFC
/// new-class tag (`FF FF` — `CVersionMap` in every observed 2013–2017 file).
pub fn detect_container(d: &[u8]) -> Container {
    match header::object_stream_start(d) {
        Some(start) if start + 2 <= d.len() && d[start..start + 2] == [0xFF, 0xFF] => {
            Container::Carchive2017
        }
        _ => Container::Unknown,
    }
}

/// Parse context threaded through the walk (Phase 0.1).
#[derive(Debug, Clone)]
pub struct Ctx {
    /// The file's version string with the braces stripped, e.g. `17.3.116`.
    pub file_version: String,
    pub container: Container,
    /// `CVersionMap` class → schema, as the FILE declares them.
    pub class_schemas: HashMap<String, u32>,
}

impl Ctx {
    /// Build the context for a file: container check, header version, and one
    /// `CVersionMap` read. `None` when the container is not [`Container::Carchive2017`]
    /// or the version map is unreadable.
    pub fn of(d: &[u8]) -> Option<Ctx> {
        let container = detect_container(d);
        if container != Container::Carchive2017 {
            return None;
        }
        let hdr = header::parse_header(d)?;
        let class_schemas: HashMap<String, u32> = inventory(d).ok()?.into_iter().collect();
        Some(Ctx {
            file_version: hdr.version.trim_matches(['{', '}']).to_string(),
            container,
            class_schemas,
        })
    }

    /// The schema number the file declares for `class`, if any.
    pub fn schema(&self, class: &str) -> Option<u32> {
        self.class_schemas.get(class).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(name: &str) -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpus/2017")
            .join(name);
        std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    #[test]
    fn corpus_2017_files_are_carchive2017() {
        for f in [
            "box.skp",
            "blank-template-box.skp",
            "../legacy/box-v2013.skp",
            "../legacy/box-v2016.skp",
        ] {
            assert_eq!(detect_container(&corpus(f)), Container::Carchive2017, "{f}");
        }
    }

    #[test]
    fn post_2017_container_is_unknown() {
        assert_eq!(
            detect_container(&corpus("../future/box-v2026.skp")),
            Container::Unknown
        );
    }

    #[test]
    fn garbage_is_unknown() {
        assert_eq!(detect_container(&[]), Container::Unknown);
        assert_eq!(detect_container(&[0u8; 64]), Container::Unknown);
    }

    #[test]
    fn ctx_of_box_reads_version_and_schemas() {
        let ctx = Ctx::of(&corpus("box.skp")).expect("ctx");
        assert_eq!(ctx.file_version, "17.3.116");
        assert_eq!(ctx.container, Container::Carchive2017);
        assert!(ctx.schema("CFace").is_some(), "CFace in the version map");
        assert!(ctx.class_schemas.len() > 30);
    }

    #[test]
    fn ctx_of_post_2017_is_none() {
        assert!(Ctx::of(&corpus("../future/box-v2026.skp")).is_none());
    }
}
