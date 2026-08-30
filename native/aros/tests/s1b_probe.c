/* SPDX-License-Identifier: BSD-2-Clause */

/* Verify the desktop assigns and a durable preference write after the pivot. */

#include <dos/dos.h>
#include <proto/dos.h>

#include <string.h>

static const UBYTE origin[] = "afsplus-s1b";

static int fail(const char *stage, SIPTR result)
{
    Printf("[AFSPLUS-S1B] FAIL %s result %ld error %ld\n",
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

static int require_file(const char *path)
{
    BPTR lock = Lock(path, SHARED_LOCK);

    if (lock == BNULL)
        return fail(path, DOSFALSE);
    UnLock(lock);
    return RETURN_OK;
}

int main(void)
{
    static const struct {
        const char *logical;
        const char *physical;
    } assigns[] = {
        { "ENVARC:", "AFSPLUS19:Prefs/Env-Archive" },
        { "LOCALE:", "AFSPLUS19:Locale" },
        { "FONTS:", "AFSPLUS19:Fonts" },
        { "WANDERER:", "AFSPLUS19:System/Wanderer" },
        { "THEMES:", "AFSPLUS19:Prefs/Presets/Themes" },
        { "THEME:", "AFSPLUS19:Prefs/Presets/Themes/AROSDefault" },
        { "IMAGES:", "AFSPLUS19:Prefs/Presets/Themes/AROSDefault/images" }
    };
    static const char *required_files[] = {
        "WANDERER:Wanderer",
        "SYS:Prefs/Locale",
        "SYS:Utilities/Clock",
        "THEME:system/Config"
    };
    UBYTE content[sizeof(origin)];
    BPTR file;
    LONG result;
    unsigned int index;

    for (index = 0; index < sizeof(assigns) / sizeof(assigns[0]); ++index)
    {
        result = require_same_lock(assigns[index].logical, assigns[index].physical);
        if (result != RETURN_OK)
            return result;
    }
    for (index = 0; index < sizeof(required_files) / sizeof(required_files[0]); ++index)
    {
        result = require_file(required_files[index]);
        if (result != RETURN_OK)
            return result;
    }

    file = Open("SYS:s1b-origin", MODE_OLDFILE);
    if (file == BNULL)
        return fail("open S1b origin", DOSFALSE);
    memset(content, 0, sizeof(content));
    result = Read(file, content, sizeof(content));
    if (result != (LONG)(sizeof(origin) - 1)
        || memcmp(content, origin, sizeof(origin) - 1) != 0)
    {
        Close(file);
        return fail("read S1b origin", result);
    }
    if (!Close(file))
        return fail("close S1b origin", DOSFALSE);

    file = Open("ENVARC:AFSPlus/S1b-probe", MODE_NEWFILE);
    if (file == BNULL)
        return fail("create S1b preference marker", DOSFALSE);
    result = Write(file, origin, sizeof(origin) - 1);
    if (result != (LONG)(sizeof(origin) - 1))
    {
        Close(file);
        return fail("write S1b preference marker", result);
    }
    if (!Flush(file))
    {
        Close(file);
        return fail("flush S1b preference marker", DOSFALSE);
    }
    if (!Close(file))
        return fail("close S1b preference marker", DOSFALSE);

    Printf("[AFSPLUS-S1B] PASS desktop assigns and preference write on AFSPLUS19\n");
    return RETURN_OK;
}
