//! C ABI over the `skp` core reader. This is a thin binding layer — no decoding
//! logic lives here. See `include/openskp.h` for the C header.
//!
//! Lifecycle: `openskp_open`/`openskp_parse` return an opaque handle; query it with the
//! accessors; release it with `openskp_free`. Returned strings are owned by the
//! handle and valid until `openskp_free`.

use std::ffi::{c_char, CStr, CString};

/// Opaque handle: the parsed model plus a cache for the JSON string so the
/// pointer returned by `openskp_model_json` stays valid until `openskp_free`.
pub struct OpenSkpModel {
    model: openskp::Model,
    json: Option<CString>,
    mesh_json: Option<CString>,
}

/// Parse a `.skp` from a filesystem path (NUL-terminated UTF-8).
/// Returns NULL on error (bad path, unreadable file, malformed `.skp`).
///
/// # Safety
/// `path` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn openskp_open(path: *const c_char) -> *mut OpenSkpModel {
    if path.is_null() {
        return std::ptr::null_mut();
    }
    let path = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    match openskp::Model::read(path) {
        Ok(model) => Box::into_raw(Box::new(OpenSkpModel { model, json: None, mesh_json: None })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Parse a `.skp` from an in-memory buffer. Returns NULL on error.
///
/// # Safety
/// `data` must point to at least `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn openskp_parse(data: *const u8, len: usize) -> *mut OpenSkpModel {
    if data.is_null() {
        return std::ptr::null_mut();
    }
    let bytes = std::slice::from_raw_parts(data, len);
    match openskp::Model::parse(bytes) {
        Ok(model) => Box::into_raw(Box::new(OpenSkpModel { model, json: None, mesh_json: None })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// The whole model as a NUL-terminated UTF-8 JSON string, owned by `m` (valid
/// until `openskp_free`). Returns NULL if `m` is NULL.
///
/// # Safety
/// `m` must be a handle returned by `openskp_open`/`openskp_parse` and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn openskp_model_json(m: *mut OpenSkpModel) -> *const c_char {
    let handle = match m.as_mut() {
        Some(h) => h,
        None => return std::ptr::null(),
    };
    if handle.json.is_none() {
        // to_json() never contains interior NUL (JSON escapes control chars).
        handle.json = CString::new(handle.model.to_json()).ok();
    }
    match &handle.json {
        Some(c) => c.as_ptr(),
        None => std::ptr::null(),
    }
}

/// The concrete mesh as NUL-terminated UTF-8 JSON, owned by `m` (valid until
/// `openskp_free`): per-run vertices (metres) / edges / faces, plus the composed
/// `scene` leaves (run + row-major 4×4 world matrix + inherited material
/// slot) — everything needed to assemble the world-space model. Returns NULL
/// if `m` is NULL.
///
/// # Safety
/// `m` must be a handle returned by `openskp_open`/`openskp_parse` and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn openskp_mesh_json(m: *mut OpenSkpModel) -> *const c_char {
    let handle = match m.as_mut() {
        Some(h) => h,
        None => return std::ptr::null(),
    };
    if handle.mesh_json.is_none() {
        handle.mesh_json = CString::new(handle.model.mesh_json()).ok();
    }
    match &handle.mesh_json {
        Some(c) => c.as_ptr(),
        None => std::ptr::null(),
    }
}

/// Number of component/group definitions. 0 if `m` is NULL.
///
/// # Safety
/// `m` must be a valid handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn openskp_definition_count(m: *const OpenSkpModel) -> usize {
    match m.as_ref() {
        Some(h) => h.model.definitions.len(),
        None => 0,
    }
}

/// Number of decoded geometry runs. 0 if `m` is NULL.
///
/// # Safety
/// `m` must be a valid handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn openskp_geometry_run_count(m: *const OpenSkpModel) -> usize {
    match m.as_ref() {
        Some(h) => h.model.geometry.len(),
        None => 0,
    }
}

/// Release a handle from `openskp_open`/`openskp_parse`. NULL is ignored.
///
/// # Safety
/// `m` must have come from `openskp_open`/`openskp_parse` and not been freed already.
#[no_mangle]
pub unsafe extern "C" fn openskp_free(m: *mut OpenSkpModel) {
    if !m.is_null() {
        drop(Box::from_raw(m));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(name: &str) -> Vec<u8> {
        let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("../../corpus/2017");
        p.push(name);
        std::fs::read(p).unwrap()
    }

    #[test]
    fn ffi_roundtrip() {
        let data = corpus("box.skp");
        unsafe {
            let m = openskp_parse(data.as_ptr(), data.len());
            assert!(!m.is_null());
            let json = openskp_model_json(m);
            assert!(!json.is_null());
            let s = CStr::from_ptr(json).to_str().unwrap();
            assert!(s.contains("\"version\""));
            assert!(s.contains("{17.3.116}"));
            // §4s continuous walk: box.skp's model is the default template's
            // figure definition (1, with its own run) + the drawn cube's
            // ROOT run — the old 0/1 came from the legacy byte-scan path.
            assert_eq!(openskp_definition_count(m), 1);
            assert_eq!(openskp_geometry_run_count(m), 2);
            // Phase 6.3: the mesh surface — the drawn cube's 8 vertices
            // arrive through the C ABI.
            let mesh = openskp_mesh_json(m);
            assert!(!mesh.is_null());
            let s = CStr::from_ptr(mesh).to_str().unwrap();
            assert!(s.contains("\"vertices_m\"") && s.contains("\"scene\""));
            openskp_free(m);

            // NULL-safety
            assert!(openskp_model_json(std::ptr::null_mut()).is_null());
            assert_eq!(openskp_definition_count(std::ptr::null()), 0);
            openskp_free(std::ptr::null_mut());
        }
    }
}
