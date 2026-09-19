/* SPDX-License-Identifier: BSD-2-Clause
 *
 * AFS+ filesystem probe: recognise an AFS+ volume from its first 4 KiB.
 *
 * For partition editors, `blkid`-style probers and installers that need to
 * say "this is an AFS+ volume, labelled X, with UUID Y" and nothing more.
 * One read of AFSPLUS_PROBE_BYTES at byte offset 0 of the partition, one
 * call, no allocation, no dependency beyond <stdint.h> and <string.h>. The
 * whole block is covered by a CRC32C, so a probe that says yes has seen a
 * block AFS+ wrote, not a coincidence of magic bytes.
 *
 * The layout is the identification block of the on-disk format
 * (spec/disk-layout.md, spec/afsplus_format.h); this file duplicates the few
 * constants it needs so that it can be dropped into another tree as it is.
 */
#ifndef AFSPLUS_PROBE_H
#define AFSPLUS_PROBE_H

#include <stddef.h>
#include <stdint.h>

/* Bytes to read at offset 0 of the device or partition. */
#define AFSPLUS_PROBE_BYTES 4096u

/* Results of afsplus_probe(). */
#define AFSPLUS_PROBE_OK 0            /* an AFS+ volume; *out is filled */
#define AFSPLUS_PROBE_NOT_AFSPLUS 1   /* no AFS+ identification here */
#define AFSPLUS_PROBE_DAMAGED 2       /* AFS+ magic, but the block does not check */
#define AFSPLUS_PROBE_UNSUPPORTED 3   /* AFS+, but a layout this probe does not read */
#define AFSPLUS_PROBE_SHORT 4         /* fewer than AFSPLUS_PROBE_BYTES given */

struct afsplus_probe_result {
    uint32_t epoch;             /* format epoch, 1 */
    uint32_t ident_version;     /* identification layout, 3 */
    uint8_t uuid[16];           /* the volume's identity, as stored */
    uint32_t block_size;        /* bytes per logical block, 4096 */
    uint64_t total_blocks;      /* size of the volume in blocks */
    uint64_t size_bytes;        /* block_size * total_blocks */
    uint64_t compat_features;   /* feature masks: an unknown incompat bit */
    uint64_t ro_compat_features;/* means a reader must not mount read-write */
    uint64_t incompat_features; /* (see spec/feature-registry.toml) */
    uint8_t label_len;          /* 0 to 64 bytes of UTF-8, no NUL inside */
    char label[65];             /* NUL-terminated copy */
};

/* Probes `block`, the first `len` bytes of a device (len >= AFSPLUS_PROBE_BYTES).
 * Returns one of the AFSPLUS_PROBE_* codes; `out` is written only on OK. */
int afsplus_probe(const uint8_t *block, size_t len, struct afsplus_probe_result *out);

/* Writes the UUID as 36 lowercase hex characters with dashes plus a NUL
 * into `text` (37 bytes). */
void afsplus_probe_uuid_text(const uint8_t uuid[16], char text[37]);

/* One line for a person, or the reason a probe said no. */
const char *afsplus_probe_status_text(int status);

#endif
