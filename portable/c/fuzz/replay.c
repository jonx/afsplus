/* SPDX-License-Identifier: BSD-2-Clause */

#include "harness.h"

#include <stdio.h>
#include <stdlib.h>

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

static int invoked_operations_succeeded(const struct afspr_fuzz_outcome *result)
{
    return result->packet_status == AFSPR_OK &&
           result->probe_status != AFSPR_NOT_CHECKED &&
           result->probe_status == AFSPR_OK &&
           (result->object_status == AFSPR_NOT_CHECKED ||
            result->object_status == AFSPR_OK) &&
           (result->directory_status == AFSPR_NOT_CHECKED ||
            result->directory_status == AFSPR_OK) &&
           (result->read_status == AFSPR_NOT_CHECKED ||
            result->read_status == AFSPR_OK);
}

int main(int argc, char **argv)
{
    const char *path;
    int require_success = 0;
    uint8_t *bytes = NULL;
    size_t size = 0u;
    struct afspr_fuzz_outcome result;
    int status;

    if (argc == 3 && argv[1][0] == '-' && argv[1][1] == 's' &&
        argv[1][2] == '\0') {
        require_success = 1;
        path = argv[2];
    } else if (argc == 2) {
        path = argv[1];
    } else {
        fprintf(stderr, "usage: %s [-s] PACKET\n", argv[0]);
        return EXIT_FAILURE;
    }
    if (read_file(path, &bytes, &size) != 0) {
        fprintf(stderr, "cannot read packet: %s\n", path);
        return EXIT_FAILURE;
    }
    status = afspr_fuzz_run_input(bytes, size, &result);
    printf("packet=%s bytes=%lu harness=%s probe=%s object=%s directory=%s "
           "read=%s stage=%s block=",
           path, (unsigned long)size, afspr_status_string(status),
           afspr_status_string(result.probe_status),
           afspr_status_string(result.object_status),
           afspr_status_string(result.directory_status),
           afspr_status_string(result.read_status),
           afspr_probe_stage_string(result.diagnostic.stage));
    if (result.diagnostic.block == AFSPR_NO_BLOCK) {
        printf("n/a\n");
    } else {
        printf("%llu\n", (unsigned long long)result.diagnostic.block);
    }
    free(bytes);
    if (require_success && !invoked_operations_succeeded(&result)) {
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
