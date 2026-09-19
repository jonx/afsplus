/* SPDX-License-Identifier: BSD-2-Clause
 *
 * AFS+ filesystem probe. See afsplus_probe.h. Compile with
 * -DAFSPLUS_PROBE_MAIN for a command that probes a file or a device:
 *
 *     cc -std=c99 -DAFSPLUS_PROBE_MAIN -o afsplus-probe afsplus_probe.c
 *     afsplus-probe /dev/disk4s2          # one line, exit 0 when AFS+
 *     afsplus-probe --json image.afsp     # one JSON object
 */
#include "afsplus_probe.h"

#include <string.h>

/* The identification block: a 32-byte common header, then its payload. */
#define HEADER_SIZE 32u
#define CHECKSUM_OFFSET 28u
#define HEADER_VERSION 1u
#define BLOCK_TYPE_IDENT UINT32_C(0x49534641)          /* "AFSI" */
#define MAGIC UINT64_C(0x3153554c50534641)             /* "AFSPLUS1" */
#define EPOCH 1u
#define IDENT_VERSION 3u
#define IDENT_PAYLOAD 165u
#define LABEL_MAX 64u

static uint16_t le16(const uint8_t *p)
{
    return (uint16_t)(p[0] | ((uint16_t)p[1] << 8));
}

static uint32_t le32(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) |
           ((uint32_t)p[3] << 24);
}

static uint64_t le64(const uint8_t *p)
{
    return (uint64_t)le32(p) | ((uint64_t)le32(p + 4) << 32);
}

/* CRC32C (Castagnoli, reflected 0x82F63B78), bit by bit: a probe runs once. */
static uint32_t crc32c(uint32_t crc, const uint8_t *data, size_t len)
{
    size_t i;
    int bit;
    for (i = 0; i < len; ++i) {
        crc ^= data[i];
        for (bit = 0; bit < 8; ++bit)
            crc = (crc >> 1) ^ (0x82F63B78u & (0u - (crc & 1u)));
    }
    return crc;
}

/* The block's checksum covers every byte with the checksum field as zero. */
static uint32_t block_crc(const uint8_t *block, size_t len)
{
    static const uint8_t zero[4] = {0, 0, 0, 0};
    uint32_t crc = UINT32_MAX;
    crc = crc32c(crc, block, CHECKSUM_OFFSET);
    crc = crc32c(crc, zero, sizeof zero);
    crc = crc32c(crc, block + CHECKSUM_OFFSET + 4u, len - CHECKSUM_OFFSET - 4u);
    return ~crc;
}

static int valid_utf8(const uint8_t *s, size_t n)
{
    size_t i = 0;
    while (i < n) {
        uint8_t c = s[i];
        size_t more;
        uint32_t cp;
        if (c < 0x80u) { i++; continue; }
        if ((c & 0xE0u) == 0xC0u) { more = 1; cp = c & 0x1Fu; if (cp < 2u) return 0; }
        else if ((c & 0xF0u) == 0xE0u) { more = 2; cp = c & 0x0Fu; }
        else if ((c & 0xF8u) == 0xF0u) { more = 3; cp = c & 0x07u; }
        else return 0;
        if (n - i - 1u < more) return 0;
        {
            size_t k;
            for (k = 1; k <= more; ++k) {
                if ((s[i + k] & 0xC0u) != 0x80u) return 0;
                cp = (cp << 6) | (s[i + k] & 0x3Fu);
            }
        }
        if ((more == 2 && cp < 0x800u) || (more == 3 && cp < 0x10000u) ||
            cp > 0x10FFFFu || (cp >= 0xD800u && cp <= 0xDFFFu))
            return 0;
        i += more + 1u;
    }
    return 1;
}

int afsplus_probe(const uint8_t *block, size_t len, struct afsplus_probe_result *out)
{
    const uint8_t *p;
    uint32_t payload_len;
    size_t tail;
    uint32_t version;

    if (block == NULL || len < AFSPLUS_PROBE_BYTES) return AFSPLUS_PROBE_SHORT;
    len = AFSPLUS_PROBE_BYTES;
    p = block + HEADER_SIZE;

    /* Is it AFS+ at all? The block type and the magic both say so. A block
     * with neither is foreign; one with either is AFS+ that does not check,
     * which a person wants told apart from "not AFS+". */
    if (le32(block) != BLOCK_TYPE_IDENT && le64(p) != MAGIC)
        return AFSPLUS_PROBE_NOT_AFSPLUS;
    if (le32(block) != BLOCK_TYPE_IDENT || le64(p) != MAGIC)
        return AFSPLUS_PROBE_DAMAGED;

    /* A block AFS+ wrote checks as a whole; anything else is damage. */
    if (le32(block + CHECKSUM_OFFSET) != block_crc(block, len))
        return AFSPLUS_PROBE_DAMAGED;
    if (le16(block + 4) != HEADER_VERSION) return AFSPLUS_PROBE_UNSUPPORTED;
    if (le16(block + 6) != 0u || le64(block + 8) != 0u) return AFSPLUS_PROBE_DAMAGED;
    payload_len = le32(block + 24);
    if (payload_len > len - HEADER_SIZE) return AFSPLUS_PROBE_DAMAGED;
    for (tail = HEADER_SIZE + payload_len; tail < len; ++tail)
        if (block[tail] != 0u) return AFSPLUS_PROBE_DAMAGED;

    if (le32(p + 8) != EPOCH) return AFSPLUS_PROBE_UNSUPPORTED;
    version = le32(p + 12);
    if (version != IDENT_VERSION) return AFSPLUS_PROBE_UNSUPPORTED;
    if (payload_len != IDENT_PAYLOAD) return AFSPLUS_PROBE_DAMAGED;

    if (out == NULL) return AFSPLUS_PROBE_OK;
    memset(out, 0, sizeof *out);
    out->epoch = EPOCH;
    out->ident_version = version;
    memcpy(out->uuid, p + 16, 16);
    if (p[32] > 30u) return AFSPLUS_PROBE_DAMAGED;   /* block shift */
    out->block_size = (uint32_t)1u << p[32];
    out->total_blocks = le64(p + 40);
    out->size_bytes = out->total_blocks * out->block_size;
    out->label_len = p[72];
    if (out->label_len > LABEL_MAX || !valid_utf8(p + 73, out->label_len) ||
        memchr(p + 73, 0, out->label_len) != NULL)
        return AFSPLUS_PROBE_DAMAGED;
    memcpy(out->label, p + 73, out->label_len);
    out->label[out->label_len] = '\0';
    out->compat_features = le64(p + 137);
    out->ro_compat_features = le64(p + 145);
    out->incompat_features = le64(p + 153);
    return AFSPLUS_PROBE_OK;
}

