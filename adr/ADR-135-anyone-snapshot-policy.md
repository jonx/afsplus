# ADR-135: Admit every local task to AFSSYS snapshots

Status: Accepted
Amends: ADR-125, ADR-131

## Context

The AROS system on an Apple Silicon Mac has one person at the keyboard. ROOT
admits only the security.library root identity, and fails closed when that
service is absent, so AFSSYS refused every snapshot, backup and restore
request. SINGLEUSER admits only when the service is absent. Neither policy lets
the person using the machine take and restore snapshots.

## Decision

Add the ANYONE policy for `BACKUP=` and `RESTORE=` in the DOSDriver Control
string. It admits every sender whose reply port is a signal port with a task,
without querying any authority service. Malformed senders are refused as under
the other policies. Generic handler defaults remain DENY.

The Apple AFSSYS deployment selects `BACKUP=ANYONE RESTORE=ANYONE`. The snapshot
budgets of ADR-131 are unchanged. Snapshot readers and directories still belong
to the task that opened them.

## Consequences

Any local program can read every registered snapshot, including the content of
files deleted since, and can restore into a destination it can write. That is
the intended single-user behaviour. A deployment that admits several users
selects ROOT, or a narrower policy, before those users share a volume.

## Qualification

The host authority test admits ANYONE with and without a loaded
security.library, issues no owner query for it and refuses malformed senders.
The Control parser test accepts `BACKUP=ANYONE RESTORE=ANYONE`.
