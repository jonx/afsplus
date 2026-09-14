# ADR-099: Bind semantic scenarios and replay results in complete bundles

Status: Proposed

## Decision under qualification

A replay bundle preserves the inputs needed to reproduce a specific filesystem
qualification run. The bundle binds a versioned semantic scenario, source revision
and dirty-worktree identity, base geometry/image identity, recorded block trace,
fault-model version and selected cut, expected state and observed diagnostics.
Each file has a fixed relative role and content digest in a manifest. Reject
absolute paths, parent traversal, symlinks, duplicate roles, missing roles,
unknown versions and unbounded file sizes before replay admission.

Semantic operations use stable scenario-local object labels and preserve exact
name/data bytes. The runner resolves labels to filesystem identities and records
those mappings as diagnostics. User paths and shell commands are not executable
scenario operations. The first finite profile must cover the Stage A image
operation ladder: format, create, write, truncate, directory creation, rename,
unlink, sync and remount with exact namespace/content assertions. Additional
operation families extend a versioned profile with their queue-owned gates.

Replay reads the supplied base exclusively or copies an admitted base into a
stable memory provider, verifies its identity, and applies operations only to
bounded overlay branches or a newly created private fixture image. Original
artifacts remain unchanged. A stored block trace supports crash-state reproduction;
it does not replace the semantic scenario or an independent expected-state oracle.

Create a fresh output directory exclusively. Write all artifact files, verify
sizes/digests and synchronize them before publishing the completion manifest;
synchronize the containing directory and the new bundle name in its parent
after publication. An interrupted export can leave an incomplete directory or a
fully formed bundle. A failed final barrier must report failure even when the
manifest is readable; integrity verification cannot prove that barrier completed. Never overwrite caller data or silently
accept a partial bundle. Keep failure and successful-qualification bundles distinct
through explicit outcome fields; integrity alone cannot imply test success.

## Minimization contract

A minimizer starts from a reproducing failure and tests deletion of semantic
operation ranges under the same deterministic settings and fault model. Invalid
label dependencies are rejected candidates. Retain a reduction only when the
same observable failure signature recurs; an unrelated parser error, resource
refusal or missing prerequisite cannot replace the original failure. Preserve the
original bundle and publish a new complete bundle for the reduced reproducer.

## Qualification

Require fresh-process export/read/replay equivalence, exact expected-state checks,
source/geometry/base/fault binding, corrupted or incomplete bundle refusal and
zero writes to original artifacts. Interrupt each export publication boundary.
Include a deliberately wrong expected state as a negative control and minimize a
scenario with irrelevant operations while preserving its failure signature.

Keep the complete Stage A failure-artifact list in the developer harness, including
semantic operations, block I/O, flight recorder, images, fault model and expected/
actual records. This proposal does not remove cache-profile or resource-accounting
requirements. Native provider qualification remains separate.
