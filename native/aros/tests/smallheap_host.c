/* SPDX-License-Identifier: BSD-2-Clause
 * Host test of afsplus_smallheap.h: four threads allocate, fill, check and
 * free blocks of every size, and free one another's blocks, as mounts
 * sharing one handler do. Every block must hold what was written to it and
 * no two live blocks may overlap. Build with -fsanitize=address or thread. */
#include <assert.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>

static size_t system_blocks;
static void *system_alloc(size_t bytes)
{
    __atomic_add_fetch(&system_blocks, 1, __ATOMIC_RELAXED);
    return malloc(bytes);
}
#define SMALLHEAP_SYSTEM_ALLOC(bytes) system_alloc(bytes)
#include "afsplus_smallheap.h"

enum { THREADS = 4, HELD = 512, ROUNDS = 400000, SHARED = 64 };
struct held { unsigned char *block; size_t size; unsigned char mark; };
static struct held shared[SHARED]; /* blocks one thread allocates and another frees */
static pthread_mutex_t shared_lock = PTHREAD_MUTEX_INITIALIZER;

static void check_free(struct held *h)
{
    for (size_t i = 0; i < h->size; ++i) assert(h->block[i] == h->mark);
    smallheap_free(h->block, h->size);
    h->block = NULL;
}
static void *worker(void *argument)
{
    unsigned seed = (unsigned)(size_t)argument * 2654435761u + 1;
    struct held held[HELD] = {{0}};
    for (long round = 0; round < ROUNDS; ++round) {
        seed = seed * 1103515245u + 12345u;
        struct held *h = &held[(seed >> 8) % HELD];
        if (h->block) check_free(h);
        h->size = 1 + (seed >> 16) % SMALLHEAP_MAX;
        if ((seed >> 4) % 8) h->size = 1 + h->size % 96; /* mostly small, as the library's are */
        h->mark = (unsigned char)(seed >> 24);
        int clear = (seed >> 3) & 1;
        h->block = smallheap_alloc(h->size, clear);
        assert(h->block && ((size_t)h->block & 15) == 0);
        if (clear) for (size_t i = 0; i < h->size; ++i) assert(h->block[i] == 0);
        memset(h->block, h->mark, h->size);
        if ((seed >> 5) % 16 == 0) { /* hand it to whoever comes next */
            pthread_mutex_lock(&shared_lock);
            struct held other = shared[(seed >> 12) % SHARED];
            shared[(seed >> 12) % SHARED] = *h;
            pthread_mutex_unlock(&shared_lock);
            *h = other;
        }
    }
    for (int i = 0; i < HELD; ++i) if (held[i].block) check_free(&held[i]);
    return NULL;
}
int main(void)
{
    pthread_t threads[THREADS];
    for (size_t i = 0; i < THREADS; ++i) pthread_create(&threads[i], NULL, worker, (void *)(i + 1));
    for (int i = 0; i < THREADS; ++i) pthread_join(threads[i], NULL);
    for (int i = 0; i < SHARED; ++i) if (shared[i].block) check_free(&shared[i]);
    /* one thread alone never goes to the system for a block it has freed */
    size_t before = system_blocks;
    for (int i = 0; i < 100000; ++i) smallheap_free(smallheap_alloc(1 + (size_t)i % SMALLHEAP_MAX, 0), 1 + (size_t)i % SMALLHEAP_MAX);
    assert(system_blocks - before <= SMALLHEAP_MAX * 2 / SMALLHEAP_PIECE + 2);
    printf("smallheap: %zu pieces, %zu system blocks, PASS\n", smallheap_pieces, system_blocks);
    return 0;
}
