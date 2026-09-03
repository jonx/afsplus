/* SPDX-License-Identifier: BSD-2-Clause */

#include <libafsplus_reader.h>
#include <libafsplus_writer.h>

#include <string.h>

int main(void)
{
    return strcmp(afspr_status_string(AFSPR_ERR_CORRUPT), "corrupt format") ==
                       0 &&
                   (afspr_capabilities() & AFSPR_CAP_FILE_READ) != 0u
                   && (afspr_capabilities() & AFSPR_CAP_INTENT_FILE_READ) != 0u
                   && (afspw_capabilities() &
                       AFSPW_CAP_RENAME_FILE_NO_REPLACE) != 0u
                   && (afspw_capabilities() &
                       AFSPW_CAP_TRUNCATE_FILE_DATA_FREE) != 0u
                   && strcmp(
                          afspw_status_string(
                              AFSPW_ERR_TAIL_REWRITE_REQUIRED),
                          "truncate tail rewrite required") == 0
               ? 0
               : 1;
}
