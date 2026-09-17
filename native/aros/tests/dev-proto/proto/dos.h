/* SPDX-License-Identifier: BSD-2-Clause */
/* Development stand-in for the generated <proto/dos.h>; see proto/exec.h. */
#ifndef AFSPLUS_DEV_PROTO_DOS_H
#define AFSPLUS_DEV_PROTO_DOS_H

#include <dos/dos.h>
#include <dos/dosextens.h>

SIPTR IoErr(void);
struct DateStamp *DateStamp(struct DateStamp *date);
struct DosList *MakeDosEntry(CONST_STRPTR name, LONG type);
LONG FreeDosEntry(struct DosList *dlist);
LONG AddDosEntry(struct DosList *dlist);
LONG RemDosEntry(struct DosList *dlist);
struct DosList *AttemptLockDosList(ULONG flags);
void UnLockDosList(ULONG flags);

#endif
