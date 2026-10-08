/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 John Knipper */
APTR host_alloc_lock(struct Library *);
void host_free_lock(struct Library *, APTR);
int host_obtain_lock(struct Library *, APTR, ULONG, ULONG);
void host_release_lock(struct Library *, APTR, ULONG);
int host_obtain_system_lock(struct Library *, struct List *, ULONG, ULONG);
void host_release_system_lock(struct Library *, struct List *, ULONG);
#define AllocLock() host_alloc_lock(ExecLockBase)
#define FreeLock(lock) host_free_lock(ExecLockBase, lock)
#define ObtainLock(lock, mode, flags) host_obtain_lock(ExecLockBase, lock, mode, flags)
#define ReleaseLock(lock, flags) host_release_lock(ExecLockBase, lock, flags)
#define ObtainSystemLock(list, mode, flags) host_obtain_system_lock(ExecLockBase, list, mode, flags)
#define ReleaseSystemLock(list, flags) host_release_system_lock(ExecLockBase, list, flags)
