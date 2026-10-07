/*
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * The thread and lock primitives Rust's standard library refers to, answered
 * without pthread.library.
 *
 * The MacAROS std glues aros_thread_glue.c and aros_sync_glue.c implement
 * these over pthread.library, and linking pthread.library brings its fixed
 * thread table with it: one `threads` object of 1,841,328 bytes in .bss,
 * taken at load on every machine that has the handler in L/, whether or not
 * a volume is ever mounted. The handler creates no thread -- it serves its
 * packets on the one task DOS gives it -- so it pays for a table it never
 * reads a row of.
 *
 * What std really reaches is the eighteen entry points below: a Mutex, a
 * Condvar and thread spawn/join/yield/sleep. The Mutex is real, because std
 * takes it (stdout, the once-cells behind panic output) and a handler must
 * not deadlock on itself; it is an exec SignalSemaphore, which exec always
 * provides and which costs 56 bytes per lock instead of a megabyte at load.
 * Everything that only a second thread could make true -- a condition wait,
 * a spawn, a join -- says so in the debug log and fails; none of it may
 * return a quiet success, because a quiet success there is a handler that
 * waits forever or loses work with nothing written down.
 */

/* Exec is always there; asking autoinit for it would emit a requirement the
 * handler's own objects already define. */
#define AROS_LIBREQ(bname, ver)

#include <exec/execbase.h>
#include <exec/semaphores.h>
#include <proto/exec.h>

#include <aros/debug.h>

#include <stddef.h>
#include <stdint.h>

extern struct ExecBase *SysBase;

/* EPERM and EDEADLK as AROS numbers them (aros/stdc/errno.h). A lock that
 * cannot be taken and a thread that cannot be made are refusals, not
 * shortages: nothing frees up later. */
#define AFSPLUS_EPERM 1
#define AFSPLUS_EDEADLK 11
#define AFSPLUS_EINVAL 22

/*
 * A std Mutex is a zeroed buffer of 160 bytes that init() prepares and lock()
 * uses. The magic word is what tells a prepared lock from a zeroed one, so a
 * lock taken before its init is caught here rather than stepping through an
 * uninitialised list.
 */
#define MUTEX_MAGIC UINT32_C(0x4166534D) /* "AfSM" */
#define COND_MAGIC UINT32_C(0x41665343)  /* "AfSC" */

struct boot_mutex
{
    uint32_t magic;
    uint32_t pad;
    struct SignalSemaphore semaphore;
};

struct boot_cond
{
    uint32_t magic;
    uint32_t pad;
};

unsigned long aros_mtx_size(void)
{
    return (unsigned long)sizeof(struct boot_mutex);
}

int aros_mtx_init(void *raw)
{
    struct boot_mutex *mutex = raw;

    if (mutex == NULL)
        return AFSPLUS_EINVAL;
    InitSemaphore(&mutex->semaphore);
    mutex->magic = MUTEX_MAGIC;
    return 0;
}

static struct boot_mutex *prepared(void *raw)
{
    struct boot_mutex *mutex = raw;

    if (mutex != NULL && mutex->magic == MUTEX_MAGIC)
        return mutex;
    bug("afsplus: a std mutex was used before it was initialised\n");
    return NULL;
}

/*
 * std's Mutex is not reentrant: taking it twice from one task is a bug in the
 * caller, and on a real pthread NORMAL mutex it deadlocks. A SignalSemaphore
 * would instead nest and hand the same data to two borrows, so the nesting is
 * undone and the call fails. std turns a non-zero lock into a panic, which is
 * where such a bug belongs.
 */
int aros_mtx_lock(void *raw)
{
    struct boot_mutex *mutex = prepared(raw);

    if (mutex == NULL)
        return AFSPLUS_EINVAL;
    ObtainSemaphore(&mutex->semaphore);
    if (mutex->semaphore.ss_NestCount > 1)
    {
        bug("afsplus: a std mutex was locked twice from one task\n");
        ReleaseSemaphore(&mutex->semaphore);
        return AFSPLUS_EDEADLK;
    }
    return 0;
}

int aros_mtx_trylock(void *raw)
{
    struct boot_mutex *mutex = prepared(raw);

    if (mutex == NULL)
        return AFSPLUS_EINVAL;
    if (!AttemptSemaphore(&mutex->semaphore))
        return AFSPLUS_EDEADLK;
    if (mutex->semaphore.ss_NestCount > 1)
    {
        bug("afsplus: a std mutex was try-locked while this task held it\n");
        ReleaseSemaphore(&mutex->semaphore);
        return AFSPLUS_EDEADLK;
    }
    return 0;
}

