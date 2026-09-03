/* SPDX-License-Identifier: BSD-2-Clause */

#include <libafsplus_reader.h>

#include <string.h>

int main(void)
{
    return strcmp(afspr_status_string(AFSPR_ERR_CORRUPT), "corrupt format") ==
                       0 &&
                   (afspr_capabilities() & AFSPR_CAP_FILE_READ) != 0u
               ? 0
               : 1;
}
