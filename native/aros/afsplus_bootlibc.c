/*
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * The C library functions the handler calls while it runs, on exec alone.
 *
 * Rust's standard library reaches malloc, calloc, realloc, free and
 * arc4random_buf through stdc.library. A boot volume's handler starts before
 * stdc.library can: stdc needs dos.library to initialise, and dos.library
 * waits for the boot volume. These definitions are linked ahead of the
 * library stubs, so the handler needs no C library to serve packets, and
 * afsplus_handler.c declares stdc, posixc and stdcio optional.
 */

/* Exec is always there; this file asks autoinit for nothing. Each inline
 * exec call would otherwise emit a requirement symbol that the handler's own
 * objects already define. */
#define AROS_LIBREQ(bname, ver)

#include <exec/execbase.h>
#include <exec/memory.h>
#include <proto/exec.h>

#include <stddef.h>
#include <stdint.h>
#include <string.h>

extern struct ExecBase *SysBase;

/* Every block carries its size in a 16-byte header, which keeps the address
 * handed out on the 16-byte alignment AllocMem gives on 64-bit AROS and that
 * the Rust heap relies on (heap.rs, MALLOC_ALIGN). */
#define HEADER 16

static void *allocate(size_t size, ULONG flags)
{
    size_t total;
    uint8_t *block;

    if (size > (size_t)-1 - HEADER)
        return NULL;
    total = size + HEADER;
    block = AllocMem(total, flags);
    if (block == NULL)
        return NULL;
    *(size_t *)block = total;
    return block + HEADER;
}

/*
 * The same memory for the Rust heap, without the header. Rust's GlobalAlloc
 * hands the layout to dealloc, so the size is known at both ends and exec
 * can be asked directly: no 16-byte header and no header write. The
 * library's allocations are mostly a dozen bytes each -- a create makes
 * some 1,310 of them, 710 of those of 8 to 15 bytes -- so the header was
 * costing more than the block it described.
 *
 * malloc below keeps its header, because the C shell frees without a size.
 */
void *afsplus_exec_alloc(size_t size, int clear)
{
    if (size == 0)
        return NULL;
    return AllocMem(size, clear ? (MEMF_ANY | MEMF_CLEAR) : MEMF_ANY);
}

void afsplus_exec_free(void *pointer, size_t size)
{
    if (pointer == NULL || size == 0)
        return;
    FreeMem(pointer, size);
}

void *malloc(size_t size)
{
    return allocate(size, MEMF_ANY);
}

void *calloc(size_t count, size_t size)
{
    if (size != 0 && count > (size_t)-1 / size)
        return NULL;
    return allocate(count * size, MEMF_ANY | MEMF_CLEAR);
}

void free(void *pointer)
{
    uint8_t *block;

    if (pointer == NULL)
        return;
    block = (uint8_t *)pointer - HEADER;
    FreeMem(block, *(size_t *)block);
}

void *realloc(void *pointer, size_t size)
{
    uint8_t *moved;
    size_t held;

    if (pointer == NULL)
        return malloc(size);
    if (size == 0)
    {
        free(pointer);
        return NULL;
    }
    held = *(size_t *)((uint8_t *)pointer - HEADER) - HEADER;
    if (size <= held && size >= held / 2)
        return pointer;
    moved = malloc(size);
    if (moved == NULL)
        return NULL;
    memcpy(moved, pointer, size < held ? size : held);
    free(pointer);
    return moved;
}

/*
 * Seeds for Rust's HashMap, which only needs keys that differ between runs
 * and processes, not secrecy: the handler's maps are keyed by values of its
 * own volume. A splitmix64 stream over a counter, the dispatch and idle
 * counts and the addresses in play.
 */
void arc4random_buf(void *buffer, size_t length)
{
    static uint64_t state;
    uint8_t *out = buffer;

    Forbid();
    state += (uint64_t)(uintptr_t)buffer ^ ((uint64_t)SysBase->DispCount << 32)
        ^ (uint64_t)SysBase->IdleCount ^ (uint64_t)(uintptr_t)&length;
    while (length > 0)
    {
        uint64_t word;
        size_t take;

        state += UINT64_C(0x9E3779B97F4A7C15);
        word = state;
        word = (word ^ (word >> 30)) * UINT64_C(0xBF58476D1CE4E5B9);
        word = (word ^ (word >> 27)) * UINT64_C(0x94D049BB133111EB);
        word ^= word >> 31;
        take = length < sizeof(word) ? length : sizeof(word);
        memcpy(out, &word, take);
        out += take;
        length -= take;
    }
    Permit();
}
