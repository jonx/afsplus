/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
/* Native host thread driver, kept separate from the AROS header translation
 * units so that libc/pthread and AROS ABI declarations never mix. */
#include <assert.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>

void claim_test_init(void);
void claim_test_phase(unsigned, unsigned);
void claim_test_finish(void);
static pthread_mutex_t claim_mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_mutex_t barrier_mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t barrier_cond = PTHREAD_COND_INITIALIZER;
static unsigned arrived, epoch;
static atomic_int allocations;
void host_claim_lock(void) { assert(pthread_mutex_lock(&claim_mutex) == 0); }
void host_claim_unlock(void) { assert(pthread_mutex_unlock(&claim_mutex) == 0); }
void *host_claim_alloc(size_t n)
{
    void *p = calloc(1, n);
    assert(p);
    atomic_fetch_add(&allocations, 1);
    return p;
}
void host_claim_free(void *p)
{
    atomic_fetch_sub(&allocations, 1);
    free(p);
}
static void barrier(void)
{
    assert(pthread_mutex_lock(&barrier_mutex) == 0);
    unsigned previous = epoch;
    if (++arrived == 8) {
        arrived = 0;
        epoch++;
        assert(pthread_cond_broadcast(&barrier_cond) == 0);
    } else {
        while (epoch == previous)
            assert(pthread_cond_wait(&barrier_cond, &barrier_mutex) == 0);
    }
    assert(pthread_mutex_unlock(&barrier_mutex) == 0);
}
void host_test_barrier(void) { barrier(); }
static void *worker(void *arg)
{
    unsigned index = (unsigned)(size_t)arg;
    for (unsigned round = 0; round < 1000; round++) {
        for (unsigned phase = 0; phase < 4; phase++) {
            claim_test_phase(index, phase);
            barrier();
        }
    }
    return NULL;
}
int main(void)
{
    pthread_t threads[8];
    claim_test_init();
    for (size_t i = 0; i < 8; i++)
        assert(pthread_create(&threads[i], NULL, worker, (void *)i) == 0);
    for (unsigned i = 0; i < 8; i++)
        assert(pthread_join(threads[i], NULL) == 0);
    claim_test_finish();
    assert(atomic_load(&allocations) == 0);
    puts("claim concurrent modules: PASS (8 threads, 1000 rounds)");
    return 0;
}
