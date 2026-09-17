/* SPDX-License-Identifier: BSD-2-Clause */
/* Cross-read probe for the snapshot record codecs (ADR-072). The value or
 * key is given as hexadecimal. Exit 0 means the C verdict equals the
 * expectation; 1 means it differs; 2 means the probe could not run.
 *
 *   snapshot_probe key <hex> reject | <id>
 *   snapshot_probe registry <hex> reject | <next-id>
 *   snapshot_probe record <hex> <max-generation> <total-blocks>
 *                         reject | <generation> <transaction> <object-map>
 *   snapshot_probe lifetime <hex> <start> <max-generation> <total-blocks>
 *                           reject | <blocks> <birth> <retirement>
 *   snapshot_probe ledger <hex> <total-blocks>
 *                         reject | <scan-position> <retained>
 */
#include "libafsplus_reader.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static uint64_t number(const char *text)
{
    return (uint64_t)strtoull(text, NULL, 0);
}

static int nibble(char c)
{
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    return -1;
}

/* "-" is the empty value. */
static int unhex(const char *text, uint8_t *out, size_t capacity, size_t *size)
{
    size_t length = strlen(text), i;
    if (strcmp(text, "-") == 0) {
        *size = 0u;
        return 1;
    }
    if (length % 2u != 0u || length / 2u > capacity) return 0;
    for (i = 0u; i < length / 2u; ++i) {
        int high = nibble(text[2u * i]), low = nibble(text[2u * i + 1u]);
        if (high < 0 || low < 0) return 0;
        out[i] = (uint8_t)(high * 16 + low);
    }
    *size = length / 2u;
    return 1;
}

int main(int argc, char **argv)
{
    uint8_t bytes[64];
    size_t size;
    const char *kind;
    int reject;

    if (argc < 4 || !unhex(argv[2], bytes, sizeof(bytes), &size)) return 2;
    kind = argv[1];
    reject = strcmp(argv[argc - 1], "reject") == 0;
    if (strcmp(kind, "key") == 0) {
        uint64_t id = 99u;
        int status = afspr_decode_snapshot_key(bytes, size, &id);
        if (reject) return status != AFSPR_OK && id == 99u ? 0 : 1;
        return status == AFSPR_OK && id == number(argv[3]) ? 0 : 1;
    }
    if (strcmp(kind, "registry") == 0) {
        uint64_t next = 99u;
        int status = afspr_decode_snapshot_registry_state(bytes, size, &next);
        if (reject) return status != AFSPR_OK && next == 99u ? 0 : 1;
        return status == AFSPR_OK && next == number(argv[3]) ? 0 : 1;
    }
    if (strcmp(kind, "record") == 0 && argc >= 6) {
        struct afspr_snapshot_record record;
        int status;
        memset(&record, 0xa5, sizeof(record));
        status = afspr_decode_snapshot_record(bytes, size, number(argv[3]),
                                              number(argv[4]), &record);
        if (reject) {
            return status != AFSPR_OK &&
                           record.generation == UINT64_C(0xa5a5a5a5a5a5a5a5)
                       ? 0
                       : 1;
        }
        if (argc != 8) return 2;
        return status == AFSPR_OK && record.generation == number(argv[5]) &&
                       record.committed_tx_id == number(argv[6]) &&
                       record.object_map_root == number(argv[7])
                   ? 0
                   : 1;
    }
    if (strcmp(kind, "lifetime") == 0 && argc >= 7) {
        struct afspr_snapshot_lifetime lifetime;
        int status;
        memset(&lifetime, 0xa5, sizeof(lifetime));
        status = afspr_decode_snapshot_lifetime(
            bytes, size, number(argv[3]), number(argv[4]), number(argv[5]),
            &lifetime);
        if (reject) {
            return status != AFSPR_OK &&
                           lifetime.blocks == UINT64_C(0xa5a5a5a5a5a5a5a5)
                       ? 0
                       : 1;
        }
        if (argc != 9) return 2;
        return status == AFSPR_OK && lifetime.blocks == number(argv[6]) &&
                       lifetime.birth == number(argv[7]) &&
                       lifetime.retirement == number(argv[8])
                   ? 0
                   : 1;
    }
    if (strcmp(kind, "ledger") == 0 && argc >= 5) {
        uint64_t position = 99u, retained = 98u;
        int status = afspr_decode_snapshot_ledger_state(
            bytes, size, number(argv[3]), &position, &retained);
        if (reject) {
            return status != AFSPR_OK && position == 99u && retained == 98u
                       ? 0
                       : 1;
        }
        if (argc != 6) return 2;
        return status == AFSPR_OK && position == number(argv[4]) &&
                       retained == number(argv[5])
                   ? 0
                   : 1;
    }
    return 2;
}
