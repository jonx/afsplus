/* SPDX-License-Identifier: BSD-2-Clause */
/* Development stand-in for the generated <proto/locale.h>; see proto/exec.h. */
#ifndef AFSPLUS_DEV_PROTO_LOCALE_H
#define AFSPLUS_DEV_PROTO_LOCALE_H

#include <libraries/locale.h>

struct Locale *OpenLocale(CONST_STRPTR name);
void CloseLocale(struct Locale *locale);

#endif
