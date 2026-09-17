/* SPDX-License-Identifier: BSD-2-Clause */
/* Development stand-in for the generated <proto/exec.h>: plain prototypes of
 * the calls the AFS+ handler shell makes, for a syntax and type check on a
 * host whose AROS build has not generated the proto headers yet. The real
 * header routes these through SysBase; signatures follow exec.conf. */
#ifndef AFSPLUS_DEV_PROTO_EXEC_H
#define AFSPLUS_DEV_PROTO_EXEC_H

#include <exec/types.h>
#include <exec/ports.h>
#include <exec/io.h>
#include <exec/tasks.h>
#include <exec/libraries.h>

APTR AllocMem(IPTR byteSize, ULONG requirements);
void FreeMem(APTR memoryBlock, IPTR byteSize);
struct MsgPort *CreateMsgPort(void);
void DeleteMsgPort(struct MsgPort *port);
struct Message *GetMsg(struct MsgPort *port);
void PutMsg(struct MsgPort *port, struct Message *message);
void ReplyMsg(struct Message *message);
struct Message *WaitPort(struct MsgPort *port);
ULONG Wait(ULONG signalSet);
void Signal(struct Task *task, ULONG signalSet);
struct Task *FindTask(CONST_STRPTR name);
struct Library *OpenLibrary(CONST_STRPTR libName, ULONG version);
struct Library *TaggedOpenLibrary(LONG tag);
void CloseLibrary(struct Library *library);
APTR OpenResource(CONST_STRPTR resName);
LONG OpenDevice(CONST_STRPTR devName, IPTR unitNumber,
    struct IORequest *iORequest, ULONG flags);
void CloseDevice(struct IORequest *iORequest);
LONG DoIO(struct IORequest *iORequest);
APTR CreateIORequest(struct MsgPort *ioReplyPort, ULONG size);
void DeleteIORequest(APTR iorequest);

#endif
