# ADR-134: Query a permanently loaded AROS authority service

Status: Accepted
Amends: ADR-125

## Decision

The AROS ROOT policy admits the loaded `security.library` service with major
version 45 and revision at least 17. This host ABI guarantees that the loaded
library remains registered until shutdown: its Expunge vector never removes
it. The implementation contract is `rom/security/security.conf`'s
`options noexpunge`, whose generated Expunge returns zero. Future compatible
45.x revisions must retain that lifetime, vector and identity contract; a
different lifetime requires a new host ABI and admission decision.

Read the loaded library identity under Exec's public library-list exclusion.
If it is absent, ROOT fails closed; only explicitly configured SINGLEUSER
admits that case. A loaded unsupported version fails closed under either
policy. Never ask the loader to find a library that was not already admitted.

After validating the permanent loaded service, release the list lock before
normal OpenLibrary and the ownership query. Its permanent lifetime closes the
lookup/open unload race. Require the returned base to equal the validated
base; otherwise close the acquired reference and refuse admission.

Open the service once, at the first admission, and keep that open for the
life of the loaded handler. The service's Open and Close vectors push and pop
a context under its task-owner semaphore: opening per packet would block the
handler behind a root tool that holds the semaphore while it uses the same
volume. Later packets only query the task owner. Handler processes that share
the loaded code publish the base under Forbid; one that loses closes its own
open. The open is never returned, which the permanent lifetime makes
harmless. No adapter operation changes `lib_OpenCnt` directly. The list lock
does not serialize generated library Open/Close counter updates, so a temporary
manual pin is not a valid SMP lifetime primitive. This decision does not claim
to repair generic library lifecycle synchronization in Exec or the loader.

Bind the service's public vector 7, Task pointer in D0 and unsigned 32-bit
owner result. Its ROOT UID is 65535 and nobody UID is zero. Test the high-bit
ROOT owner explicitly. The adapter binds these identities to the validated
runtime ABI, independent of an older build SDK's conflicting ROOT constants.
An older SDK can compile the external handler using this same binding; it
cannot make an incompatible loaded authority service trusted. No legacy
identity fallback is supplied.

Preserve ADR-125's current-sender checks and per-packet revocation. File and
tree restoration require both BACKUP and RESTORE authority; stage reclamation
requires RESTORE and reads no historical data. Never hold the library-list
spinlock across library hooks or the task-owner semaphore query.

## Qualification

Compile the production helper into a host regression. Check absent, supported,
older and unknown services; actual ROOT 65535, nobody zero and non-root owners;
failed Open and a different returned base; one Open across many packets,
with no Close after admission; and a lost publication race that closes its own
open.
Assert no manual reference-counter mutations and no library hooks under the
list lock. Inspect the admitted core's no-expunge implementation independently.
Keep the loader-recursion and skipped-BACKUP negative controls effective.
Compile public AArch64 and m68k SDK objects, then qualify the exact VM candidate
and an identified installed M1 generation with retained harness artifacts.
