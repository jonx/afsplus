# ADR-059: Treat guest failure diagnostics as qualification verdicts

Status: Accepted for all current AROS runtime gates

## Context

An AROS `Software Failure!` requester can stop the filesystem task while the
emulator, serial connection or Hosted process remains alive. A timeout or clean
host process status is therefore not a sufficient guest verdict. This first
hid the M68000 illegal instruction diagnosed in ADR-057.

The gates had also drifted: FS-UAE scanned two requester names, Hosted scripts
used several overlapping crash patterns, and native QEMU relied on its boot
markers and QEMU fault log. A new failure spelling could consequently be
accepted on one platform and rejected on another.

## Decision

[`tools/check-aros-serial-log.sh`](../tools/check-aros-serial-log.sh) is the shared fatal-diagnostic boundary. It
rejects modal `Software Failure!` and `Guru Meditation` requesters plus the
Hosted fatal markers already used by the project: an AFS+ failure line, trap,
alert, unrecoverable state or host halt.

Every current runtime gate invokes it on the diagnostic channel available to
that environment:

- FS-UAE scans every serial/stdout log before accepting emulator status;
- Hosted Alpha-0, replay, S1 and S1b scan the captured AROS window log; and
- native Apple-AArch64 QEMU scans both firmware serial and semihost output.

Successful evidence records `guest_failure_requester=none` in native/m68k
reports or a checksummed `guest-failure-requester.txt` marker in Hosted result
sets. Existing operation, checker, boot-marker and consumed-input-integrity
verdicts remain mandatory; the diagnostic scan supplements rather than
replaces them.

## Evidence

The scanner self-test accepts a clean fixture and rejects both a complete
M68000 illegal-instruction requester and a Hosted trap fixture. It also rejects
the preserved historical M68000 failure log that originally exposed the bad
`MULU.L`, while accepting all seven logs from the qualified Alpha-0 plus
six-replay M68000 run.

On 2026-08-31 the integrated scanner passed:

- the bidirectional Hosted AROS → macFUSE → Hosted AROS Alpha-0 gate;
- all six Hosted intent-log replay cases;
- native Apple-AArch64 QEMU Alpha-0 with extracted checker-clean generation 7;
  and
- all six native QEMU replay cases, each with its expected old/new state and a
  checker-clean extracted image with zero pending intent records.

## Consequences

A modal requester can no longer masquerade as a hang or a passing outer
process in any supported qualification stage. When a new fatal AROS spelling
is found, it is added once with a negative fixture and becomes common policy.

Visual inspection remains useful for diagnosis and UI-only failures, but it is
not the sole protection against the known software-failure requester class.