int aros_mtx_unlock(void *raw)
{
    struct boot_mutex *mutex = prepared(raw);

    if (mutex == NULL)
        return AFSPLUS_EINVAL;
    ReleaseSemaphore(&mutex->semaphore);
    return 0;
}

/* Drop runs on a mutex that was never initialised, so a zeroed buffer here is
 * ordinary and says nothing. A held semaphore has no owner left to release
 * it, and that is worth a line. */
int aros_mtx_destroy(void *raw)
{
    struct boot_mutex *mutex = raw;

    if (mutex == NULL || mutex->magic != MUTEX_MAGIC)
        return 0;
    if (mutex->semaphore.ss_NestCount != 0)
        bug("afsplus: a std mutex was dropped while it was held\n");
    mutex->magic = 0;
    return 0;
}

unsigned long aros_cond_size(void)
{
    return (unsigned long)sizeof(struct boot_cond);
}

int aros_cond_init(void *raw)
{
    struct boot_cond *cond = raw;

    if (cond == NULL)
        return AFSPLUS_EINVAL;
    cond->magic = COND_MAGIC;
    return 0;
}

/*
 * Signalling a condition nobody waits on is what a signal does on any
 * implementation, so these succeed. Waiting is the other matter: the handler
 * has one task, so whatever would notify it cannot run until the wait
 * returns. A wait here never ends, and saying so is better than not ending.
 */
int aros_cond_signal(void *raw)
{
    (void)raw;
    return 0;
}

int aros_cond_broadcast(void *raw)
{
    (void)raw;
    return 0;
}

int aros_cond_wait(void *cond, void *mutex)
{
    (void)cond;
    (void)mutex;
    bug("afsplus: the handler waited on a condition variable; it has one "
        "task, so nothing can signal it\n");
    return AFSPLUS_EPERM;
}

int aros_cond_timedwait(void *cond, void *mutex, unsigned int secs,
    unsigned int nsecs)
{
    (void)cond;
    (void)mutex;
    (void)secs;
    (void)nsecs;
    bug("afsplus: the handler waited on a condition variable; it has one "
        "task, so nothing can signal it\n");
    return AFSPLUS_EPERM;
}

int aros_cond_destroy(void *raw)
{
    struct boot_cond *cond = raw;

    if (cond != NULL)
        cond->magic = 0;
    return 0;
}

/*
 * Threads. The handler serves its packets on the task DOS gave it and makes
 * no other; a spawn is a change nobody made on purpose. std turns the error
 * into an io::Error the caller sees, and the log line names the handler.
 */
int aros_thr_spawn(unsigned long stacksize, void *(*start)(void *), void *arg,
    unsigned int *out_tid)
{
    (void)stacksize;
    (void)start;
    (void)arg;
    (void)out_tid;
    bug("afsplus: the handler tried to spawn a thread; it links no thread "
        "library\n");
    return AFSPLUS_EPERM;
}

int aros_thr_join(unsigned int tid)
{
    (void)tid;
    bug("afsplus: the handler tried to join a thread it never spawned\n");
    return AFSPLUS_EPERM;
}

int aros_thr_detach(unsigned int tid)
{
    (void)tid;
    bug("afsplus: the handler tried to detach a thread it never spawned\n");
    return AFSPLUS_EPERM;
}

/* Yielding to the other threads of a task that has none is a no-op wherever
 * it is implemented, but reaching it means the handler is spinning on
 * something, so it is said once rather than every turn of the loop. */
void aros_thr_yield(void)
{
    static int said;

    if (!said)
    {
        said = 1;
        bug("afsplus: the handler yielded; it has one task to yield to\n");
    }
}

/* std ignores what sleep returns, so the log line is the whole of the
 * warning. The handler takes its time from its packets and must not stop the
 * task DOS is waiting on. */
int aros_thr_sleep(unsigned int secs, unsigned int nsecs)
{
    bug("afsplus: the handler slept for %u.%09u s on the task that serves "
        "its packets\n", secs, nsecs);
    return AFSPLUS_EPERM;
}

/*
 * The other door into pthread.library is emulated TLS. The aarch64 code
 * generator gives a `#[thread_local]` static an __emutls_get_address call,
 * and compiler-rt's emutls.c asks a thread library for one TLS key, one
 * mutex and one once-guard. collect-aros watches for exactly that -- it adds
 * -lpthread by itself when a pthread symbol is left undefined, saying
 * "(emulated-TLS dependency)" -- so removing -lpthread from the profile is
 * not enough on its own: these six have to be answered here too, or the
 * thread table comes back through the linker's own hand.
 *
 * Each task that touches a thread-local value gets its own set below, so a
 * second handler instance served from the same resident segment (the mounted
 * update image next to the installed volume) keeps its own values.
 */

