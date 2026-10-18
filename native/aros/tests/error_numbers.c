/* SPDX-License-Identifier: BSD-2-Clause */

/* Compile-time check against target DOS and adapter-specific error headers. */

#include <dos/dos.h>
#include "afsplus_aros.h"

#define AFSPLUS_DOS_ERROR(name, number) \
    _Static_assert(name == number, #name " differs from its canonical error header");
#include "error_numbers.h"

int afsplus_error_numbers_checked;
