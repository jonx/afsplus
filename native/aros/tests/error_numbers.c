/* SPDX-License-Identifier: BSD-2-Clause */

/* Compile-time check of error_numbers.h against the target's DOS header. */

#include <dos/dos.h>

#define AFSPLUS_DOS_ERROR(name, number) \
    _Static_assert(name == number, #name " differs from <dos/dos.h>");
#include "error_numbers.h"

int afsplus_error_numbers_checked;
