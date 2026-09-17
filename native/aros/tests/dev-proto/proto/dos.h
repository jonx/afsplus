/* SPDX-License-Identifier: BSD-2-Clause */
/* Development stand-in for the generated <proto/dos.h>; see proto/exec.h. */
#ifndef AFSPLUS_DEV_PROTO_DOS_H
#define AFSPLUS_DEV_PROTO_DOS_H

#include <dos/dos.h>
#include <dos/dosextens.h>

SIPTR IoErr(void);
SIPTR SetIoErr(SIPTR result);
SIPTR DoPkt(struct MsgPort *port, LONG action, SIPTR arg1, SIPTR arg2,
    SIPTR arg3, SIPTR arg4, SIPTR arg5);
LONG Seek(BPTR file, LONG position, LONG mode);
LONG Read(BPTR file, APTR buffer, LONG length);
LONG Write(BPTR file, CONST_APTR buffer, LONG length);
BPTR Open(CONST_STRPTR name, LONG mode);
LONG Close(BPTR file);
BPTR Lock(CONST_STRPTR name, LONG mode);
LONG UnLock(BPTR lock);
BPTR DupLock(BPTR lock);
BPTR OpenFromLock(BPTR lock);
BPTR CurrentDir(BPTR lock);
LONG Rename(CONST_STRPTR from, CONST_STRPTR to);
LONG DeleteFile(CONST_STRPTR name);
struct DateStamp *DateStamp(struct DateStamp *date);
struct DosList *MakeDosEntry(CONST_STRPTR name, LONG type);
LONG FreeDosEntry(struct DosList *dlist);
LONG AddDosEntry(struct DosList *dlist);
LONG RemDosEntry(struct DosList *dlist);
struct DosList *AttemptLockDosList(ULONG flags);
void UnLockDosList(ULONG flags);

#endif
