/* SPDX-License-Identifier: BSD-2-Clause */
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
typedef uint32_t ULONG;
typedef const char *CONST_STRPTR;
struct Library { uint16_t lib_OpenCnt; };
struct SecurityBase { struct Library library; };
struct List { unsigned unused; };
struct ExecBase { struct List LibList; };
struct Task { ULONG owner; };
struct MsgPort { unsigned mp_Flags; struct Task *mp_SigTask; };
struct DosPacket { struct MsgPort *dp_Port; };
#define PF_ACTION 3
#define PA_SIGNAL 0
#define ERROR_READ_PROTECTED 224
#define SECURITYNAME "security.library"
#define secROOT_UID 0
#define SPINLOCK_MODE_WRITE 1
#define LOCKF_FORBID 2
static unsigned smp;
static struct Library lock_resource;
static struct Library *OpenResource(const char *name)
{ assert(!strcmp(name,"execlock.resource")); return smp ? &lock_resource : NULL; }
static unsigned forbidden, queries;
static struct SecurityBase security;
static struct Library *loaded;
static void Forbid(void) { assert(!forbidden); forbidden=1; }
static void Permit(void) { assert(forbidden); forbidden=0; }
static void ObtainSystemLock(struct List *list, unsigned mode, unsigned flags)
{ (void)list; assert(mode==SPINLOCK_MODE_WRITE && flags==LOCKF_FORBID); Forbid(); }
static void ReleaseSystemLock(struct List *list, unsigned flags)
{ (void)list; assert(flags==LOCKF_FORBID); Permit(); }
static void *FindName(struct List *list, CONST_STRPTR name)
{ (void)list; assert(forbidden); assert(!strcmp(name,SECURITYNAME)); return loaded; }
static ULONG owner_query(struct SecurityBase *base, struct Task *task)
{
    assert(!forbidden); /* The real query can wait for TaskOwnerSem. */
    assert(base == &security && base->library.lib_OpenCnt == 8);
    queries++;
    return task->owner;
}
#define afsplus_security_task_owner(base, task) owner_query((struct SecurityBase *)(base), task)
static unsigned opens, closes, open_fail;
static struct Library *OpenLibrary(CONST_STRPTR name, unsigned version)
{
    assert(!forbidden && !strcmp(name,SECURITYNAME) && version==40);
    /* An absent loaded node would enter lddemon and deadlock SYS. */
    assert(loaded && loaded->lib_OpenCnt==8);
    opens++;
    if (open_fail) return NULL;
    loaded->lib_OpenCnt++;
    return loaded;
}
static void CloseLibrary(struct Library *library)
{ assert(!forbidden && library==loaded && library->lib_OpenCnt==8); closes++; library->lib_OpenCnt--; }
#include "afsplus_authority.h"
int main(void)
{
    struct ExecBase exec = {0};
    struct Task sender = {0};
    struct MsgPort port = {PA_SIGNAL,&sender};
    struct DosPacket packet = {&port};
    security.library.lib_OpenCnt=7;
    for (unsigned policy=0; policy<=3; ++policy) {
        assert(afsplus_aros_authorize_policy(&exec,&packet,policy)
            == (policy==2 ? 0 : ERROR_READ_PROTECTED));
    }
    assert(queries==0 && !forbidden);
    loaded=&security.library;
    smp=1;
    for (unsigned policy=1; policy<=2; ++policy) {
        sender.owner=0;
        assert(afsplus_aros_authorize_policy(&exec,&packet,policy)==0);
        assert(security.library.lib_OpenCnt==7);
        sender.owner=1u<<16; /* Same sender, authority revoked between packets. */
        assert(afsplus_aros_authorize_policy(&exec,&packet,policy)==ERROR_READ_PROTECTED);
        assert(security.library.lib_OpenCnt==7);
    }
    assert(queries==4 && opens==4 && closes==4);
    open_fail=1;
    assert(afsplus_aros_authorize_policy(&exec,&packet,2)==ERROR_READ_PROTECTED);
    assert(security.library.lib_OpenCnt==7 && queries==4 && opens==5 && closes==4);
    open_fail=0;
    security.library.lib_OpenCnt=UINT16_MAX;
    assert(afsplus_aros_authorize_policy(&exec,&packet,1)==ERROR_READ_PROTECTED);
    security.library.lib_OpenCnt=7;
    port.mp_Flags=1;
    assert(afsplus_aros_authorize_policy(&exec,&packet,2)==ERROR_READ_PROTECTED);
    port.mp_Flags=PA_SIGNAL; port.mp_SigTask=NULL;
    assert(afsplus_aros_authorize_policy(&exec,&packet,2)==ERROR_READ_PROTECTED);
    packet.dp_Port=NULL;
    assert(afsplus_aros_authorize_policy(&exec,&packet,2)==ERROR_READ_PROTECTED);
    assert(queries==4 && !forbidden && security.library.lib_OpenCnt==7);
    puts("resident-only authority: PASS (absence, revocation, sender, pin lifetime)");
    return 0;
}
