/* SPDX-License-Identifier: BSD-2-Clause */
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
typedef int32_t LONG;typedef unsigned long ULONG;
#define CMD_UPDATE 4
#define IOERR_NOCMD -3
#define ERROR_BAD_NUMBER 115
#define ERROR_ACTION_NOT_KNOWN 209
#define ERROR_NOT_A_DOS_DISK 225
struct Node{const char *ln_Name;};struct Library{struct Node lib_Node;uint16_t lib_Version,lib_Revision;};struct Device{struct Library dd_Library;};struct ExecBase{int unused;};
struct IORequest{struct Device *io_Device;unsigned io_Command,io_Flags;void *io_Data;ULONG io_Length,io_Offset,io_Actual;LONG io_Error;};struct IOExtTD{struct IORequest iotd_Req;};struct AfsplusArosHandler{struct ExecBase *SysBase;struct IOExtTD *device_request;};
static unsigned calls;static LONG device_error,transport_error;
static LONG actual_io(struct IORequest *r){assert(r->io_Command==CMD_UPDATE&&!r->io_Flags&&!r->io_Data&&!r->io_Length&&!r->io_Offset&&!r->io_Actual);calls++;r->io_Error=device_error;return transport_error;}
#define DoIO(r) ((void)SysBase,actual_io(r))
#include "actual.c"
int main(void){struct ExecBase exec={0};struct Device dev={.dd_Library={.lib_Node.ln_Name="hostdisk.device",.lib_Version=1,.lib_Revision=1}};struct IOExtTD req={.iotd_Req.io_Device=&dev};struct AfsplusArosHandler handler={&exec,&req};unsigned cases=0;
 assert(!device_sync(&handler)&&calls==1);cases++;dev.dd_Library.lib_Revision=0;assert(device_sync(&handler)==ERROR_ACTION_NOT_KNOWN&&calls==1);cases++;dev.dd_Library.lib_Version=0;assert(device_sync(&handler)==ERROR_ACTION_NOT_KNOWN&&calls==1);cases++;dev.dd_Library.lib_Version=2;assert(!device_sync(&handler)&&calls==2);cases++;
 dev.dd_Library.lib_Node.ln_Name="other.device";dev.dd_Library.lib_Version=0;assert(!device_sync(&handler));cases++;device_error=IOERR_NOCMD;assert(device_sync(&handler)==ERROR_ACTION_NOT_KNOWN);cases++;device_error=0;transport_error=IOERR_NOCMD;assert(device_sync(&handler)==ERROR_ACTION_NOT_KNOWN);cases++;transport_error=0;device_error=20;assert(device_sync(&handler)==ERROR_NOT_A_DOS_DISK);cases++;device_error=0;transport_error=20;assert(device_sync(&handler)==ERROR_NOT_A_DOS_DISK);cases++;
 req.iotd_Req.io_Device=NULL;assert(device_sync(&handler)==ERROR_BAD_NUMBER);cases++;handler.device_request=NULL;assert(device_sync(&handler)==ERROR_BAD_NUMBER);cases++;printf("PASS %u actual AFS hostdisk version/support/error cases\n",cases);return 0;}
