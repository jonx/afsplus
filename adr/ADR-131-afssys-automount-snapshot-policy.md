# ADR-131: Supply AFSSYS snapshot budgets through its deployment Control

Status: Accepted
References: ADR-071, ADR-073, ADR-125

## Context

Apple NVMe automount creates the AFSSYS device node directly. Its environment
already reaches DE_BOOTBLOCKS, including DE_CONTROL, but that field was zero.
The AROS handler therefore received no snapshot budgets and refused volumes
whose immutable format enables snapshots.

## Decision

The Apple AFSSYS deployment supplies its Control BSTR explicitly: snapshot edit
records 4096, views 8 and reclaim records 64, with BACKUP=ROOT and RESTORE=ROOT.
This is trusted compiled deployment configuration, declared in a dedicated
header and overridable by the administrator when building a generation.
Generic handler defaults remain DENY and zero budgets. ROOT requires the
actual security.library task-owner check and denies when that service is absent
or the sender is unknown. There is no implicit SINGLEUSER authority; an
installation without security.library needs explicit administrative deployment
configuration before admitting historical operations.

Use the platform's AROS_CONST_BSTR representation and its resident static
lifetime, retained by MakeDosNode's copied environment. applenvme.device is
non-expunging. Bound the literal to the classic BSTR maximum, including its
terminator. ESP and WORK FAT nodes keep their existing zero Control. Pass no
FORMATSNAPSHOTS option: this changes runtime mounting only, neither formatting
policy nor any partition bytes. No new firmware property or boot ABI is added.

The selected defaults provide explicit bounded admission, not a measured M1
memory guarantee. Raising them requires resource qualification. Reclaim work
is smaller than the edit budget; caller limits and registered-view lifetime
remain those of ADR-071 and ADR-073.

## Qualification

Execute actual add_disk_boot_node source on the host for classic and fast BSTR
representations. Capture the copied environment, parse its Control with the
production handler parser, and check exact budgets, ROOT policy and absent
format opt-in. Check FAT nodes, failed allocation/registration cleanup and
resident BSTR lifetime. Test explicit build override and overlong-literal
refusal. Existing authority tests prove ROOT refuses without security.library.
Compile for Apple AArch64 without changing an in-flight VM SDK. Actual AFSSYS
native qualification requires an installed generation. Enabling snapshots on
internal AFSSYS still requires separately confirmed migration or reformatting.
