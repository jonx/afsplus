/* SPDX-License-Identifier: BSD-2-Clause */

#include "harness.h"

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int parse_u64(const char *text, uint64_t *value)
{
    char *end = NULL;
    unsigned long long parsed;

    errno = 0;
    parsed = strtoull(text, &end, 0);
    if (errno != 0 || end == text || *end != '\0') {
        return -1;
    }
    *value = (uint64_t)parsed;
    return 0;
}

static int read_file(const char *path, uint8_t **bytes, size_t *size)
{
    FILE *file;
    long length;

    file = fopen(path, "rb");
    if (file == NULL || fseek(file, 0, SEEK_END) != 0) {
        return -1;
    }
    length = ftell(file);
    if (length < 0 || fseek(file, 0, SEEK_SET) != 0) {
        (void)fclose(file);
        return -1;
    }
    *size = (size_t)(unsigned long)length;
    *bytes = (uint8_t *)malloc(*size == 0u ? 1u : *size);
    if (*bytes == NULL || fread(*bytes, 1, *size, file) != *size) {
        free(*bytes);
        *bytes = NULL;
        (void)fclose(file);
        return -1;
    }
    if (fclose(file) != 0) {
        free(*bytes);
        *bytes = NULL;
        return -1;
    }
    return 0;
}

static uint64_t mix(uint64_t value)
{
    value ^= value >> 30;
    value *= UINT64_C(0xbf58476d1ce4e5b9);
    value ^= value >> 27;
    value *= UINT64_C(0x94d049bb133111eb);
    return value ^ (value >> 31);
}

static uint32_t load_u32(const uint8_t *bytes)
{
    return ((uint32_t)bytes[0]) | ((uint32_t)bytes[1] << 8) |
           ((uint32_t)bytes[2] << 16) | ((uint32_t)bytes[3] << 24);
}

static void store_u32(uint8_t *bytes, uint32_t value)
{
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8);
    bytes[2] = (uint8_t)(value >> 16);
    bytes[3] = (uint8_t)(value >> 24);
}

static uint32_t crc32c_update(uint32_t crc, const uint8_t *bytes, size_t size)
{
    size_t index;

    for (index = 0u; index < size; ++index) {
        unsigned int bit;

        crc ^= bytes[index];
        for (bit = 0u; bit < 8u; ++bit) {
            uint32_t mask = (uint32_t)(0u - (crc & 1u));
            crc = (crc >> 1) ^ (UINT32_C(0x82f63b78) & mask);
        }
    }
    return crc;
}

static void reseal_block(uint8_t *block)
{
    static const uint8_t zero[4] = {0u, 0u, 0u, 0u};
    uint32_t crc = UINT32_MAX;

    crc = crc32c_update(crc, block, 28u);
    crc = crc32c_update(crc, zero, sizeof(zero));
    crc = crc32c_update(crc, block + 32u,
                        AFSPR_MIN_SCRATCH_SIZE - 32u);
    store_u32(block + 28u, ~crc);
}

static size_t seed_record_count(const uint8_t *seed, size_t seed_size)
{
    size_t available;
    uint32_t declared;

    if (seed_size < AFSPR_FUZZ_PACKET_HEADER_SIZE) {
        return 0u;
    }
    available = (seed_size - AFSPR_FUZZ_PACKET_HEADER_SIZE) /
                AFSPR_FUZZ_PACKET_RECORD_SIZE;
    declared = load_u32(seed + 36u);
    return (size_t)declared < available ? (size_t)declared : available;
}

static void reseal_records(uint8_t *bytes, size_t size, size_t record_count)
{
    size_t record;

    for (record = 0u; record < record_count; ++record) {
        size_t position = AFSPR_FUZZ_PACKET_HEADER_SIZE +
                          (record * AFSPR_FUZZ_PACKET_RECORD_SIZE);

        if (position > size || AFSPR_FUZZ_PACKET_RECORD_SIZE > size - position) {
            break;
        }
        reseal_block(bytes + position + 8u);
    }
}

static size_t mutate(const uint8_t *seed, size_t seed_size, uint8_t *output,
                     size_t capacity, uint64_t case_number)
{
    uint64_t random = mix(case_number + UINT64_C(0x9e3779b97f4a7c15));
    size_t record_count = seed_record_count(seed, seed_size);
    uint64_t structured_cases = (uint64_t)record_count * UINT64_C(256);
    size_t size = seed_size;
    size_t index;
    unsigned int changes;
    unsigned int change;

    if (seed_size > capacity) {
        return 0u;
    }
    memcpy(output, seed, seed_size);
    if (case_number == 0u || seed_size == 0u) {
        return size;
    }
    if (case_number <= AFSPR_FUZZ_PACKET_HEADER_SIZE) {
        output[(size_t)(case_number - 1u)] ^= UINT8_C(0xff);
        return size;
    }

    if (case_number - AFSPR_FUZZ_PACKET_HEADER_SIZE <= structured_cases) {
        uint64_t structured = case_number -
                              AFSPR_FUZZ_PACKET_HEADER_SIZE - 1u;
        size_t record = (size_t)(structured / UINT64_C(256));
        size_t within_record = (size_t)(structured % UINT64_C(256));
        size_t block_position = AFSPR_FUZZ_PACKET_HEADER_SIZE +
                                (record * AFSPR_FUZZ_PACKET_RECORD_SIZE) + 8u;
        size_t byte = within_record / 2u;

        output[block_position + byte] ^= UINT8_C(0x80);
        if ((within_record & 1u) != 0u) {
            reseal_block(output + block_position);
        }
        return size;
    }

    switch ((unsigned int)(random & 3u)) {
    case 0u:
        index = (size_t)(mix(random) % seed_size);
        output[index] ^= (uint8_t)(1u << (unsigned int)((random >> 8) & 7u));
        break;
    case 1u:
        changes = 1u + (unsigned int)((random >> 12) & 3u);
        for (change = 0u; change < changes; ++change) {
            random = mix(random + change + 1u);
            index = (size_t)(random % seed_size);
            output[index] ^= (uint8_t)(random >> 24);
        }
        break;
    case 2u:
        size = (size_t)(mix(random) % (seed_size + 1u));
        break;
    default:
        index = (size_t)(mix(random) % seed_size);
        output[index] = (random & 4u) == 0u ? UINT8_C(0x00) : UINT8_C(0xff);
        break;
    }
    if ((case_number & 1u) != 0u) {
        reseal_records(output, size, record_count);
    }
    return size;
}

