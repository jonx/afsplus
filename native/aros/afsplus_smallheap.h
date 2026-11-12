/*
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * Small blocks for the Rust heap, without a system call per block.
 *
 * The library's allocations are small and many: reading the information of
 * one file makes about thirty, finding a file eight directories deep a few
 * hundred. exec's AllocMem and FreeMem take the system's memory lock for
 * each; measured on a two-CPU AROS, a pair costs half a microsecond, and
 * that was most of the time of a packet.
 *
 * Blocks up to SMALLHEAP_MAX bytes are cut here from 64 KiB pieces of
 * system memory and kept, when freed, on one list per size, a size being a
 * multiple of 16 bytes. Rust's allocator gives the size back with the block
 * (GlobalAlloc), so a block carries nothing.
 *
 * Several mounts run on one copy of the handler, each on its own task, so
 * the lists are shared. A task enters them with one atomic test-and-set and
 * never waits: finding another task inside, it takes its block from the
 * system instead, or leaves the block it frees on a hand-over stack that
 * the next task inside moves to the lists. No task can therefore hold up
 * another, whatever their priorities.
 *
 * What it costs: memory freed here goes back to the lists, not to the
 * system. The handler keeps the most its small blocks ever held at once
 * for as long as it stays loaded; a later mount uses it again.
 *
 * The including file defines SMALLHEAP_SYSTEM_ALLOC(bytes), which must give
 * 16-byte alignment.
 */
#ifndef AFSPLUS_SMALLHEAP_H
#define AFSPLUS_SMALLHEAP_H

#include <stddef.h>
#include <stdint.h>
#include <string.h>

#define SMALLHEAP_GRAIN ((size_t)16)
#define SMALLHEAP_MAX ((size_t)2048)
#define SMALLHEAP_CLASSES (SMALLHEAP_MAX / SMALLHEAP_GRAIN)
#define SMALLHEAP_PIECE ((size_t)64 * 1024)

struct smallheap_block
{
    struct smallheap_block *next;
    size_t index; /* which list, for the hand-over stack */
};

static struct smallheap_block *smallheap_lists[SMALLHEAP_CLASSES];
static uint8_t *smallheap_piece, *smallheap_piece_end;
static char smallheap_inside;
static struct smallheap_block *smallheap_handed;
static size_t smallheap_pieces;

static int smallheap_enter(void)
{
    return !__atomic_test_and_set(&smallheap_inside, __ATOMIC_ACQUIRE);
}

static void smallheap_keep(void *pointer, size_t index)
{
    struct smallheap_block *block = pointer;

    block->index = index;
    block->next = smallheap_lists[index];
    smallheap_lists[index] = block;
}

/* Inside: take what other tasks handed over while they could not enter. */
static void smallheap_collect(void)
{
    struct smallheap_block *block;

    if (__atomic_load_n(&smallheap_handed, __ATOMIC_RELAXED) == NULL)
        return;
    block = __atomic_exchange_n(&smallheap_handed, NULL, __ATOMIC_ACQUIRE);
    while (block != NULL)
    {
        struct smallheap_block *next = block->next;

        smallheap_keep(block, block->index);
        block = next;
    }
}

static void smallheap_leave(void)
{
    smallheap_collect();
    __atomic_clear(&smallheap_inside, __ATOMIC_RELEASE);
}

/* size is 1..SMALLHEAP_MAX. Null when the system has no memory. */
static void *smallheap_alloc(size_t size, int clear)
{
    size_t index = (size - 1) / SMALLHEAP_GRAIN;
    size_t bytes = (index + 1) * SMALLHEAP_GRAIN;
    struct smallheap_block *block;

    if (!smallheap_enter())
    {
        /* A block of the list's size: it can join the list when freed. */
        block = SMALLHEAP_SYSTEM_ALLOC(bytes);
        if (block != NULL && clear)
            memset(block, 0, size);
        return block;
    }
    smallheap_collect();
    block = smallheap_lists[index];
    if (block != NULL)
        smallheap_lists[index] = block->next;
    else
    {
        if ((size_t)(smallheap_piece_end - smallheap_piece) < bytes)
        {
            uint8_t *piece;

            /* What is left of the piece becomes blocks of the sizes it holds. */
            while ((size_t)(smallheap_piece_end - smallheap_piece)
                >= SMALLHEAP_GRAIN)
            {
                size_t left = (size_t)(smallheap_piece_end - smallheap_piece);
                size_t take = left > SMALLHEAP_MAX ? SMALLHEAP_MAX
                    : left / SMALLHEAP_GRAIN * SMALLHEAP_GRAIN;

                smallheap_keep(smallheap_piece, take / SMALLHEAP_GRAIN - 1);
                smallheap_piece += take;
            }
            piece = SMALLHEAP_SYSTEM_ALLOC(SMALLHEAP_PIECE);
            if (piece == NULL)
            {
                smallheap_leave();
                block = SMALLHEAP_SYSTEM_ALLOC(bytes);
                if (block != NULL && clear)
                    memset(block, 0, size);
                return block;
            }
            smallheap_piece = piece;
            smallheap_piece_end = piece + SMALLHEAP_PIECE;
            smallheap_pieces++;
        }
        block = (struct smallheap_block *)smallheap_piece;
        smallheap_piece += bytes;
    }
    smallheap_leave();
    if (clear)
        memset(block, 0, size);
    return block;
}

/* size is the size the block was allocated with. */
static void smallheap_free(void *pointer, size_t size)
{
    size_t index = (size - 1) / SMALLHEAP_GRAIN;
    struct smallheap_block *block = pointer;

    if (smallheap_enter())
    {
        smallheap_keep(block, index);
        smallheap_leave();
        return;
    }
    block->index = index;
    block->next = __atomic_load_n(&smallheap_handed, __ATOMIC_RELAXED);
    while (!__atomic_compare_exchange_n(&smallheap_handed, &block->next,
        block, 1, __ATOMIC_RELEASE, __ATOMIC_RELAXED))
        ;
}

#endif
