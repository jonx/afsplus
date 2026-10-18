/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
/* Confined fresh file/tree restore, preserving every supported metadata field. */
#include <dos/dos.h>
#include <dos/dosextens.h>
#include <proto/dos.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include "../client/afsplus_client.h"

static int parse_id(const char *text, uint64_t *id)
{
    uint64_t value = 0;
    unsigned digit;
    if (*text == 0) return 0;
    while (*text)
    {
        if (*text < '0' || *text > '9') return 0;
        digit = (unsigned)(*text++ - '0');
        if (value > (UINT64_MAX - digit) / 10) return 0;
        value = value * 10 + digit;
    }
    *id = value;
    return value != 0;
}
int main(int argc, char **argv)
{
    struct DevProc *device;
    BPTR destination = BNULL;
    uint64_t id, reader = 0, object = 1, restored, parsed;
    struct AfsplusArosStat captured;
    uint32_t max_nodes = 4096, max_depth = 64, max_metadata_bytes = 4 * 1024 * 1024;
    LONG error = 0, close_error;
    int attempted = 0, published = 0;
    const char *phase = "destination";
    char *path, *component, *slash;
    if ((argc != 6 && argc != 9) || !parse_id(argv[2], &id) || !argv[3][0]
        || !argv[5][0] || strpbrk(argv[3], ":") != NULL
        || strpbrk(argv[5], ":/") != NULL
        || !strcmp(argv[5], ".") || !strcmp(argv[5], ".."))
    {
        Printf((CONST_STRPTR)"usage: AFSPlusRestore <volume> <snapshot-id> <snapshot-path> "
               "<target-directory> <new-name> [nodes depth metadata-bytes]\n"
               "Restores a file or directory on the same volume; / selects the captured root.\n"
               "Existing targets, links and security descriptors are refused.\n"
               "Tree defaults: 4096 nodes, 64 levels, 4194304 metadata bytes.\n");
        return RETURN_ERROR;
    }
    if (argc == 9)
    {
        if (!parse_id(argv[6], &parsed) || parsed > UINT32_MAX) return RETURN_ERROR;
        max_nodes = (uint32_t)parsed;
        if (!parse_id(argv[7], &parsed) || parsed > UINT32_MAX) return RETURN_ERROR;
        max_depth = (uint32_t)parsed;
        if (!parse_id(argv[8], &parsed) || parsed > UINT32_MAX) return RETURN_ERROR;
        max_metadata_bytes = (uint32_t)parsed;
    }
    device = GetDeviceProc((CONST_STRPTR)argv[1], NULL);
    if (device == NULL)
    {
        fprintf(stderr, "AFSPlusRestore: volume %s: DOS error %ld\n", argv[1], (long)IoErr());
        return RETURN_FAIL;
    }
    destination = Lock((CONST_STRPTR)argv[4], SHARED_LOCK);
    if (destination == BNULL) { error = (LONG)IoErr(); goto done; }
    if (afsplus_client_lock_port(destination) != device->dvp_Port)
    { error = ERROR_RENAME_ACROSS_DEVICES; goto done; }
    path = malloc(strlen(argv[3]) + 1);
    if (path == NULL) { error = ERROR_NO_FREE_STORE; goto done; }
    strcpy(path, argv[3]);
    phase = "snapshot-open";
    error = afsplus_client_snapshot_open(device->dvp_Port, id, &reader);
    component = path;
    if (*component == '/') component++;
    while (error == 0 && !(path[0] == '/' && *component == 0))
    {
        phase = "snapshot-path";
        slash = strchr(component, '/');
        if (slash != NULL) *slash = 0;
        if (!*component || !strcmp(component, ".") || !strcmp(component, ".."))
        { error = ERROR_INVALID_COMPONENT_NAME; break; }
        error = afsplus_client_snapshot_lookup(device->dvp_Port, reader, object,
            (CONST_STRPTR)component, &object);
        if (slash == NULL) break;
        component = slash + 1;
    }
    free(path);
    if (error == 0)
    {
        phase = "snapshot-stat";
        error = afsplus_client_snapshot_stat(device->dvp_Port, reader, object, &captured);
    }
    if (error == 0)
    {
        attempted = 1;
        phase = "restore";
        if (captured.kind == AFSPLUS_AROS_KIND_DIRECTORY)
            error = afsplus_client_snapshot_restore_tree(device->dvp_Port, reader,
                object, destination, (CONST_STRPTR)argv[5], max_nodes, max_depth,
                max_metadata_bytes, &restored);
        else if (captured.kind == AFSPLUS_AROS_KIND_FILE && argc == 6)
            error = afsplus_client_snapshot_restore_file(device->dvp_Port, reader,
                object, destination, (CONST_STRPTR)argv[5], &restored);
        else error = ERROR_OBJECT_WRONG_TYPE;
        if (error == 0) published = 1;
    }
    if (reader != 0)
    {
        close_error = afsplus_client_snapshot_close(device->dvp_Port, reader);
        if (error == 0 && close_error != 0) { phase = "snapshot-close"; error = close_error; }
    }
done:
    if (destination != BNULL) UnLock(destination);
    FreeDeviceProc(device);
    if (error != 0)
    {
        fprintf(stderr, "AFSPlusRestore: %s: DOS error %ld\n", phase, (long)error);
        if (published)
            fprintf(stderr, "The restored destination %s in %s was published; closing its snapshot reader failed.\n", argv[5], argv[4]);
        else if (attempted && (error == ERROR_NOT_A_DOS_DISK || error == ERROR_UNKNOWN))
        {
            char stage[80];
            snprintf(stage, sizeof stage, ".afsplus-restore-%016llx-%016llx-*",
                (unsigned long long)reader, (unsigned long long)object);
            fprintf(stderr, "Publication or cleanup is uncertain. Inspect destination %s and staged files matching %s in %s before retry.\n",
                argv[5], stage, argv[4]);
        }
    }
    else Printf((CONST_STRPTR)"AFSPlusRestore: restored %s in %s\n", argv[5], argv[4]);
    return error == 0 ? RETURN_OK : RETURN_FAIL;
}
