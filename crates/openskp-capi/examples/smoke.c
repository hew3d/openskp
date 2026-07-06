/* Minimal C consumer of libopenskp.
 *
 * Build (from repo root):
 *   cargo build -p openskp-capi --release
 *   cc crates/openskp-capi/examples/smoke.c \
 *      -I crates/openskp-capi/include \
 *      -L target/release -lskp \
 *      -o /tmp/openskp_smoke
 *   /tmp/openskp_smoke corpus/2017/box.skp
 *
 * On macOS you may need: -framework CoreFoundation (not required for this crate).
 */
#include <stdio.h>
#include "openskp.h"

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <file.skp>\n", argv[0]);
        return 2;
    }
    openskp_model *m = openskp_open(argv[1]);
    if (!m) {
        fprintf(stderr, "failed to open %s\n", argv[1]);
        return 1;
    }
    printf("definitions:   %zu\n", openskp_definition_count(m));
    printf("geometry runs: %zu\n", openskp_geometry_run_count(m));
    printf("json:\n%s\n", openskp_model_json(m));
    openskp_free(m);
    return 0;
}
