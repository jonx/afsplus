/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
#include "../../dev-proto/proto/exec.h"
struct Node *FindName(struct List *, CONST_STRPTR);
void Reschedule(void);
ULONG host_process_message(struct ExecBase *, struct Task *, ULONG, struct Message *, ULONG);
#define TryProcessMessage(task, id, message, signals) \
    host_process_message(SysBase, task, id, message, signals)
