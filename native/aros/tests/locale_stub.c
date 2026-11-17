/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
#include <assert.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>

typedef const char *CONST_STRPTR;
struct List { int unused; };
struct ExecBase { struct List LibList; };
struct Library { unsigned lib_Version; };
struct LocaleBase { struct Library library; };
struct Locale { int32_t loc_GMTOffset; };
static struct Library resource;
static struct LocaleBase service;
static struct Locale current;
static unsigned locked, present, opens, visits, smp, fail_open;
#define SPINLOCK_MODE_READ 1
#define LOCKF_FORBID 2
static struct Library *OpenResource(const char *name)
{ assert(!strcmp(name, "execlock.resource")); return smp ? &resource : NULL; }
static void Forbid(void) { assert(!locked); locked=1; }
static void Permit(void) { assert(locked); locked=0; }
static int ObtainSystemLock(struct List *list, int mode, int flags)
{ assert(list && mode==SPINLOCK_MODE_READ && flags==LOCKF_FORBID); Forbid(); return 1; }
static void ReleaseSystemLock(struct List *list, int flags)
{ assert(list && flags==LOCKF_FORBID); Permit(); }
static void *FindName(struct List *list, const char *name)
{ assert(list && locked && !strcmp(name,"locale.library")); return present ? &service : NULL; }
static void *OpenLibrary(const char *name, unsigned version)
{
    assert(!locked && present && !strcmp(name,"locale.library") && service.library.lib_Version>=version);
    opens++;
    return fail_open ? NULL : &service;
}
static struct Locale *OpenLocale(const char *name)
{ assert(!locked && !name); visits++; return &current; }
static void CloseLocale(struct Locale *locale) { assert(!locked && locale==&current); }
#include "afsplus_locale.h"

int main(void)
{
    struct ExecBase exec={0};
    for (smp=0; smp<=1; smp++) {
        struct LocaleBase *slot=NULL;
        present=opens=visits=0;
        assert(afsplus_aros_minutes_west(&exec,&slot)==0 && !opens && !slot);
        present=1;
        service.library.lib_Version=37;
        assert(afsplus_aros_minutes_west(&exec,&slot)==0 && !opens && !slot);
        service.library.lib_Version=51;
        fail_open=1;
        assert(afsplus_aros_minutes_west(&exec,&slot)==0 && opens==1 && !slot);
        fail_open=0;
        current.loc_GMTOffset=-60;
        assert(afsplus_aros_minutes_west(&exec,&slot)==-60 && slot==&service);
        current.loc_GMTOffset=330;
        assert(afsplus_aros_minutes_west(&exec,&slot)==330 && opens==2 && visits==2 && !locked);
    }
    return 0;
}
