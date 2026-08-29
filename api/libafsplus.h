#ifndef LIBAFSPLUS_H
#define LIBAFSPLUS_H

#include <stdint.h>
#include <stddef.h>

typedef struct afsp_volume afsp_volume;
typedef struct afsp_tx afsp_tx;
typedef struct afsp_object afsp_object;
typedef struct afsp_iter afsp_iter;

struct afsp_block_ops {
    void *ctx;
    int (*read)(void *ctx, uint64_t block, uint32_t count, void *dst);
    int (*write)(void *ctx, uint64_t block, uint32_t count, const void *src);
    int (*flush)(void *ctx);
    int (*discard)(void *ctx, uint64_t block, uint64_t count);
    uint64_t (*block_count)(void *ctx);
    uint32_t (*sector_size)(void *ctx);
};

enum afsp_open_mode {
    AFSP_OPEN_RW,
    AFSP_OPEN_READ_ONLY,
    AFSP_OPEN_NO_CHANGES,
    AFSP_OPEN_RECOVERY
};

int afsp_open(const struct afsp_block_ops *, enum afsp_open_mode, afsp_volume **out);
void afsp_close(afsp_volume *);

int afsp_lookup(afsp_volume *, uint64_t parent, const uint8_t *name, size_t len, uint64_t *object_id);
int afsp_read(afsp_volume *, uint64_t object_id, uint64_t offset, void *dst, size_t len, size_t *done);
int afsp_write(afsp_volume *, uint64_t object_id, uint64_t offset, const void *src, size_t len, size_t *done);

int afsp_tx_begin(afsp_volume *, afsp_tx **out);
int afsp_tx_commit(afsp_tx *);
void afsp_tx_abort(afsp_tx *);

int afsp_enumerate_objects(afsp_volume *, afsp_iter **out);
int afsp_changes_since(afsp_volume *, uint64_t sequence, afsp_iter **out);

#endif
