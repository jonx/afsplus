/* SPDX-License-Identifier: BSD-2-Clause */

/* Bootstrap helper: atomically install AFS+ system assigns, then run S1. */

#include <dos/dos.h>
#include <dos/dostags.h>
#include <proto/dos.h>

static int fail(const char *stage, SIPTR result)
{
    Printf("[AFSPLUS-S1-PIVOT] FAIL %s result %ld error %ld\n",
        stage, result, IoErr());
    return RETURN_FAIL;
}

static int install_assign(const char *name, BPTR *lock)
{
    Printf("[AFSPLUS-S1-PIVOT] install %s\n", name);
    Flush(Output());
    if (!AssignLock(name, *lock))
        return fail(name, DOSFALSE);
    *lock = BNULL;
    Printf("[AFSPLUS-S1-PIVOT] assigned %s\n", name);
    Flush(Output());
    return RETURN_OK;
}

int main(void)
{
    static const struct {
        const char *name;
        const char *path;
    } system_assigns[] = {
        { "SYS", "AFSPLUS19:" },
        { "C", "AFSPLUS19:C" },
        { "L", "AFSPLUS19:L" },
        { "LIBS", "AFSPLUS19:Libs" },
        { "DEVS", "AFSPLUS19:Devs" },
        { "S", "AFSPLUS19:S" }
    };
    BPTR boot_lock = BNULL;
    BPTR system_locks[sizeof(system_assigns) / sizeof(system_assigns[0])] = { BNULL };
    LONG result;
    unsigned int index;

    Printf("[AFSPLUS-S1-PIVOT] acquire all source locks\n");
    Flush(Output());
    boot_lock = Lock("SYS:", SHARED_LOCK);
    if (boot_lock == BNULL)
        return fail("lock SYS", DOSFALSE);
    Printf("[AFSPLUS-S1-PIVOT] locked SYS\n");
    Flush(Output());
    for (index = 0; index < sizeof(system_assigns) / sizeof(system_assigns[0]); ++index)
    {
        Printf("[AFSPLUS-S1-PIVOT] lock %s\n", system_assigns[index].path);
        Flush(Output());
        system_locks[index] = Lock(system_assigns[index].path, SHARED_LOCK);
        if (system_locks[index] == BNULL)
        {
            result = fail(system_assigns[index].path, DOSFALSE);
            goto cleanup;
        }
        Printf("[AFSPLUS-S1-PIVOT] locked %s\n", system_assigns[index].path);
        Flush(Output());
    }
    Printf("[AFSPLUS-S1-PIVOT] all source locks acquired\n");
    Flush(Output());

    result = install_assign("BOOTSYS", &boot_lock);
    if (result != RETURN_OK)
        goto cleanup;
    for (index = 0; index < sizeof(system_assigns) / sizeof(system_assigns[0]); ++index)
    {
        result = install_assign(system_assigns[index].name, &system_locks[index]);
        if (result != RETURN_OK)
            goto cleanup;
    }
    Printf("[AFSPLUS-S1-PIVOT] assigns installed; executing AFS+ sequence\n");
    result = SystemTags("C:Execute S:S1-Sequence", SYS_Asynch, FALSE, TAG_DONE);
    if (result != RETURN_OK)
        return fail("execute S1 sequence", result);
    Printf("[AFSPLUS-S1-PIVOT] PASS\n");
    return RETURN_OK;

cleanup:
    if (boot_lock != BNULL)
        UnLock(boot_lock);
    for (index = 0; index < sizeof(system_locks) / sizeof(system_locks[0]); ++index)
    {
        if (system_locks[index] != BNULL)
            UnLock(system_locks[index]);
    }
    return result;
}
