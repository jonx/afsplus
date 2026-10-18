/* SPDX-License-Identifier: BSD-2-Clause */
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include "afsplus_security_abi.h"
typedef uint32_t ULONG;
typedef const char *CONST_STRPTR;
/* Deliberately no reference-count member: adapter code cannot change one. */
struct Library { uint16_t lib_Version, lib_Revision; };
struct List { unsigned unused; };
struct ExecBase { struct List LibList; };
struct Task { ULONG owner; };
struct MsgPort { unsigned mp_Flags; struct Task *mp_SigTask; };
struct DosPacket { struct MsgPort *dp_Port; };
#define PF_ACTION 3
#define PA_SIGNAL 0
#define ERROR_READ_PROTECTED 224
#define SECURITYNAME "security.library"
/* An old build SDK must never make modern nobody(0) into ROOT. */
#define secROOT_UID 0
#define SPINLOCK_MODE_READ 0
#define LOCKF_FORBID 2
static unsigned smp, forbidden, queries, opens, closes, open_fail, return_other;
static unsigned references=7, other_references=2, race;
static struct Library lock_resource, other={45,17}, security={45,17};
static struct Library *loaded;
static void (*race_winner)(void);
static struct Library *OpenResource(CONST_STRPTR name)
{ assert(!strcmp(name,"execlock.resource")); return smp ? &lock_resource : NULL; }
static void Forbid(void) { assert(!forbidden); forbidden=1; }
static void Permit(void) { assert(forbidden); forbidden=0; }
static void ObtainSystemLock(struct List *list, unsigned mode, unsigned flags)
{ (void)list; assert(mode==SPINLOCK_MODE_READ && flags==LOCKF_FORBID); Forbid(); }
static void ReleaseSystemLock(struct List *list, unsigned flags)
{ (void)list; assert(flags==LOCKF_FORBID); Permit(); }
static void *FindName(struct List *list, CONST_STRPTR name)
{ (void)list; assert(forbidden); assert(!strcmp(name,SECURITYNAME)); return loaded; }
static ULONG afsplus_security_task_owner(struct Library *base, struct Task *task)
{
    assert(!forbidden && base==&security && references==8);
    queries++;
    return task->owner;
}
static struct Library *OpenLibrary(CONST_STRPTR name, unsigned version)
{
    assert(!forbidden && !strcmp(name,SECURITYNAME));
    assert(version==AFSPLUS_SECURITY_VERSION);
    /* Absent or unsupported service would reach the forbidden disk loader. */
    assert(loaded==&security && security.lib_Version==45 && security.lib_Revision>=17);
    assert(references==7);
    opens++;
    if(open_fail)return NULL;
    if(return_other){other_references++;return &other;}
    references++;
    /* Another handler process admitted the service between our lookup and
     * Open: it holds its own open, and the slot is already published. */
    if(race){race=0;references++;race_winner();}
    return loaded;
}
static void CloseLibrary(struct Library *library)
{
    assert(!forbidden);closes++;
    if(library==&other){assert(other_references==3);other_references--;return;}
    assert(library==&security && references==9);
    references--;
}
#include "afsplus_authority.h"
static void publish_winner(void) { afsplus_aros_security_base=&security; }
int main(void)
{
    struct ExecBase exec={0};
    struct Task sender={0};
    struct MsgPort port={PA_SIGNAL,&sender};
    struct DosPacket packet={&port};
    race_winner=publish_winner;
    assert(AFSPLUS_SECURITY_ROOT_UID==65535);
    assert(AFSPLUS_SECURITY_GET_TASK_OWNER_LVO==7);
    /* Absent service: SINGLEUSER only, nothing loaded or opened. */
    for(unsigned policy=0;policy<=3;policy++)
        assert(afsplus_aros_authorize_policy(&exec,&packet,policy)
            ==(policy==2?0:ERROR_READ_PROTECTED));
    assert(!queries && !opens && !forbidden);
    loaded=&security;smp=1;
    /* Unsupported ABIs are refused before any Open. */
    for(unsigned variant=0;variant<3;variant++) {
        security.lib_Version=variant==0?40:variant==1?46:45;
        security.lib_Revision=variant==2?16:17;
        for(unsigned policy=1;policy<=2;policy++)
            assert(afsplus_aros_authorize_policy(&exec,&packet,policy)==ERROR_READ_PROTECTED);
    }
    assert(!opens && !queries);
    security.lib_Version=45;security.lib_Revision=17;
    sender.owner=UINT32_C(0xffff0000);
    /* A failed or foreign Open publishes nothing and keeps no reference. */
    open_fail=1;
    assert(afsplus_aros_authorize_policy(&exec,&packet,2)==ERROR_READ_PROTECTED);
    open_fail=0;
    return_other=1;
    assert(afsplus_aros_authorize_policy(&exec,&packet,1)==ERROR_READ_PROTECTED);
    return_other=0;
    assert(references==7 && other_references==2 && !queries
        && opens==2 && closes==1 && afsplus_aros_security_base==NULL);
    /* Losing the publication race closes our own open, uses the winner's. */
    race=1;
    assert(afsplus_aros_authorize_policy(&exec,&packet,1)==0);
    assert(opens==3 && closes==2 && references==8 && queries==1
        && afsplus_aros_security_base==&security);
    /* Every later packet: owner query only, no Open, no Close. */
    for(unsigned n=0;n<1000;n++)
        for(unsigned policy=1;policy<=2;policy++) {
            sender.owner=(AFSPLUS_SECURITY_ROOT_UID<<16)|123;
            assert(sender.owner & UINT32_C(0x80000000));
            assert(afsplus_aros_authorize_policy(&exec,&packet,policy)==0);
            sender.owner=0; /* Modern nobody and the old SDK's ROOT value. */
            assert(afsplus_aros_authorize_policy(&exec,&packet,policy)==ERROR_READ_PROTECTED);
            sender.owner=1u<<16; /* Same sender, current authority revoked. */
            assert(afsplus_aros_authorize_policy(&exec,&packet,policy)==ERROR_READ_PROTECTED);
        }
    assert(opens==3 && closes==2 && references==8 && queries==1+6000 && !forbidden);
    /* Malformed senders are refused before the owner query. */
    port.mp_Flags=1;
    assert(afsplus_aros_authorize_policy(&exec,&packet,2)==ERROR_READ_PROTECTED);
    port.mp_Flags=PA_SIGNAL;port.mp_SigTask=NULL;
    assert(afsplus_aros_authorize_policy(&exec,&packet,2)==ERROR_READ_PROTECTED);
    packet.dp_Port=NULL;
    assert(afsplus_aros_authorize_policy(&exec,&packet,2)==ERROR_READ_PROTECTED);
    assert(opens==3 && queries==6001 && !forbidden && references==8);
    puts("loaded permanent authority: PASS ROOT65535/nobody0/version/one-open/race/revocation");
    return 0;
}
