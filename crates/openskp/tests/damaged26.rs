//! A damaged or extended 2026 file degrades instead of failing: a record
//! this reader does not know is reported, and a damaged entity container
//! loses only what follows the damage, with the loss recorded as a
//! `Diagnostic::Skipped`. Both are built from `corpus/2026/feature-pack.skp`
//! by rewriting its archive with stored (uncompressed) entries.

use std::path::PathBuf;

use openskp::{Diagnostic, Model};

fn corpus(rel: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../corpus");
    p.push(rel);
    std::fs::read(p).unwrap()
}

/// The archive's offset in a 2026 file: the first local-header signature
/// after the header's string records.
fn zip_start(file: &[u8]) -> usize {
    file.windows(4).position(|w| w == b"PK\x03\x04").unwrap()
}

fn u16_at(d: &[u8], o: usize) -> usize {
    u16::from_le_bytes([d[o], d[o + 1]]) as usize
}

fn u32_at(d: &[u8], o: usize) -> usize {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize
}

/// The archive's entries, inflated, from its central directory.
fn entries(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
    // The archive's own end record: the central directory it names ends
    // exactly where it begins (embedded `.skc` archives have their own).
    let eocd = (0..=zip.len() - 22)
        .rev()
        .find(|&i| {
            zip[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]
                && u32_at(zip, i + 16) + u32_at(zip, i + 12) == i
        })
        .unwrap();
    let count = u16_at(zip, eocd + 10);
    let shift = 0;
    let mut o = u32_at(zip, eocd + 16);
    let mut out = Vec::new();
    for _ in 0..count {
        assert_eq!(&zip[o..o + 4], b"PK\x01\x02");
        let method = u16_at(zip, o + 10);
        let compressed = u32_at(zip, o + 20);
        let size = u32_at(zip, o + 24);
        let (nlen, xlen, clen) = (
            u16_at(zip, o + 28),
            u16_at(zip, o + 30),
            u16_at(zip, o + 32),
        );
        let local = u32_at(zip, o + 42) + shift;
        let name = String::from_utf8(zip[o + 46..o + 46 + nlen].to_vec()).unwrap();
        let data_at = local + 30 + u16_at(zip, local + 26) + u16_at(zip, local + 28);
        let raw = &zip[data_at..data_at + compressed];
        let data = match method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec(raw).unwrap(),
            m => panic!("method {m}"),
        };
        assert_eq!(data.len(), size);
        out.push((name, data));
        o += 46 + nlen + xlen + clen;
    }
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn write_stored(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let crc = crc32(data);
        let local = out.len() as u32;
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // version, flags, method 0, time, date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]); // extra, comment, disk, attrs
        central.extend_from_slice(&local.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_start = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// `feature-pack.skp` with its `model.dat` replaced by `edit(model.dat)`.
fn rebuilt(edit: impl Fn(&mut Vec<u8>)) -> Vec<u8> {
    let file = corpus("2026/feature-pack.skp");
    let start = zip_start(&file);
    let mut entries = entries(&file[start..]);
    for (name, data) in &mut entries {
        if name == "model.dat" {
            edit(data);
        }
    }
    let mut out = file[..start].to_vec();
    out.extend(write_stored(&entries));
    out
}

fn skipped(m: &Model) -> Vec<&str> {
    m.diagnostics
        .iter()
        .filter_map(|d| match d {
            Diagnostic::Skipped { class, .. } => Some(class.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_rebuilt_archive_reads_like_the_original() {
    let a = Model::parse(&corpus("2026/feature-pack.skp")).unwrap();
    let b = Model::parse(&rebuilt(|_| {})).unwrap();
    assert_eq!(a.definitions, b.definitions);
    assert_eq!(a.geometry.len(), b.geometry.len());
    assert!(b.diagnostics.is_empty(), "{:?}", b.diagnostics);
}

#[test]
fn an_unknown_top_level_record_is_reported_not_dropped() {
    // Splice a `0xbeef` record with a 4-byte payload after the top
    // container's header and grow that container's length to match.
    let m = Model::parse(&rebuilt(|d| {
        let len = u32::from_le_bytes([d[2], d[3], d[4], d[5]]);
        let rec = [0xef, 0xbe, 4, 0, 0, 0, 1, 2, 3, 4];
        d.splice(6..6, rec);
        d[2..6].copy_from_slice(&(len + rec.len() as u32).to_le_bytes());
    }))
    .unwrap();
    assert_eq!(skipped(&m), ["unknown top-level record 0xbeef"]);
    let a = Model::parse(&corpus("2026/feature-pack.skp")).unwrap();
    assert_eq!(a.definitions, m.definitions, "the model still reads");
}

/// The record tree's path to the first definition's entity container:
/// `(offset of each ancestor's length field, container payload offset)`.
fn first_definition_container(d: &[u8]) -> (Vec<usize>, usize) {
    fn kids(d: &[u8], start: usize, end: usize) -> Vec<(u16, usize, usize)> {
        let mut out = Vec::new();
        let mut o = start;
        while o + 6 <= end {
            let tag = u16::from_le_bytes([d[o], d[o + 1]]);
            let len = u32::from_le_bytes([d[o + 2], d[o + 3], d[o + 4], d[o + 5]]) as usize;
            out.push((tag, o + 6, o + 6 + len));
            o += 6 + len;
        }
        out
    }
    let mut lens = vec![2usize]; // the top container's length field
    let (mut s, mut e) = (6usize, d.len());
    for tag in [0x01f9u16, 0x1770, 0x1771, 0x157c, 0x1388] {
        let (_, a, z) = kids(d, s, e).into_iter().find(|k| k.0 == tag).unwrap();
        lens.push(a - 4);
        (s, e) = (a, z);
    }
    (lens, s)
}

fn grow(d: &mut [u8], at: usize, by: usize) {
    let v = u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]]) as usize + by;
    d[at..at + 4].copy_from_slice(&(v as u32).to_le_bytes());
}

#[test]
fn an_unknown_record_inside_an_entity_container_is_reported() {
    let a = Model::parse(&corpus("2026/feature-pack.skp")).unwrap();
    let m = Model::parse(&rebuilt(|d| {
        let (lens, at) = first_definition_container(d);
        let rec = [0xef, 0xbe, 4, 0, 0, 0, 1, 2, 3, 4];
        d.splice(at..at, rec);
        for l in lens {
            grow(d, l, rec.len());
        }
    }))
    .unwrap();
    assert_eq!(skipped(&m), ["unknown entity container record 0xbeef"]);
    assert_eq!(a.definitions, m.definitions, "the model still reads");
    let vertices = |m: &Model| {
        m.geometry
            .iter()
            .map(|r| r.topology.vertices)
            .sum::<usize>()
    };
    assert_eq!(vertices(&a), vertices(&m));
}

#[test]
fn damage_inside_an_entity_container_is_skipped_and_recorded() {
    let a = Model::parse(&corpus("2026/feature-pack.skp")).unwrap();
    let vertices = |m: &Model| {
        m.geometry
            .iter()
            .map(|r| r.topology.vertices)
            .sum::<usize>()
    };
    // The largest run's container: overwrite 64 bytes 3000 bytes in, which
    // lands inside its vertex pool.
    let run = a
        .geometry
        .iter()
        .max_by_key(|r| r.topology.vertices)
        .unwrap();
    let at = run.start + 3000;
    let m = Model::parse(&rebuilt(|d| d[at..at + 64].fill(0xff))).unwrap();
    let classes = skipped(&m);
    assert!(
        classes.iter().any(|c| c.starts_with("damaged pool")),
        "{:?}",
        m.diagnostics
    );
    assert_eq!(a.definitions, m.definitions, "definitions survive");
    // The damaged run is still there, with the vertices before the damage
    // and fewer than before; every other run is unchanged.
    let hit = m
        .geometry
        .iter()
        .find(|r| r.def_index == run.def_index)
        .expect("the damaged run survives");
    assert!(hit.topology.vertices > 0 && hit.topology.vertices < run.topology.vertices);
    assert_eq!(
        vertices(&a) - run.topology.vertices,
        vertices(&m) - hit.topology.vertices,
        "runs other than the damaged one are intact"
    );
}

/// Offsets of the record headers on the path to the first definition:
/// the `0x01f9` section, the `0x1771` list and the first `0x157c` record.
fn definition_path_headers(d: &[u8]) -> Vec<usize> {
    fn kids(d: &[u8], start: usize, end: usize) -> Vec<(u16, usize, usize)> {
        let mut out = Vec::new();
        let mut o = start;
        while o + 6 <= end {
            let tag = u16::from_le_bytes([d[o], d[o + 1]]);
            let len = u32::from_le_bytes([d[o + 2], d[o + 3], d[o + 4], d[o + 5]]) as usize;
            out.push((tag, o + 6, o + 6 + len));
            o += 6 + len;
        }
        out
    }
    let mut heads = Vec::new();
    let (mut s, mut e) = (6usize, d.len());
    for tag in [0x01f9u16, 0x1770, 0x1771] {
        let (_, a, z) = kids(d, s, e).into_iter().find(|k| k.0 == tag).unwrap();
        heads.push(a - 6);
        (s, e) = (a, z);
    }
    // The second definition's header.
    let (_, a, _) = kids(d, s, e)
        .into_iter()
        .filter(|k| k.0 == 0x157c)
        .nth(1)
        .unwrap();
    heads.push(a - 6);
    heads
}

#[test]
fn a_damaged_top_level_record_header_loses_only_what_follows_it() {
    let a = Model::parse(&corpus("2026/feature-pack.skp")).unwrap();
    // Overwrite the definitions section's own header: the file still
    // parses, with the loss of that record and everything after it recorded.
    let m = Model::parse(&rebuilt(|d| {
        let at = definition_path_headers(d)[0];
        d[at..at + 6].fill(0xff);
    }))
    .unwrap();
    assert!(
        skipped(&m).contains(&"damaged top-level records"),
        "{:?}",
        m.diagnostics
    );
    assert_eq!(a.version, m.version);
    assert_eq!(a.model_guid, m.model_guid);
    assert!(
        m.definitions.is_empty(),
        "the definitions section was the damaged one"
    );
}

#[test]
fn a_damaged_definition_header_keeps_the_other_definitions() {
    let a = Model::parse(&corpus("2026/feature-pack.skp")).unwrap();
    let m = Model::parse(&rebuilt(|d| {
        let at = definition_path_headers(d)[3];
        d[at..at + 6].fill(0xff);
    }))
    .unwrap();
    assert!(
        skipped(&m).iter().any(|c| c.starts_with("damaged pool")),
        "{:?}",
        m.diagnostics
    );
    assert_eq!(
        m.definitions.len(),
        1,
        "only the definition before the damage"
    );
    assert_eq!(a.materials, m.materials);
}
