/* SPDX-License-Identifier: BSD-2-Clause
 * A hosted crash workload using ordinary DOS calls. A host kills the isolated
 * AROS process during stress, then reboots the SAME image and runs verify.
 * Only an ACTION_FLUSH acknowledgement promises a generation's durability.
 */
#include <dos/dos.h>
#include <dos/dosextens.h>
#include <proto/dos.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define BYTES 16384
#define VOLUME "AFSCUT:"
static unsigned char bytes[BYTES];

static void pattern(uint32_t generation)
{
    unsigned i;
    for (i = 0; i < BYTES; ++i)
        bytes[i] = (unsigned char)((generation >> (8 * (i % 4))) & 255);
}

static int sync_volume(void)
{
    struct MsgPort *port = DeviceProc(VOLUME);
    return port != NULL && DoPkt(port, ACTION_FLUSH, 0, 0, 0, 0, 0) == DOSTRUE;
}

static int create(const char *name, uint32_t generation)
{
    BPTR file = Open(name, MODE_NEWFILE);
    LONG wrote;
    if (!file) return 0;
    pattern(generation);
    wrote = Write(file, bytes, BYTES);
    return Close(file) && wrote == BYTES && sync_volume();
}

static int read_generation(const char *name, uint32_t *generation)
{
    BPTR file = Open(name, MODE_OLDFILE);
    LONG count;
    unsigned i;
    unsigned char tail;
    if (!file) return 0;
    count = Read(file, bytes, BYTES);
    if (count != BYTES || Read(file, &tail, 1) != 0) {
        Close(file);
        return 0;
    }
    if (!Close(file)) return 0;
    *generation = (uint32_t)bytes[0] | (uint32_t)bytes[1] << 8
        | (uint32_t)bytes[2] << 16 | (uint32_t)bytes[3] << 24;
    for (i = 0; i < BYTES; ++i)
        if (bytes[i] != (unsigned char)((*generation >> (8 * (i % 4))) & 255))
            return 0;
    return 1;
}

int main(int argc, char **argv)
{
    uint32_t generation, kept, minimum;
    if (argc == 2 && strcmp(argv[1], "prepare") == 0) {
        if (!create(VOLUME "kept", UINT32_C(0x4b455054))
            || !create(VOLUME "target", 0)) goto failed;
        Printf("[AFSPLUS-KILL] PREPARED\n");
    } else if (argc == 2 && strcmp(argv[1], "stress") == 0) {
        BPTR file = Open(VOLUME "target", MODE_READWRITE);
        if (!file) goto failed;
        for (generation = 1; generation <= 100000; ++generation) {
            Printf("[AFSPLUS-KILL] WRITING %lu\n", (unsigned long)generation);
            if (!Flush(Output())) { Close(file); goto failed; }
            pattern(generation);
            if (Seek(file, 0, OFFSET_BEGINNING) == -1
                || Write(file, bytes, BYTES) != BYTES || !sync_volume()) {
                Close(file);
                goto failed;
            }
            Printf("[AFSPLUS-KILL] ACK %lu\n", (unsigned long)generation);
            if (!Flush(Output())) { Close(file); goto failed; }
        }
        Close(file);
        Printf("[AFSPLUS-KILL] EXHAUSTED\n");
        return RETURN_FAIL; /* The host was meant to kill the running workload. */
    } else if (argc == 3 && strcmp(argv[1], "verify") == 0) {
        minimum = (uint32_t)strtoul(argv[2], NULL, 10);
        if (!read_generation(VOLUME "kept", &kept)
            || kept != UINT32_C(0x4b455054)
            || !read_generation(VOLUME "target", &generation)
            || generation < minimum || generation > 100000
            || !create(VOLUME "after", UINT32_C(0x41465452))) goto failed;
        Printf("[AFSPLUS-KILL] VERIFIED generation=%lu acknowledged=%lu\n",
            (unsigned long)generation, (unsigned long)minimum);
    } else if (argc == 2 && strcmp(argv[1], "verify-after") == 0) {
        if (!read_generation(VOLUME "after", &generation)
            || generation != UINT32_C(0x41465452)) goto failed;
        Printf("[AFSPLUS-KILL] AFTER-PERSISTED\n");
    } else goto failed;
    Flush(Output());
    return RETURN_OK;
failed:
    Printf("[AFSPLUS-KILL] FAIL ioerr=%ld\n", IoErr());
    Flush(Output());
    return RETURN_FAIL;
}
