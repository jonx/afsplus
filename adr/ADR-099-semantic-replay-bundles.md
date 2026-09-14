# ADR-099: Bind semantic scenarios and replay results in complete bundles

Status: Proposed

## Decision under qualification

A replay bundle preserves the inputs needed to reproduce a specific filesystem
qualification run. The bundle binds a versioned semantic scenario, source revision
and dirty-worktree identity, base geometry/image identity, recorded block trace,
fault-model version and selected cut, explicit resource policy, expected state
and observed diagnostics. Versioned cache profiles bind execution, remount and
recovered observation; missing or conflicting policy records are rejected.
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
same observable failure signature, including any explicit cache profile, recurs; an unrelated parser error, resource
refusal or missing prerequisite cannot replace the original failure. Preserve the
original bundle and publish a new complete bundle for the reduced reproducer.

## Rebuilt-runner comparison

Exact replay requires the recorded source and executable identities. A separately
named rebuilt comparison permits a different caller-selected executable only
when the observed source identity matches the original. Compare all non-metadata
artifact bytes, preserve both complete bundles and report both executable digests
and outcomes. A diagnostic or block-trace difference is a comparison failure even
if the semantic run passes. An equal failing run is a reproduced failure, never
a successful qualification. Keep original reduction metadata with the original.

Publish a distinct comparison completion record after synchronizing both bundles.
Never overwrite original or partial output. The comparison records observations;
it cannot attest build provenance or replace preservation of sources, toolchains
and dependencies. Exact replay has no identity-bypass option.

## Working-source companion

Preserve a separate content-addressed source package binding the same observed
revision and working-tree digest as the semantic run. Include HEAD-reachable Git
history, working-file bytes/modes, relative symlinks, missing tracked paths and
index blobs/stages independently of working bytes. Refuse source changes during
capture. Retain raw path bytes in the manifest and validate paths, kinds, index
objects, per-file sizes, aggregate blob sizes and expanded worktree sizes before
restoration into a fresh directory. Publish a completion manifest only after the
package's artifacts and directory entries are synchronized.

Restoration must reproduce both source and index identities before success. It
never executes captured scripts or Git hooks. Unsupported host path/mode semantics
fail explicitly. Keep package restoration separate from build execution and from
preservation of dependencies, toolchains and build environment. The source
companion is a reconstruction input, not a general repository backup or build
provenance certificate.

## Registry-dependency companion

Preserve the locked registry dependency tree separately from local path sources.
Bind its complete relative-file inventory to the observed source and lockfile
identities. Capture through offline locked vendoring; absent prepared dependencies
cause refusal. A profile supports only explicitly recognized source replacements.
Validate file kinds, paths, sizes, modes, digests and crate metadata, synchronize
the generated tree, then publish a distinct completion manifest. Preserve partial
output and report late barrier failures.

A dependency reconstruction gate requires a frozen offline build with an initially
empty Cargo home and fresh target, an empty replacement-source negative control,
and independent semantic comparisons against retained bundles. Matching dependency
files or a warm-cache build alone cannot satisfy that gate. Compiler, sysroot,
linker, SDK and environment preservation are separate inputs; observed executable
digests are not copies of those inputs or a build-provenance certificate.

## Host toolchain profile

A reconstructed build records explicit compiler/sysroot, linker, SDK and runtime
requirements. Preserve selected non-system toolchain bytes and their relative
library layouts. Name the host OS/architecture and runtime prerequisites rather
than claiming that a copied compiler eliminates them. Tool selection applies to
host build scripts as well as filesystem crates. Require independent absent-linker
and empty-SDK controls, alongside the empty-dependency-source control, before
accepting the reconstructed build's input selection.

Host-profile semantic reproducibility is separate from bit-identical executables,
cross-host compatibility and physical provider qualification. A prototype copy
inventory cannot substitute for a qualified reusable package verifier or build
orchestrator. Additional host profiles follow the portability gates without
implicitly requiring every platform to close the first executable host stage.

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