#define KEY_SLOTS 16
#define KEY_OWNERS 4

/* One set of thread-local values per task that uses them: two handler
 * instances served from one resident segment (the installed volume and a
 * mounted update image) are two tasks with two sets. The table is claimed
 * under Forbid(); a fifth task is named in the log and gets nothing. */
static struct {
    struct Task *owner;
    void *values[KEY_SLOTS];
} key_tables[KEY_OWNERS];
static unsigned int keys_taken;

static void **key_values_of_this_task(void)
{
    struct Task *task = FindTask(NULL);
    int i;

    Forbid();
    for (i = 0; i < KEY_OWNERS; i++)
    {
        if (key_tables[i].owner == task)
        {
            Permit();
            return key_tables[i].values;
        }
    }
    for (i = 0; i < KEY_OWNERS; i++)
    {
        if (key_tables[i].owner == NULL)
        {
            key_tables[i].owner = task;
            Permit();
            return key_tables[i].values;
        }
    }
    Permit();
    bug("afsplus: more than %d tasks read the handler's thread-local values; "
        "the extra task gets none\n", KEY_OWNERS);
    return NULL;
}

int pthread_key_create(unsigned int *key, void (*destructor)(void *))
{
    /* Destructors run when a thread ends. The one task here ends with the
     * handler, and nothing that ends with it needs unwinding, so a
     * destructor is recorded as unserved rather than quietly dropped. */
    if (key == NULL)
        return AFSPLUS_EINVAL;
    if (destructor != NULL)
        bug("afsplus: a thread-local value asked for a destructor; the "
            "handler runs none\n");
    Forbid();
    if (keys_taken >= KEY_SLOTS)
    {
        Permit();
        bug("afsplus: the handler ran out of thread-local keys (%d)\n",
            KEY_SLOTS);
        return AFSPLUS_EPERM;
    }
    *key = keys_taken++;
    Permit();
    return 0;
}

int pthread_key_delete(unsigned int key)
{
    int i;

    if (key >= KEY_SLOTS)
        return AFSPLUS_EINVAL;
    for (i = 0; i < KEY_OWNERS; i++)
        key_tables[i].values[key] = NULL;
    return 0;
}

int pthread_setspecific(unsigned int key, const void *value)
{
    void **values;

    if (key >= KEY_SLOTS || (values = key_values_of_this_task()) == NULL)
        return AFSPLUS_EINVAL;
    values[key] = (void *)(uintptr_t)value;
    return 0;
}

void *pthread_getspecific(unsigned int key)
{
    void **values;

    if (key >= KEY_SLOTS || (values = key_values_of_this_task()) == NULL)
        return NULL;
    return values[key];
}

/*
 * emutls takes its mutex statically initialised, so there is no init call to
 * hang the semaphore on; the first lock makes it. Forbid() is what keeps two
 * tasks from making two semaphores out of one buffer -- exec cannot switch
 * inside it -- and the buffer is pthread_mutex_t's 136 bytes, which is more
 * than a struct boot_mutex needs.
 */
int pthread_mutex_lock(void *raw)
{
    struct boot_mutex *mutex = raw;

    if (mutex == NULL)
        return AFSPLUS_EINVAL;
    Forbid();
    if (mutex->magic != MUTEX_MAGIC)
    {
        InitSemaphore(&mutex->semaphore);
        mutex->magic = MUTEX_MAGIC;
    }
    Permit();
    ObtainSemaphore(&mutex->semaphore);
    return 0;
}

int pthread_mutex_unlock(void *raw)
{
    struct boot_mutex *mutex = raw;

    if (mutex == NULL || mutex->magic != MUTEX_MAGIC)
        return AFSPLUS_EINVAL;
    ReleaseSemaphore(&mutex->semaphore);
    return 0;
}

/* pthread_once_t on AROS is { volatile int done; int started; int lock; },
 * and PTHREAD_ONCE_INIT is { 0, -1, 0 }. One task cannot be interrupted
 * between the test and the call by another caller of the same guard, so
 * `done` is the whole of it. */
struct boot_once
{
    volatile int done;
    int started;
    int lock;
};

int pthread_once(void *raw, void (*routine)(void))
{
    struct boot_once *once = raw;

    if (once == NULL || routine == NULL)
        return AFSPLUS_EINVAL;
    if (once->done)
        return 0;
    once->done = 1;
    routine();
    return 0;
}
