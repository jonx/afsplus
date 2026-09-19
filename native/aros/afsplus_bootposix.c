/*
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * The POSIX functions Rust's standard library refers to, answered without
 * posixc.library.
 *
 * The handler reaches none of them while it runs: it takes time from its
 * packets, touches no host file, spawns no thread and reads no environment.
 * Linked through posixc's stubs, they still made posixc.library and
 * stdcio.library start-up requirements, and a boot volume's handler starts
 * before either can (they need dos.library, which waits for the boot
 * volume). Defined here, the stubs are never linked; tools/check-aros-ffi.sh
 * refuses a handler that still carries a posixc or stdcio base. Each answers
 * as an unavailable service does. write to standard error goes to the debug
 * log, so a panic's message survives.
 */

#define AROS_LIBREQ(bname, ver)

#include <exec/execbase.h>
#include <aros/debug.h>

#include <stddef.h>

extern struct ExecBase *SysBase;

typedef long ssize_type;

ssize_type write(int fd, const void *buffer, size_t length)
{
    const char *text = buffer;
    size_t index;

    if (fd != 1 && fd != 2)
        return -1;
    for (index = 0; index < length; index++)
        bug("%c", text[index]);
    return (ssize_type)length;
}

ssize_type read(int fd, void *buffer, size_t length)
{
    (void)fd;
    (void)buffer;
    (void)length;
    return -1;
}

int open(const char *path, int flags, ...)
{
    (void)path;
    (void)flags;
    return -1;
}

int close(int fd)
{
    (void)fd;
    return -1;
}

int dup(int fd)
{
    (void)fd;
    return -1;
}

long lseek(int fd, long offset, int whence)
{
    (void)fd;
    (void)offset;
    (void)whence;
    return -1;
}

int ftruncate(int fd, long length)
{
    (void)fd;
    (void)length;
    return -1;
}

int fstat(int fd, void *status)
{
    (void)fd;
    (void)status;
    return -1;
}

int stat(const char *path, void *status)
{
    (void)path;
    (void)status;
    return -1;
}

int lstat(const char *path, void *status)
{
    (void)path;
    (void)status;
    return -1;
}

int fchmod(int fd, unsigned mode)
{
    (void)fd;
    (void)mode;
    return -1;
}

int chmod(const char *path, unsigned mode)
{
    (void)path;
    (void)mode;
    return -1;
}

int chdir(const char *path)
{
    (void)path;
    return -1;
}

char *getcwd(char *buffer, size_t length)
{
    (void)buffer;
    (void)length;
    return NULL;
}

int mkdir(const char *path, unsigned mode)
{
    (void)path;
    (void)mode;
    return -1;
}

int rmdir(const char *path)
{
    (void)path;
    return -1;
}

int remove(const char *path)
{
    (void)path;
    return -1;
}

int unlink(const char *path)
{
    (void)path;
    return -1;
}

int rename(const char *from, const char *to)
{
    (void)from;
    (void)to;
    return -1;
}

int symlink(const char *target, const char *path)
{
    (void)target;
    (void)path;
    return -1;
}

ssize_type readlink(const char *path, char *buffer, size_t length)
{
    (void)path;
    (void)buffer;
    (void)length;
    return -1;
}

int utimes(const char *path, const void *times)
{
    (void)path;
    (void)times;
    return -1;
}

void *opendir(const char *path)
{
    (void)path;
    return NULL;
}

void *readdir(void *directory)
{
    (void)directory;
    return NULL;
}

int closedir(void *directory)
{
    (void)directory;
    return -1;
}

char *getenv(const char *name)
{
    (void)name;
    return NULL;
}

int setenv(const char *name, const char *value, int overwrite)
{
    (void)name;
    (void)value;
    (void)overwrite;
    return -1;
}

int unsetenv(const char *name)
{
    (void)name;
    return -1;
}

char *strerror(int number)
{
    (void)number;
    return (char *)"unavailable to the AFS+ handler";
}

int clock_gettime(int clock, void *time)
{
    (void)clock;
    (void)time;
    return -1;
}

int gettimeofday(void *time, void *zone)
{
    (void)time;
    (void)zone;
    return -1;
}

int posix_memalign(void **pointer, size_t alignment, size_t size)
{
    (void)alignment;
    (void)size;
    *pointer = NULL;
    return 12; /* ENOMEM: heap.rs never asks for an alignment malloc lacks. */
}