static int write_artifact(const char *path, const uint8_t *bytes, size_t size)
{
    FILE *file = fopen(path, "wb");
    int failed = 0;

    if (file == NULL) {
        return -1;
    }
    if (fwrite(bytes, 1, size, file) != size) {
        failed = 1;
    }
    if (fclose(file) != 0) {
        failed = 1;
    }
    return failed ? -1 : 0;
}

static int update_progress(const char *path, const char *seed,
                           uint64_t case_number)
{
    FILE *file;
    int failed = 0;

    if (path == NULL) {
        return 0;
    }
    file = fopen(path, "w");
    if (file == NULL) {
        return -1;
    }
    if (fprintf(file, "seed=%s\ncase=%llu\n", seed,
                (unsigned long long)case_number) < 0 ||
        fflush(file) != 0) {
        failed = 1;
    }
    if (fclose(file) != 0) {
        failed = 1;
    }
    return failed ? -1 : 0;
}

int main(int argc, char **argv)
{
    uint64_t runs = 4096u;
    uint64_t only_case = UINT64_MAX;
    const char *artifact = NULL;
    const char *progress = NULL;
    int first_seed = argc;
    int argument;
    int seed_index;

    for (argument = 1; argument < argc; ++argument) {
        if (strcmp(argv[argument], "--runs") == 0 && argument + 1 < argc) {
            if (parse_u64(argv[++argument], &runs) != 0 || runs == 0u) {
                fprintf(stderr, "invalid --runs value\n");
                return EXIT_FAILURE;
            }
        } else if (strcmp(argv[argument], "--case") == 0 &&
                   argument + 1 < argc) {
            if (parse_u64(argv[++argument], &only_case) != 0 ||
                only_case == UINT64_MAX) {
                fprintf(stderr, "invalid --case value\n");
                return EXIT_FAILURE;
            }
        } else if (strcmp(argv[argument], "--artifact") == 0 &&
                   argument + 1 < argc) {
            artifact = argv[++argument];
        } else if (strcmp(argv[argument], "--progress") == 0 &&
                   argument + 1 < argc) {
            progress = argv[++argument];
        } else if (argv[argument][0] == '-') {
            fprintf(stderr, "unknown option: %s\n", argv[argument]);
            return EXIT_FAILURE;
        } else {
            first_seed = argument;
            break;
        }
    }
    if (first_seed >= argc) {
        fprintf(stderr,
                "usage: %s [--runs N | --case N] [--artifact FILE] "
                "[--progress FILE] SEED...\n",
                argv[0]);
        return EXIT_FAILURE;
    }

    for (seed_index = first_seed; seed_index < argc; ++seed_index) {
        uint8_t *seed = NULL;
        uint8_t *mutated;
        size_t seed_size = 0u;
        size_t capacity;
        uint64_t first = only_case == UINT64_MAX ? 0u : only_case;
        uint64_t end = only_case == UINT64_MAX ? runs : only_case + 1u;
        uint64_t current;

        if (read_file(argv[seed_index], &seed, &seed_size) != 0 ||
            seed_size == SIZE_MAX) {
            fprintf(stderr, "cannot read seed: %s\n", argv[seed_index]);
            free(seed);
            return EXIT_FAILURE;
        }
        capacity = seed_size + 1u;
        mutated = (uint8_t *)malloc(capacity);
        if (mutated == NULL) {
            fprintf(stderr, "cannot allocate mutation buffer\n");
            free(seed);
            return EXIT_FAILURE;
        }
        for (current = first; current < end; ++current) {
            size_t mutated_size = mutate(seed, seed_size, mutated, capacity,
                                         current);
            if (update_progress(progress, argv[seed_index], current) != 0) {
                fprintf(stderr, "cannot update progress file\n");
                free(mutated);
                free(seed);
                return EXIT_FAILURE;
            }
            if (artifact != NULL && write_artifact(artifact, mutated,
                                                   mutated_size) != 0) {
                fprintf(stderr, "cannot write artifact\n");
                free(mutated);
                free(seed);
                return EXIT_FAILURE;
            }
            (void)LLVMFuzzerTestOneInput(mutated, mutated_size);
        }
        free(mutated);
        free(seed);
    }
    printf("portable-c-fuzz seeds=%d cases-per-seed=%llu result=PASS\n",
           argc - first_seed,
           (unsigned long long)(only_case == UINT64_MAX ? runs : 1u));
    return EXIT_SUCCESS;
}
