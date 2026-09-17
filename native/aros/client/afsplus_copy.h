/* SPDX-License-Identifier: BSD-2-Clause */

#ifndef AFSPLUS_COPY_H
#define AFSPLUS_COPY_H

/* The portable path beside afsplus_client_clone_file: a byte copy with the
 * same promise as the clone, that the target either becomes a complete copy
 * or is not touched at all. */
#include <dos/dos.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Copies the file behind source into directory under name. Returns 0 or an
 * ERROR_* value; IoErr() is left at 0.
 *
 * The bytes go to a temporary file in the same directory, which is renamed
 * to name at the end. Rename refuses an existing target, so a file of that
 * name is never replaced or truncated, whenever it appears
 * (ERROR_OBJECT_EXISTS). On every failure the temporary file is deleted and
 * nothing of the attempt remains. */
LONG afsplus_copy_file(BPTR source, BPTR directory, CONST_STRPTR name);

#ifdef __cplusplus
}
#endif

#endif
