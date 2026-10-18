# ADR-128: Publish a restored snapshot tree through one fresh directory

Status: Accepted
Amends: ADR-127

## Context

A preference directory and a selected backup directory require a tree consumer
of the fresh-allocation restore contract. Publication must preserve a complete
captured namespace at its requested destination, including directory metadata.

## Decision

Restore a captured directory into one explicitly named fresh destination on
the same volume. Check historical and destination-scoped authority at admission
as for regular-file restoration. Preflight the complete selected tree before
creating any stage. Refuse unsupported object kinds, hard-linked files,
security descriptors and exhausted caller budgets before target mutation.

The caller supplies positive node, depth and aggregate opaque-metadata budgets.
These are runtime operation limits and change no disk feature or namespace
limit. The command supplies explicit budgets and reports admission refusal.
Host qualification records workloads and resource bounds for those budgets.

Keep the snapshot pinned throughout traversal and copying. Build a unique
staging directory beneath the selected live destination parent. Exclude the
final name's versioned comparison key when choosing the stage, including
Unicode-equivalent spellings. Copy supported regular files into fresh storage
and create directories with preserved comments, attributes, protection, owner
and all captured timestamps. Restore directory metadata after its children.

Verify the complete staged namespace and metadata, then publish it with one
non-replacing rename that preserves the captured root's change timestamp.
The live parent records the publication time. A failure before publication
leaves the requested final name absent. Cleanup removes the staged subtree
where storage remains writable; failure identifies the destination and stage
prefix for inspection. A publication barrier error can leave the complete final
tree present and requires inspection before retry.

This is scoped namespace restoration. Selecting a snapshot as a boot root
requires the separately approved system rollback design.

## API and resources

Add SNAPSHOT_RESTORE_TREE with an explicit source, directory lock, target name
and operation budgets. The core's preserved-time publication helper applies to
files and directories and keeps ordinary rename semantics unchanged. Use the
existing on-disk records and fresh allocation lifetime rules.

The preflight plan consumes bounded memory proportional to admitted nodes,
names and metadata. Stream file data in bounded buffers and account for copy
headroom. Do not allocate a full file buffer or one stream buffer per traversal
frame. Staging space covers the selected allocation plus metadata and ordinary
checkpoint quarantine; restoration never expires a registered snapshot.

## Qualification

Cover nested and empty directories, captured bytes after live mutation, every
supported metadata field, case-insensitive names, stage/final key collisions,
remount and repeated restore to an existing target. Check node, depth and
metadata admission failures before writes, unsupported descendants and cleanup.
Enumerate publication flush failures and require the final namespace to be
absent or complete. Preserve the source snapshot in every permitted outcome.
Include an independent missing-publication-filter control and actual VM calls.
