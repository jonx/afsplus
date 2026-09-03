/* SPDX-License-Identifier: BSD-2-Clause */

#include "harness.h"

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size)
{
    struct afspr_fuzz_outcome outcome;

    (void)afspr_fuzz_run_input(data, size, &outcome);
    return 0;
}
