/* openskp.h — C ABI for the skp SketchUp .skp reader.
 *
 * Link against libopenskp (staticlib or cdylib) built from crates/openskp-capi.
 * All decoding lives in the Rust core; this is a binding only.
 *
 * Lifecycle:
 *   openskp_model *m = openskp_open("model.skp");   // or openskp_parse(buf, len)
 *   if (m) {
 *       const char *json = openskp_model_json(m);   // owned by m
 *       ...
 *       openskp_free(m);
 *   }
 */
#ifndef OPENSKP_H
#define OPENSKP_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque parsed-model handle. */
typedef struct OpenSkpModel openskp_model;

/* Parse from a filesystem path (NUL-terminated UTF-8). NULL on error. */
openskp_model *openskp_open(const char *path);

/* Parse from an in-memory buffer. NULL on error. */
openskp_model *openskp_parse(const unsigned char *data, size_t len);

/* Whole model as NUL-terminated UTF-8 JSON, owned by m (valid until openskp_free).
 * NULL if m is NULL. */
const char *openskp_model_json(openskp_model *m);

/* Concrete mesh as NUL-terminated UTF-8 JSON, owned by m (valid until
 * openskp_free): per-run vertices (metres) / edges / faces (ordered rings),
 * plus the composed "scene" leaves (run index + row-major 4x4 world matrix
 * + inherited material slot) for world-space assembly. NULL if m is NULL. */
const char *openskp_mesh_json(openskp_model *m);

/* Number of component/group definitions. 0 if m is NULL. */
size_t openskp_definition_count(const openskp_model *m);

/* Number of decoded geometry runs. 0 if m is NULL. */
size_t openskp_geometry_run_count(const openskp_model *m);

/* Release a handle from openskp_open/openskp_parse. NULL is ignored. */
void openskp_free(openskp_model *m);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* OPENSKP_H */
