/* SPDX-License-Identifier: BSD-2-Clause */
/* The five <string.h> functions the portable reader and writer use, for a
 * bare-metal m68k cross compiler that ships no C library headers. It lets
 * tools/check-portable-c-reader.sh compile both sources for m68k; nothing
 * is linked, so no definitions are needed. */
#ifndef AFSPLUS_M68K_FREESTANDING_STRING_H
#define AFSPLUS_M68K_FREESTANDING_STRING_H

#include <stddef.h>

void *memchr(const void *s, int c, size_t n);
int memcmp(const void *a, const void *b, size_t n);
void *memcpy(void *dst, const void *src, size_t n);
void *memmove(void *dst, const void *src, size_t n);
void *memset(void *s, int c, size_t n);

#endif
