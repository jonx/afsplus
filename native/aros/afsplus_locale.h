/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
#ifndef AFSPLUS_AROS_LOCALE_H
#define AFSPLUS_AROS_LOCALE_H

/* A boot handler can serve packets before locale.library is installed.
 * Never ask the disk loader for it from inside the filesystem. AROS keeps
 * the installed locale.library resident. Each mount keeps its own open. */
static int32_t afsplus_aros_minutes_west(struct ExecBase *SysBase,
    struct LocaleBase **slot)
{
    struct LocaleBase *LocaleBase = *slot;
    struct Locale *locale;
    int32_t offset;

    if (LocaleBase == NULL)
    {
        CONST_STRPTR name = (CONST_STRPTR)"locale.library";
        struct Library *ExecLockBase = OpenResource((CONST_STRPTR)"execlock.resource");
        struct Library *library;
        int available;
        if (ExecLockBase != NULL)
        {
            if (!ObtainSystemLock(&SysBase->LibList, SPINLOCK_MODE_READ, LOCKF_FORBID))
                return 0;
        }
        else
        {
#if defined(__AROSEXEC_SMP__)
            return 0;
#else
            Forbid();
#endif
        }
        library = (struct Library *)FindName(&SysBase->LibList, name);
        available = library != NULL && library->lib_Version >= 38;
        if (ExecLockBase != NULL)
            ReleaseSystemLock(&SysBase->LibList, LOCKF_FORBID);
        else
            Permit();
        if (!available)
            return 0;
        *slot = LocaleBase = (struct LocaleBase *)OpenLibrary(name, 38);
        if (LocaleBase == NULL)
            return 0;
    }
    locale = OpenLocale(NULL);
    if (locale == NULL)
        return 0;
    offset = locale->loc_GMTOffset;
    CloseLocale(locale);
    return offset;
}

#endif
