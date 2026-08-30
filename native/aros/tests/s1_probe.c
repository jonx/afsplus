/* SPDX-License-Identifier: BSD-2-Clause */

/* Verify that the live system assigns have pivoted onto the AFS+ volume. */

#include <dos/dos.h>
#include <proto/dos.h>

#include <string.h>

static const UBYTE origin[] = "afsplus-s1";

static int fail(const char *stage, SIPTR result)
{
    Printf("[AFSPLUS-S1] FAIL %s result %ld error %ld\n",
        stage, result, IoErr());
    return RETURN_FAIL;
}

static int require_same_lock(const char *logical, const char *physical)
{
    BPTR logical_lock = Lock(logical, SHARED_LOCK);
    BPTR physical_lock;
    LONG relation;

    if (logical_lock == BNULL)
        return fail(logical, DOSFALSE);
    physical_lock = Lock(physical, SHARED_LOCK);
    if (physical_lock == BNULL)
    {
        UnLock(logical_lock);
        return fail(physical, DOSFALSE);
    }
    relation = SameLock(logical_lock, physical_lock);
    UnLock(physical_lock);
    UnLock(logical_lock);
    if (relation != LOCK_SAME)
        return fail(logical, relation);
    return RETURN_OK;
}

int main(void)
{
    static const struct {
        const char *logical;
        const char *physical;
    } assigns[] = {
        { "SYS:", "AFSPLUS19:" },
        { "C:", "AFSPLUS19:C" },
        { "L:", "AFSPLUS19:L" },
        { "LIBS:", "AFSPLUS19:Libs" },
        { "DEVS:", "AFSPLUS19:Devs" },
        { "S:", "AFSPLUS19:S" }
    };
    UBYTE content[sizeof(origin)];
    BPTR file;
    BPTR afs_root;
    BPTR bootstrap_root;
    LONG result;
    unsigned int index;

    for (index = 0; index < sizeof(assigns) / sizeof(assigns[0]); ++index)
    {
        result = require_same_lock(assigns[index].logical, assigns[index].physical);
        if (result != RETURN_OK)
            return result;
    }

    bootstrap_root = Lock("BOOTSYS:", SHARED_LOCK);
    if (bootstrap_root == BNULL)
        return fail("recorded BOOTSYS bootstrap assign", DOSFALSE);
    afs_root = Lock("AFSPLUS19:", SHARED_LOCK);
    if (afs_root == BNULL)
    {
        UnLock(bootstrap_root);
        return fail("lock AFS+ root", DOSFALSE);
    }
    result = SameLock(bootstrap_root, afs_root);
    UnLock(afs_root);
    UnLock(bootstrap_root);
    if (result != LOCK_DIFFERENT)
        return fail("BOOTSYS must remain the distinct bootstrap volume", result);

    file = Open("SYS:s1-origin", MODE_OLDFILE);
    if (file == BNULL)
        return fail("open origin", DOSFALSE);
    memset(content, 0, sizeof(content));
    result = Read(file, content, sizeof(content));
    if (result != (LONG)(sizeof(origin) - 1)
        || memcmp(content, origin, sizeof(origin) - 1) != 0)
    {
        Close(file);
        return fail("read origin", result);
    }
    if (!Close(file))
        return fail("close origin", DOSFALSE);

    file = Open("SYS:s1-runtime", MODE_NEWFILE);
    if (file == BNULL)
        return fail("create runtime marker", DOSFALSE);
    result = Write(file, origin, sizeof(origin) - 1);
    if (result != (LONG)(sizeof(origin) - 1))
    {
        Close(file);
        return fail("write runtime marker", result);
    }
    if (!Flush(file))
    {
        Close(file);
        return fail("flush runtime marker", DOSFALSE);
    }
    if (!Close(file))
        return fail("close runtime marker", DOSFALSE);

    Printf("[AFSPLUS-S1] PASS SYS/C/L/LIBS/DEVS/S on AFSPLUS19\n");
    return RETURN_OK;
}