void afsplus_probe_uuid_text(const uint8_t uuid[16], char text[37])
{
    static const char hex[] = "0123456789abcdef";
    int i, at = 0;
    for (i = 0; i < 16; ++i) {
        if (i == 4 || i == 6 || i == 8 || i == 10) text[at++] = '-';
        text[at++] = hex[uuid[i] >> 4];
        text[at++] = hex[uuid[i] & 15u];
    }
    text[at] = '\0';
}

const char *afsplus_probe_status_text(int status)
{
    switch (status) {
    case AFSPLUS_PROBE_OK: return "AFS+ volume";
    case AFSPLUS_PROBE_NOT_AFSPLUS: return "not an AFS+ volume";
    case AFSPLUS_PROBE_DAMAGED: return "AFS+ magic, but the identification block is damaged";
    case AFSPLUS_PROBE_UNSUPPORTED: return "AFS+ volume of a layout this probe does not read";
    case AFSPLUS_PROBE_SHORT: return "fewer than 4096 bytes given";
    default: return "unknown status";
    }
}

#ifdef AFSPLUS_PROBE_MAIN
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv)
{
    const char *path = NULL;
    int json = 0, i;
    uint8_t block[AFSPLUS_PROBE_BYTES];
    struct afsplus_probe_result r;
    char uuid[37];
    FILE *f;
    size_t got;
    int status;

    for (i = 1; i < argc; ++i) {
        if (strcmp(argv[i], "--json") == 0) json = 1;
        else if (path == NULL) path = argv[i];
        else { fprintf(stderr, "usage: afsplus-probe [--json] <device-or-image>\n"); return 2; }
    }
    if (path == NULL) { fprintf(stderr, "usage: afsplus-probe [--json] <device-or-image>\n"); return 2; }
    f = fopen(path, "rb");
    if (f == NULL) { fprintf(stderr, "afsplus-probe: cannot open %s\n", path); return 2; }
    got = fread(block, 1, sizeof block, f);
    fclose(f);
    status = afsplus_probe(block, got, &r);
    if (status != AFSPLUS_PROBE_OK) {
        if (json) printf("{\"schema\":\"afsplus-probe\",\"schema_version\":1,\"afsplus\":false,\"status\":%d,\"reason\":\"%s\"}\n",
                         status, afsplus_probe_status_text(status));
        else printf("%s: %s\n", path, afsplus_probe_status_text(status));
        return 1;
    }
    afsplus_probe_uuid_text(r.uuid, uuid);
    if (json) {
        printf("{\"schema\":\"afsplus-probe\",\"schema_version\":1,\"afsplus\":true,"
               "\"epoch\":%u,\"identification_version\":%u,\"uuid\":\"%s\",\"label\":\"",
               r.epoch, r.ident_version, uuid);
        for (i = 0; i < (int)r.label_len; ++i) {
            unsigned char c = (unsigned char)r.label[i];
            if (c == '"' || c == '\\') printf("\\%c", c);
            else if (c < 0x20u) printf("\\u%04x", c);
            else putchar(c);
        }
        printf("\",\"block_size\":%u,\"total_blocks\":%llu,\"size_bytes\":%llu,"
               "\"features\":{\"compat\":\"0x%016llx\",\"ro_compat\":\"0x%016llx\",\"incompat\":\"0x%016llx\"}}\n",
               r.block_size, (unsigned long long)r.total_blocks, (unsigned long long)r.size_bytes,
               (unsigned long long)r.compat_features, (unsigned long long)r.ro_compat_features,
               (unsigned long long)r.incompat_features);
    } else {
        printf("%s: AFS+ volume \"%s\", uuid %s, %llu blocks of %u bytes (%llu MiB), features compat 0x%llx ro 0x%llx incompat 0x%llx\n",
               path, r.label, uuid, (unsigned long long)r.total_blocks, r.block_size,
               (unsigned long long)(r.size_bytes >> 20), (unsigned long long)r.compat_features,
               (unsigned long long)r.ro_compat_features, (unsigned long long)r.incompat_features);
    }
    return 0;
}
#endif
