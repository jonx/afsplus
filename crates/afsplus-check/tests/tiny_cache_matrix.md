# Tiny-cache family audit

Audit baseline: `a2c241e`. Parent: Stage A `a-cache`, `roadmap-31`, board task 1.
This evidence inventory does not change the central milestone status.

Profiles: **P** = explicit 2/4/8/unlimited; **2** = explicit two-page;
**U** = default unlimited mount; **component** = allocator/tree model rather
than a mounted filesystem. A named test is source evidence, not a claimed
execution result. Unlimited means `usize::MAX`, not zero pages. Tests in this
directory are linked relative to this file; core tests use explicit paths.

## Baseline family-to-test matrix

| Executable family | Named baseline evidence | Profile and exact scope | Missing combinations |
|---|---|---|---|
| Batch create/delete, cancellation, payload tails | [cache_profiles.rs](cache_profiles.rs): `batch_create_delete_and_remount_match_at_two_four_eight_and_unlimited_pages`; [batch_payloads.rs](batch_payloads.rs): `surviving_payloads_borrow_full_blocks_and_zero_each_tail_in_all_profiles`, `cancelled_and_invalid_batches_do_not_issue_payload_writes` | P: exact namespace/bytes, checker, real spills, peak staged nodes, zero payload writes on refusal | None |
| Batch publication failures | [batch_payloads.rs](batch_payloads.rs): `borrowed_write_and_data_barrier_errors_preserve_old_state_and_allow_retry`, `borrowed_payload_crash_matrix_preserves_exact_old_or_new_state` | P: borrowed data faults and modeled cuts, exact old/new contents | None beyond an unlogged window; a window whose group reached the intent log belongs to the deferred families |
| Staged directory split | [cache_profiles.rs](cache_profiles.rs): `spilled_directory_split_is_atomic_at_every_modeled_cut`, `failed_provisional_spills_leave_the_committed_view_and_allow_retry` | 2: real spill, all modeled split cuts; writes 0/1/7/31 fail then retry | None beyond the measured descent depth of four levels stated in the evidence |
| Single create, mkdir, rmdir, hard links | [basic.rs](basic.rs); [crash_matrix.rs](crash_matrix.rs): `every_crash_state_of_a_create_transaction_recovers_to_an_allowed_state` | U: functional namespace/lifetime and create cuts | None |
| Cross-directory rename, root split/collapse | [crash_matrix.rs](crash_matrix.rs): `every_crash_state_of_cross_directory_rename_is_atomic`, `directory_root_split_and_collapse_are_crash_atomic` | U: atomic namespace and checker | None beyond the measured descent depth of four levels stated in the evidence |
| Replace rename, shared target | [faults.rs](faults.rs): `replacement_write_and_flush_failures_preserve_complete_namespace_and_bytes`; [shared_crash.rs](shared_crash.rs): `rename_replace_of_a_shared_target_is_crash_atomic`, `shared_replace_io_failures_preserve_exact_ownership_and_allow_retry_in_all_profiles`, `shared_replace_ambiguous_publication_requires_remount_in_all_profiles` | P: shared-target cuts with exact victim/incoming/peer bytes and ownership; shared-target write/flush faults with pre-retry remount, same-handle retry and ambiguous publication; U: replacement I/O faults | None |
| Symlink create/rename/unlink | [metadata.rs](metadata.rs): `symlink_namespace_preserves_target_through_metadata_rename_and_remount`, `symlink_publication_cuts_preserve_namespace_and_captured_target`, `symlink_refusals_and_short_reads_do_not_write` | U: exact opaque target, retained target, cuts and no-write refusals | None beyond the measured staged demand of three nodes, which four and eight pages exceed |
| Protection and preserved metadata | [metadata.rs](metadata.rs): `metadata_publication_cuts_preserve_exact_old_or_new_state_and_snapshot`, `metadata_refuses_open_windows_and_uncertain_publication_requires_remount`; [core tests/flight.rs](../../afsplus-core/tests/flight.rs): `api_snapshot_and_metadata_calls_preserve_captured_state_and_busy_refusals` | U: exact metadata/cuts/poison; P: protection and busy snapshot deletion with observed/unobserved image equality | None beyond the measured staged demand of one node, which every bounded profile admits |
| Full-COW write, sparse write | [streaming_api.rs](streaming_api.rs): `seeded_mixed_io_preserves_bytes_across_remounts_and_clones`; [crash_matrix.rs](crash_matrix.rs): `every_crash_state_of_a_sparse_write_is_atomic`; [tiny_cache_data_matrix.rs](tiny_cache_data_matrix.rs): full-COW cases and wide-fixture writes | U: independent mixed-byte oracle, sparse cuts; P: direct partial and full rewrite, sparse extension, tree overwrite and tree-to-direct rewrite with exact bytes, ranges, layout flag and captured snapshot at every modeled cut and write/flush fault, no-write refusals; spilled full-COW writes at 2/4/8 with faults, 2-page spilled cuts; zero spills at unlimited | None beyond the measured limits stated in [Residual data family matrix](#residual-data-family-matrix) |
| Bounded write, reservation initialization | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `bounded_writes_crash_to_exact_bytes_and_preserve_captured_zeros`, `reservation_write_crashes_preserve_old_zeros_or_complete_new_bytes`, `reservation_write_io_errors_preserve_old_logical_zeros_and_require_reconciliation`; [tiny_cache_data_matrix.rs](tiny_cache_data_matrix.rs): bounded-write cases and wide-fixture reservation writes | U: captured zeros, exact bytes, reconciliation; P: admitted budgets and block, result-record, zero and unbounded budget refusals with zero writes and retry; initialization at the reserved physical addresses; shared-reservation fallback to a fresh block with an unchanged peer; cuts, faults and captured snapshot; spilled bounded writes at 2/4, spilled reservation initialization at 2 (bounded and unbounded) and 8 (unbounded) with faults; 2-page spilled cuts and 4-page bounded-write cuts | None beyond the measured limits stated in [Residual data family matrix](#residual-data-family-matrix) |
| Preallocation / bounded reservation | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `bounded_reservation_refusal_and_boundary_retry_preserve_layout`, `bounded_reservation_tree_publication_preserves_snapshot_at_every_cut`; [tiny_cache_data_matrix.rs](tiny_cache_data_matrix.rs): preallocation cases and wide-fixture hole reservation | U: allocation layout, refusal/retry, snapshot cuts; P: exact ranges, bytes and size for direct past-EOF, tree-hole and reservation-tree past-EOF reservations at every cut and fault; block, record, invalid-limit, range-overflow, time, directory and `NoSpace` refusals plus empty and covered no-ops with zero writes and retry; spilled multi-leaf reservation at 2/4/8 with faults and 2-page cuts; headroom-violating and near-full preallocation refusals with retry after reclaim in [Low-space refusal and retry profiles](#low-space-refusal-and-retry-profiles) | None beyond the measured limits stated in [Residual data family matrix](#residual-data-family-matrix) |
| Truncate / sparse growth / bounded shrink | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `sparse_growth_publication_is_atomic_with_retained_reservations`, `bounded_shrink_crash_preserves_shared_and_captured_bytes`; [shared_crash.rs](shared_crash.rs): `truncate_across_private_and_shared_subruns_is_crash_atomic`; [tiny_cache_data_matrix.rs](tiny_cache_data_matrix.rs): size-change cases and wide-fixture shrink/growth | P: private/shared truncate cuts with independent survivor bytes; U: sparse growth, bounded shrink and retained-state cuts; P: direct-to-tree and tree sparse growth, bounded tree, direct and reservation shrink, tree-to-direct and direct-to-tree shrink transitions with exact bytes, ranges and captured snapshot at every cut and fault; retirement, window-record, zero-record, directory and time refusals plus a same-size no-op; spilled full and bounded shrink at 2/4/8 and growth at 2/4 with faults; 2-page spilled cuts and 4-page bounded shrink, full shrink and growth cuts | None beyond the measured limits stated in [Residual data family matrix](#residual-data-family-matrix) |
| In-place policy flag and private data | [data_policy_persistence.rs](data_policy_persistence.rs): `opt_in_persists_across_remount_and_takes_the_in_place_path`, `policy_flag_publication_cuts_preserve_exact_choice_and_bytes_in_all_profiles`, `policy_flag_io_failures_preserve_exact_choice_and_retry_in_all_profiles`, `policy_flag_ambiguous_publication_blocks_mutations_until_remount_in_all_profiles`, `policy_flag_applicable_refusals_issue_no_writes_and_preserve_state_in_all_profiles`, `retained_snapshot_forces_cow_for_flagged_private_writes_in_all_profiles`, `in_place_write_after_a_crash_keeps_metadata_clean`; [data_policy.rs](data_policy.rs): `in_place_crash_matrix_keeps_metadata_clean_but_allows_torn_old_data` | P: persistent flag, actual reuse, flag cuts, flag write/flush faults, ambiguous flag publication, no-write refusals, retained-snapshot COW fallback and both private-data tear matrices; opted-in in-place writes use the weaker torn-old oracle and never full-COW old/new bytes; [family_matrix_data_policy.rs](family_matrix_data_policy.rs): in-place write faults and ambiguous publication, retained-snapshot COW fallback, extending and shared-block fallback cuts and faults, staged demand of two nodes with 400 long names | Eviction of the in-place write's staged nodes, whose demand of two fits every bounded profile |
| CloneFile / CloneRange | [shared_crash.rs](shared_crash.rs): first-clone and aligned range-boundary profile tests, `first_clone_io_failures_preserve_ownership_in_all_profiles`; [faults.rs](faults.rs): `completed_checkpoint_write_and_adoption_read_errors_block_mutations`; [tiny_cache_matrix.rs](tiny_cache_matrix.rs): `clone_publication_cuts_preserve_snapshot_namespace_in_all_profiles`; [shared_clone.rs](shared_clone.rs): partial-boundary and refusal tests | P: first CloneFile and aligned CloneRange cuts with exact bytes/refcounts; CloneFile I/O and ambiguous-publication faults; retained-snapshot CloneFile cuts; U: unaligned range boundaries and range refusals | Forced eviction of CloneRange and of a write into a published clone, where the measured staged demand is one node, and of CloneFile above two pages, where it is three |
| Shared write/delete/final owner reuse | [shared_crash.rs](shared_crash.rs): `shared_write_split_is_crash_atomic`, `unlink_at_count_three_is_crash_atomic`, `unlink_at_count_two_never_reclaims_the_survivor`, `shared_storage_is_reused_only_after_the_last_owner_disappears`, `shared_write_io_failures_preserve_exact_ownership_and_allow_retry_in_all_profiles`, `shared_write_ambiguous_publication_requires_remount_in_all_profiles` | P: shared-write and reference-count cuts, exact survivor bytes, quarantine and final-owner storage reuse; shared-write write/flush faults with retry and ambiguous publication | None beyond the measured staged demand of one node for the write and three for the unlinks |
| Orphan setup/move/open-target replace/cleanup | [orphans.rs](orphans.rs): `every_orphan_lifecycle_checkpoint_cut_recovers_to_an_allowed_state`, `every_open_target_replace_cut_is_old_or_new_namespace`, `fragmented_orphan_cleanup_is_extent_bounded_and_resumes_after_remount` | P: insertion/update/cleanup and open-target replace cuts with the preparatory orphan-directory checkpoint as an allowed state, retry from every cut, extent-bounded cleanup with exact tail-trimmed bytes; [family_matrix_orphans.rs](family_matrix_orphans.rs): insertion and cleanup faults with same-handle retry, ambiguous publication, retained snapshots, forced eviction with sampled cuts, insertion under exhausted ordinary allocation | None beyond the unreachable `NoSpace` refusal of insertion |
| Deferred namespace/write/truncate fsync | [cache_profiles.rs](cache_profiles.rs): `durable_window_recovery_honors_the_mount_profile`; [intent_log.rs](intent_log.rs): `existing_file_write_and_truncate_replay_in_order`, `successive_existing_writes_recover_only_monotone_prefixes`; [family_matrix_deferred.rs](family_matrix_deferred.rs): three-group namespace, write and truncate families | P create replay, spills and idempotence; U data/truncate ordering; P acknowledged-prefix oracles over three durable groups per family, cuts inside and after every intent-group publication, exhaustive or seeded recovery cuts with a second recovery, the recovery fault matrix and retained snapshots | Forced eviction of the deferred namespace, write and truncate recovery commits; a resource refusal of `window_write_file_at` and of `window_truncate_file` |
| Deferred cancellation admission/ownership | [intent_log.rs](intent_log.rs): `cancellation_preflight_refusals_preserve_the_pending_window`, `cancellation_preflight_read_failure_keeps_the_window_retryable`; [family_matrix_deferred.rs](family_matrix_deferred.rs): `window_refusals`, `window_group_limit`, mixed-window family | P: acknowledged/unlogged ownership, no-write refusal and read-failure retry; P: 21 refusal and read-failure paths of the four mutating entry points and of a no-changes mount with zero writes and flushes, the full-log `window_fsync` refusal, and cut oracles over windows that mix namespace, write and truncate work | Read failures inside `window_fsync` and `window_commit`; forced eviction of a mixed window's recovery commit |
| Shared deferred replay / orphan replay | [shared_crash.rs](shared_crash.rs): `durable_shared_unlink_replay_is_crash_atomic_and_idempotent`, `logged_write_replay_splits_shared_data_and_survives_replay_crashes`; [intent_replay_orphans.rs](intent_replay_orphans.rs); [family_matrix_replay.rs](family_matrix_replay.rs): six replay families, `deferred_refusal`, `repeated_recovery` | P: shared unlink/replacement/write replay cuts, exact survivor/orphan bytes and repeat-remount idempotence; U: other orphan branches; P: the final-link delete, that delete under consumed ordinary allocation and over a wide root with spills, a write paired with a final delete, a replacing rename, a sixteen-file group, the shared unlink and the shared write, each with logging cuts, seeded recovery campaigns, the recovery fault matrix, retained snapshots and a no-write `NoSpace` refusal with its corrective step | Faults injected during the logging phase; forced eviction of the shared replay and replacing-rename families |
| Snapshot registry create/delete | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `snapshot_create_and_delete_publication_cuts_preserve_exact_membership_and_bytes`, `uncertain_snapshot_publication_blocks_mutation_and_remount_resolves_membership`; [family_matrix_snapshot.rs](family_matrix_snapshot.rs): registry create and delete families, `admission_limits` | U: membership/bytes/cuts/poison; P normal create/delete and busy refusal in flight test; P: membership, live and captured bytes at every modeled cut, a fault at every recorded write and flush with same-handle retries, both ambiguous modes, a wide-root create whose measured demand is two nodes, and the admission-limit, busy and missing-identity refusals with zero writes and flushes | An exhausted registry identity space, which needs a planted allocator record |
| Snapshot lifetime / maintenance / selectable older view | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `constrained_tree_profiles_preserve_snapshots_and_shared_survivors`, `last_snapshot_deletion_preserves_older_selectable_view_during_the_next_write`; [family_matrix_snapshot.rs](family_matrix_snapshot.rs): maintenance and release families, `previous_slot_protection` | P: 192-entry spilled batch, exact captured bytes, clone survivor, both checkpoints; U: last-view deletion safety; P: maintenance-step and last-view release cuts, faults at every write and flush, both ambiguous modes, and the unreadable previous-slot registry refusing an optional in-place write with the exact old bytes preserved | Forced eviction of the maintenance and release commits; a maintenance pass whose ledger scan wraps |
| Snapshot mount/recovery | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `public_snapshot_mount_recovery_cuts_preserve_acknowledged_live_and_historical_bytes`, `public_snapshot_mount_validates_admission_before_pending_recovery_writes`; [family_matrix_snapshot.rs](family_matrix_snapshot.rs): `mount_recovery`, `admission_before_recovery` | U: exact acknowledged live/historical state, bounded admission; P: two durable groups over a captured file with logging cuts, exhaustive recovery cuts, a second recovery of every image, the recovery fault matrix, and 20 invalid-admission refusals across the four mount modes behind a device that refuses every write and flush | Forced eviction of the snapshot-aware recovery commit |
| Reclaim queue seal/consume/cursor and allocation rotation | [reclaim.rs](reclaim.rs): `crash_matrix_over_a_sealing_transaction`, `crash_matrix_over_segment_consumption_and_disappearance`, `crash_matrix_over_a_mid_run_cursor_advance`; [core src/volume.rs](../../afsplus-core/src/volume.rs): `allocation_cache_keeps_spilled_nodes_across_checkpoint_rotation` | P: sealing/consumption/cursor cuts with literal free/pending accounting and namespace bytes; P: allocation rotation with a retained snapshot and a shared run, exact live/captured/per-checkpoint bytes, spills at 2/4/8; [family_matrix_reclaim.rs](family_matrix_reclaim.rs): reclaim-step faults with same-handle retry, ambiguous publication and retained snapshot, promotion across eight allocation-root leaves with spills at 2/4/8 and sampled cuts, spilled create batch with two allocation-root nodes and sampled cuts | The mid-run cursor transition, whose fixture shape is outside the family driver's persistent-snapshot format, where the recorded step promotes no block |
| Low-space refusal and progress | [allocation_pressure.rs](allocation_pressure.rs): `near_full_enospc_publishes_nothing_and_delete_can_recover_space`, `repeated_near_full_cow_and_reclaim_preserve_shared_survivors`, `near_full_delete_survives_every_modeled_power_cut` | P: deterministic no-write ENOSPC, same-size retry after reclaim, near-full COW/reclaim cycles with shared survivors; U: near-full delete cuts; [family_matrix_low_space.rs](family_matrix_low_space.rs): P near-full delete cuts, faults and ambiguous publication, forced eviction with two-page spills, ENOSPC of a spilled 64-file batch with unreachable provisional writes and a reclaim-drained retry | Eviction at four and eight pages, above the three-node demand |
| COW tree spill/reload component | [core src/cow_tree.rs](../../afsplus-core/src/cow_tree.rs) staged-tree tests | 2/4/8 component: staged-node bounds/spill/reload | Total-heap limits |
| Common uncertain commit tail | [faults.rs](faults.rs): `uncertain_checkpoint_publication_requires_remount_before_more_writes`, `completed_checkpoint_write_and_adoption_read_errors_block_mutations`; [core tests/flight.rs](../../afsplus-core/tests/flight.rs) | U semantic poisoning/reconciliation; P observed/unobserved failure equality; P family-matrix ambiguous modes for the in-place write, orphan insertion and cleanup, reclaim step and near-full delete with exact live, remounted and captured state | The staged directory delete, the three baseline symlink publication paths, the deep split, the deep cross-directory rename, the single create, mkdir and rmdir paths, the three symlink paths at the maximum target length, orphan cleanup with ordinary allocation exhausted, and the four reload-failure fixtures |

## Scope and interpretation

Normal mount is not an exhaustive integrity oracle. Pair remounted semantics
with `check_device`; snapshot ownership tests additionally verify retained views
and selectable checkpoints. A clean mount alone does not prove publication order.

The cut model enumerates full-write subsets of unflushed tails up to twelve
writes and representative single prefix tears. Larger spill transactions need
an explicitly bounded fixture or a separately identified sampled campaign.
Successful P execution alone does not prove eviction: require nonzero spill
counters and bounded resident staged nodes. A no-write resource refusal must
preserve previously acknowledged state and permit the specified retry.

Deferred recovery, orphan cleanup and snapshot creation can publish more than
one checkpoint. Their permitted states must follow the acknowledged-prefix or
operation-specific protocol, not an assumed universal one-generation rule.
Native cache/VM behavior, total process memory, aged workloads and hardware
power-loss qualification belong to later gates.

## Added executable matrix

All entries below use [tiny_cache_matrix.rs](tiny_cache_matrix.rs). The namespace
families are single create, mkdir, rmdir, hard link, cross-directory rename,
replace rename, unlink, symlink create/move/unlink, protection and metadata restore.
Each case starts from its own cloned durable image; the cache profile is applied
before mounting and replay, not changed after recovery.

| Test | Profiles | Oracle | Deliberate limit |
|---|---|---|---|
| `namespace_and_metadata_preserve_exact_live_and_retained_state_in_all_profiles` | P | Exact root and child entries, IDs for existing objects, link counts, file bytes, opaque symlink targets, metadata fields, complete paginated captured names with EOF checks, captured metadata/bytes, remount, selected and older-checkpoint checker | Small trees; not an eviction qualification |
| `namespace_and_metadata_cuts_two_pages`, `namespace_and_metadata_cuts_four_pages`, `namespace_and_metadata_cuts_eight_pages`, `namespace_and_metadata_cuts_unlimited` | P | Every modeled write/flush cut per family; exact permitted generation/namespace/content and invariant captured view; both outcomes required | Bounded full-write-subset/single-tear model, not native durability or large split/collapse |
| `namespace_and_metadata_write_and_barrier_failures_preserve_ownership` | P | Failure at every recorded write and flush; exact old state before publication, successful retry or explicit remount requirement; final-barrier poisoning; exact state after reconciliation | FaultBackend fails before writes; completed-but-reported-failed writes and adoption-read errors keep separate baseline coverage |
| `namespace_and_metadata_refusals_issue_no_writes_and_preserve_retained_bytes` | P | Duplicate names, occupied rename target, invalid symlink target/type, invalid metadata time, busy snapshot deletion; zero writes/flushes, unchanged live/captured state, successful subsequent create | Refusal list is explicit, not every invalid API argument |
| `provisional_spill_failures_retry_at_two_four_and_eight_pages` | 2/4/8 | 192-entry batch proves more than 31 spills and staged-node bounds; write indices 0/1/7/31 fail, preserve empty committed namespace, retry and remount all payload bytes, checker | Early spill faults only; unlimited has no eviction; no claim of large-tail exhaustive cuts |
| `bounded_reservation_refusal_preserves_layout_and_retries_in_all_profiles` | P | One-record budget refuses a fragmented local edit with no writes; exact original metadata/allocation enumeration and zero bytes; admitted retry adds one block; exact captured bytes and checker after remount | Short sparse fixture; no large-layout or whole-job memory claim |
| `acknowledged_write_truncate_recovery_is_restartable_in_all_profiles` | P | Durable write+truncate group; every replay cut exposes pre/post checkpoint, recovery preserves exact acknowledged and historical bytes; second recovery is idempotent | One combined group, no shared clone/orphan replay or interrupted fsync-tail matrix |

The checker wrapper rejects errors and older-checkpoint warnings. Only the
standard stopped intent-log tail diagnostic is admissible as a crash artifact.
Live content and snapshot bytes have independent literal/slice expectations;
a readable mount or equality with another writer execution is insufficient.

## CloneFile first-sharing profile gate

In [shared_crash.rs](shared_crash.rs), `first_clone_is_crash_atomic` and
`first_clone_two_pages`, `first_clone_four_pages`, `first_clone_eight_pages`
apply explicit cache profiles to setup, recorded CloneFile and recovery.
Each modeled cut preserves source bytes and exposes either no clone/no sharing
root, or the complete clone with a two-block run referenced twice. The selected
generation must match that outcome, and both outcomes must occur.
The fixture does not force eviction or retain a snapshot; error-return retries
and those resource combinations require separate qualification.

`first_clone_io_failures_preserve_ownership_in_all_profiles` injects a
before-write error at every recorded write and an error at every flush under
all four profiles. A final-barrier error must poison further mutation. Earlier
failures preserve the live source, absent clone and old generation. After
remount, the exact old state must admit retry or the complete new state must
already be present; the clone and source have identical expected bytes and one
two-block/two-reference shared run. The checker runs before and after retry.
This error model does not cover writes completed but reported failed or
adoption-read errors, and the fixture does not force a tree spill.

In [faults.rs](faults.rs),
`completed_checkpoint_write_and_adoption_read_errors_block_mutations` crosses
create/CloneFile, all four cache profiles and two ambiguous-publication faults:
a completed checkpoint write returning an error, or reads failing after
publication. Further mutation must return `WindowPoisoned`; remount must expose
the complete new file and unchanged source. Clones require one shared run with
two references. A subsequent independent mutation must preserve both files and
pass the checker. This uses a memory device, not native durability evidence.

## CloneRange reference-boundary profile gate

In [shared_crash.rs](shared_crash.rs),
`clone_range_with_multiple_reference_boundaries_is_crash_atomic` and
`clone_range_boundaries_two_pages`, `clone_range_boundaries_four_pages`,
`clone_range_boundaries_eight_pages` apply the selected cache profile to fixture
creation, transaction recording and every recovered image. Each modeled cut
must expose exactly the old or new generation and destination contents, retain
source and peer contents, and match the expected shared-run counts and lengths.
Both outcomes must occur; the exhaustive checker validates each cut image.

This fixture covers aligned range cloning across multiple reference-count
boundaries. It does not establish eviction, unaligned boundary copying,
retained snapshots, explicit I/O-error retry or CloneFile profile coverage.
Those combinations retain their separate requirements in the baseline matrix.

## Retained snapshot during CloneFile publication

`clone_publication_cuts_preserve_snapshot_namespace_in_all_profiles` in
[tiny_cache_matrix.rs](tiny_cache_matrix.rs) creates a persistent snapshot before
cloning, then enumerates modeled publication cuts under 2/4/8/unlimited caches
with an explicit sixteen-write full-subset budget.
The live namespace must match the selected old/new generation, source bytes
remain exact and a published clone has identical contents. The captured source
metadata and bytes remain exact, the historical root lists only the source,
and the clone object is absent from the snapshot. The checker also rejects
older-checkpoint warnings. This fixture does not force eviction or test mutation
of the clone after publication.

## Shared ownership transitions and replay

The shared-write, count-three/count-two unlink, private/shared truncate,
shared-target replacement and final-owner reuse fixtures in
[shared_crash.rs](shared_crash.rs) apply explicit 2/4/8/unlimited profiles to
setup, recording and cut-image mounts. Exact bytes, permitted generations,
reference ownership and quarantine checks remain operation-specific.

Shared unlink, replacement and write replay apply the same profile to logging,
recovery, no-changes inspection and subsequent remounts. Their oracles include
acknowledged survivor/orphan bytes and repeat-recovery generation stability.
Replay begins after successful fsync; interrupted fsync, explicit errors/retry,
retained snapshots, forced eviction and resource refusals remain separate.

## Reclaim queue profiles

The three crash matrices in [reclaim.rs](reclaim.rs),
`crash_matrix_over_a_sealing_transaction`,
`crash_matrix_over_segment_consumption_and_disappearance` and
`crash_matrix_over_a_mid_run_cursor_advance`, apply 2/4/8/unlimited from the
first fixture mount through recording and every recovered image, asserting the
effective profile on each mount. Every image passes the checker with no warning,
because these fixtures format no intent log, and selects the pre or post
generation. Free/pending accounting matches literals in the writer and in every
image: (222, 11) and (216, 14) for sealing, (205, 20) and (213, 13) for
consumption, (227, 12) and (228, 11) for the cursor step. Namespace entries and
file bytes are literals; each cursor image also drains to two pending blocks
and the empty steady-state free count. Both outcomes occur in every profile.

Modeled images per profile: 1,171 sealing, 68 consumption and 68 cursor; 5,228
in total. The recorded transactions issue zero spill writes and keep resident
staged nodes within the profile, so these matrices make no eviction claim.
Negative controls with wrong payload bytes, pending count and free count each
fail their matrix.

## Allocation-cache rotation with retained and shared bytes

`allocation_cache_keeps_spilled_nodes_across_checkpoint_rotation` in
[core src/volume.rs](../../afsplus-core/src/volume.rs) keeps the 1,024
allocation regions and forced rover movement of the original regression and
adds a persistent snapshot plus one two-block run shared by a source file and
two clones. Profiles 2/4/8/unlimited apply from fixture creation through every
remount. Each of three rounds writes 32 literal bytes across the source's first
block boundary, which moves the source off the run (three references become
two), then publishes a 256-entry batch with long names that rotates the write
checkpoint into the older slot.

After each batch and again after a profile remount, both selectable checkpoints
must match the cached allocation-root block sets when a cache exists, pass the
verifier's committed-state load and full sweep, hold exactly one two-block
two-reference run at the fixture's physical start, and expose the literal
source and peer bytes through that checkpoint's own object-map root and
generation. Live bytes and the snapshot's captured bytes of all three objects
stay exact, and every batch entry resolves after remount. Every bounded batch
spills: 111, 42 and 26 spill writes at 2, 4 and 8 pages over three rounds;
unlimited records zero. The test runs inside the core crate, so it calls the
verifier functions on both checkpoints directly; `check_device` lives in
afsplus-check. Deliberate limits: one shared run, three rounds, and no modeled
cuts of the spilled batch. A wrong captured-byte expectation fails.

## Data-update policy profiles

Every test in [data_policy.rs](data_policy.rs) and
[data_policy_persistence.rs](data_policy_persistence.rs) applies
2/4/8/unlimited from fixture creation to every remount and asserts the
effective profile. Checker calls reject warnings other than a stopped
intent-log tail. The looped functional tests cover the full-COW default,
single- and multi-block reuse, the metadata error after an in-place data write,
extending and shared fallbacks with literal source and clone bytes, flag
clearing across remount, feature-off and directory refusals, the planted-flag
corruption, hard links and clone destinations.

| Test | Oracle | Modeled count | Deliberate limit |
|---|---|---|---|
| `policy_flag_publication_cuts_preserve_exact_choice_and_bytes_in_all_profiles` | Enable and clear; each image selects the old or new flag with the full object record apart from flag and change time, literal bytes and one root entry; both outcomes | 197 images per direction and profile; 1,576 | Zero spill writes |
| `policy_flag_io_failures_preserve_exact_choice_and_retry_in_all_profiles` | Failure at every recorded write and flush; exact old choice before publication; poisoning after a failed checkpoint write or final barrier; exact pre-retry remount state, retry and checker | 72 injected faults; 56 separate same-handle retries | Before-write fault model |
| `policy_flag_ambiguous_publication_blocks_mutations_until_remount_in_all_profiles` | Completed checkpoint write or adoption-read error; the same and an independent mutation refuse with no writes or flushes; remount exposes the new choice; an idempotent retry keeps the generation | 16 cases | Memory device |
| `policy_flag_applicable_refusals_issue_no_writes_and_preserve_state_in_all_profiles` | Feature off, directory, symlink, missing object, invalid time, open window, read-only and no-changes mounts; zero writes and flushes, unchanged generation, window and record; opt-in succeeds afterwards where the feature exists | 32 refusals | Explicit refusal list |
| `retained_snapshot_forces_cow_for_flagged_private_writes_in_all_profiles` | A flagged private write under a retained snapshot overwrites no block in place; exact live bytes; captured metadata and bytes with an EOF sentinel after flag clearing and remount; no checker warning | 4 profiles | One partial-block range |
| `in_place_write_after_a_crash_keeps_metadata_clean`, `in_place_crash_matrix_keeps_metadata_clean_but_allows_torn_old_data` | Persistent and runtime in-place writes: the new generation holds literal new bytes; the old generation keeps bytes outside the range and only old or new values inside, with a torn-old image required; layout metadata unchanged | 203 images per profile for each test; 1,624 | ADR-062 weaker data contract |

Recorded policy transactions issue zero spill writes, which limits these tests
to profile coverage. One wrong expectation per new fixture family, seven in
total, fails its test.

## Low-space refusal and retry profiles

`near_full_enospc_publishes_nothing_and_delete_can_recover_space` and
`repeated_near_full_cow_and_reclaim_preserve_shared_survivors` in
[allocation_pressure.rs](allocation_pressure.rs) apply the profile to every
mount and remount. The first test refuses a headroom-violating and a near-full
preallocation with zero writes and flushes and unchanged generation, free count
and metadata. After delete and bounded reclaim it retries a preallocation of
the refused size, writes literal bytes, and requires the exact namespace and
bytes after a checker pass and a profile remount.

The second test runs 24 cycles per profile, 96 in total. Each near-full refusal
issues zero writes and flushes with unchanged generation, free count and size.
A boundary-crossing write to a clone is admitted or refused under pressure;
after delete and reclaim the same write is retried, and keeper and clone bytes
stay exact through the checker and a remount. Zero spill writes are observed.
`near_full_delete_survives_every_modeled_power_cut` runs at the default
unlimited profile. Wrong retry bytes and a nonzero refusal write count each
fail their test.

## Shared write and replacement failures

`shared_write_io_failures_preserve_exact_ownership_and_allow_retry_in_all_profiles`
and
`shared_replace_io_failures_preserve_exact_ownership_and_allow_retry_in_all_profiles`
in [shared_crash.rs](shared_crash.rs) fail every recorded write and flush of a
shared-extent write and of a replacement over a shared target at
2/4/8/unlimited: 13 faults per shared-write profile and 12 per replacement
profile, 100 in total. Before publication the live volume keeps the literal
source, peer and incoming bytes, one four-block two-reference run and no
quarantined shared block. A failed checkpoint write or final barrier poisons
further mutation. The pre-retry remount exposes the exact old state or the
published state; retry publishes the literal split runs (one block, then two
blocks after the rewritten block) or removes the shared record when the
replacement leaves one owner. A separate failure instance qualifies 84
same-handle retries.

`shared_write_ambiguous_publication_requires_remount_in_all_profiles` and
`shared_replace_ambiguous_publication_requires_remount_in_all_profiles` cover
16 completed-write and adoption-read cases: the same and an independent
mutation refuse without writes or flushes, remount exposes the published state,
and a later private write preserves every shared survivor. Checker calls reject
warnings other than a stopped intent-log tail. Zero spill writes are observed,
and these fixtures hold no retained snapshot. Wrong peer bytes and a wrong
namespace size fail all four tests.

## Orphan lifecycle profiles

`every_orphan_lifecycle_checkpoint_cut_recovers_to_an_allowed_state`,
`every_open_target_replace_cut_is_old_or_new_namespace` and
`fragmented_orphan_cleanup_is_extent_bounded_and_resumes_after_remount` in
[orphans.rs](orphans.rs) apply 2/4/8/unlimited to setup, recording and each
recovered image. Orphan insertion and open-target replacement publish two
checkpoints, and generation +1, the preparatory orphan directory, is an allowed
state: object 2 is absent from the selected object map at +0 and present at +1,
with the visible namespace and bytes unchanged at both. At +2 the application
name is hidden and object 2 owns the literal bytes. Every image retries to the
final state and passes the checker, which admits only a stopped intent-log tail
warning. Update cuts expose literal old or new orphan bytes. Cleanup cuts expose
the old layout, the empty tail-trimmed layout or the removed object, and a
repeated cleanup of the absent object publishes nothing.

Modeled images per profile: insertion 342/2,218/4, update 199/4, cleanup
193/345/4 and replacement 342/4,299/4, which is 7,954 per profile and 31,816 in
total. Negative controls swap the preparatory-directory expectation in the
lifecycle and replacement matrices; both fail.

The fragmented fixture writes five one-block extents at logical blocks 0, 2, 4,
6 and 8 with bytes 1 to 5 and cleans with a two-extent budget.
[ADR-066](../../../adr/ADR-066-bounded-orphan-directory.md) removes whole
extent records from the logical end and publishes the smaller file; the
published size is the logical start of the lowest removed extent
(`cleanup_orphan_data_step` in [core src/volume.rs](../../afsplus-core/src/volume.rs)).
The literal prefixes are therefore six blocks (1, hole, 2, hole, 3, hole) with
three allocated blocks, then two blocks (1, hole) with one allocated block, then
object removal, with a checker pass and a profile remount after each step. A
five-block first prefix fails. Spill counters are not observed in orphans.rs;
I/O faults, ambiguous publication, retained snapshots and resource refusals lie
outside these fixtures.

## Family matrix driver

[common/family_matrix.rs](common/family_matrix.rs) serves the
`family_matrix_*.rs` tests through `mod common;`. A family implements the
`Family` trait: the image format, fixture setup per variant, the operation,
the number of checkpoints the operation publishes, and `verify`, which asserts
the exact state after a given number of publications with literal names,
bytes and accounting. A family may add success assertions, a literal
staged-node demand, a refusal predicate and a corrective step before retry.
`profile_tests!` generates `two_pages`, `four_pages`, `eight_pages` and
`unlimited` tests inside one module per family part.

Every image is formatted with persistent snapshots. The explicit profile
applies from fixture setup through recording and every recovered mount, and
each mount asserts the effective profile. The checker wrapper rejects errors
and every warning except a stopped intent-log tail.

| Part | Driver behavior |
|---|---|
| Recording | Applies the operation to the verified fixture, requires the generation to advance by the declared publications, bounds resident staged nodes by the profile, verifies the live state, the remounted state and the checker, and reports writes, flushes, longest unflushed tail and spill counters |
| Cuts | Enumerates every modeled cut within an explicit tail budget; each image passes the checker, selects a generation between the fixture and the final publication and matches `verify` for that delta; intermediate publications retry to the final state; the fixture and final outcomes must both occur |
| Faults | A before-write fault at every recorded write and a failure at every flush. A fault on a checkpoint-slot write or on the flush that follows it must poison the handle with `WindowPoisoned`; any other fault leaves the exact published state on the live handle. Remount exposes exactly the publications that reached media before the fault, passes the checker and retries to the final state; a separate instance retries on the same handle after every certain fault |
| Ambiguous publication | The first checkpoint write completes and returns an error, or reads fail after it. The operation reports an error; the same operation and an independent create return `WindowPoisoned` with no writes or flushes; remount exposes exactly one publication, and multi-publication operations retry to the final state |
| Retained snapshot | A snapshot taken before the operation, by the driver after setup or by the family inside setup, holds literal bytes; its captured metadata and bytes, read with an EOF sentinel, are compared through recording, every cut, every fault case and both ambiguous modes |
| Forced eviction | A pre-populated fixture whose final commit has a literal staged-node demand, asserted at the unlimited profile with zero spills. A bounded profile below the demand must report nonzero spill writes, and a profile at or above it reports zero. Faults cover every write, spill images included; cuts run when the family declares a budget |
| Resource refusal | The refusal fixture must return the family's error with no flush and no write to a block reachable from either selectable checkpoint, and with no write at all unless the family admits provisional spill images. The fixture state holds on the live handle and after remount; the family's corrective step then admits the retry, verified live, after remount and through the checker |

The runners are `plain` (recording, cuts and faults), `retained` (recording,
cuts, faults and ambiguous publication), `eviction`, `eviction_recorded`
(spill evidence and the verified published state alone, for a transaction
whose write count puts the fault matrix beyond the per-test time budget),
`ambiguous` and `refusal`. Every cut budget is explicit, and an unflushed tail beyond it fails
the test.

## Replay runner of the family matrix

A `ReplayFamily` in [common/family_matrix.rs](common/family_matrix.rs) makes
its work durable through intent-log fsync groups, and the recovery transaction
of the next mount publishes it. The family supplies the image (with log slots),
the fixture per variant, the number of groups, how it logs one group, and
`verify`, which asserts the exact state after a given number of acknowledged
groups. The profile applies from formatting through logging, every inspection
mount and every recovery mount, and each mount asserts the effective profile.

| Part | Driver behavior |
|---|---|
| Logging | Logs every group on a recording backend, requires each group to leave no unlogged operation and to publish no checkpoint, and requires a no-changes mount of the logged image to report exactly the family's group count |
| Recording | Records the recovery transaction of the logged image, requires one published checkpoint and zero pending records, verifies the recovered state, the captured objects, the remount and the checker, and reports writes, flushes, the unflushed recovery tail and the spill counters |
| Logging cuts | Every modeled cut of the logging phase within an explicit budget, including cuts inside an intent-group publication. The acknowledged record count of the image names the allowed state: the image keeps the fixture generation, and a recovery and a second recovery reach exactly `verify` for that count. The zero-group and all-group outcomes must both occur |
| Recovery cuts | Every modeled cut of the recovery transaction within an explicit budget, or the seeded campaign of `recovery_sampled_cuts` for a longer tail. Each image selects the pre- or post-publication checkpoint with the matching pending-record count, and a recovery and a second recovery reach the complete acknowledged state with a stable generation |
| Recovery faults | A before-write fault at every recovery write and a failure at every recovery flush. The interrupted mount must report the error; the media then holds the writes recorded before the fault, and a later recovery and a second recovery reach the complete acknowledged state |
| Eviction | A pre-populated fixture whose recovery commit has a literal staged-node demand, asserted at the unlimited profile with zero spills; a profile below the demand must report spill writes |

The runners are `replay` (logging cuts and recovery cuts), `replay_with_faults`
and `replay_sampled` (logging cuts, the seeded recovery campaign and the fault
matrix). `recovery_sampled_cuts` visits every in-order write prefix of the
recovery log, each write torn at bytes 64, 2,048 and 4,064 after its in-order
prefix, and, per flush segment, every full-write subset when the segment has at
most `sample` of them and otherwise `sample` subsets drawn by a SplitMix64
generator from the seed named in the test. A sampled campaign qualifies the
drawn subsets together with the complete prefix and tear sets.

## In-place data policy family matrix

[family_matrix_data_policy.rs](family_matrix_data_policy.rs) drives three
families at 2/4/8/unlimited with a 12-write cut budget; 24 tests pass.

| Family and part | Oracle | Modeled count per profile | Deliberate limit |
|---|---|---|---|
| Opted-in private write, cuts and faults | The published generation holds literal new bytes; an older generation keeps literal old bytes outside the written range, holds only old or new values inside it, and keeps the in-place layout; one in-place block on success | 352 images; 12 faults (9 writes including the in-place data block, 3 flushes) and 10 same-handle retries | ADR-062 weaker data contract |
| Opted-in private write, ambiguous publication | Both modes poison with no I/O and remount to the published bytes | 2 cases | Memory device |
| Opted-in private write, retained snapshot | The write falls back to full COW with zero in-place blocks; exact old bytes at every older generation; captured metadata and bytes exact | 632 images, 13 faults, 11 same-handle retries, 2 ambiguous cases | One partial-block range |
| Opted-in private write, forced eviction | 400 long root names; at the unlimited profile the write stages exactly two nodes (object-map root and leaf) with zero spills | 632 images, 13 faults, 11 same-handle retries | A two-node demand fits every bounded profile, so this variant records the demand and zero spills; no eviction occurs |
| Extending-write fallback | Exact old or new bytes including the extension, zero in-place blocks | 632 images, 13 faults, 11 same-handle retries | Plain fixture |
| Shared-block fallback | Exact old or new source bytes, clone bytes unchanged, zero in-place blocks | 1,171 images, 14 faults, 12 same-handle retries | Plain fixture |

Wrong new-byte literals for each of the three families fail their test.

## Sampled cut campaigns for oversized eviction tails

`eviction_sampled` in the driver serves a forced-eviction fixture whose
unflushed tail exceeds the twelve-write exhaustive budget. After the spill
evidence and the fault matrix, `sampled_cuts` visits every in-order write
prefix of the recorded log, each write torn at bytes 64, 2,048 and 4,064 after
its in-order prefix, every full-write subset of each flush segment of at most
twelve writes, and, for each longer segment, 256 subsets drawn by a SplitMix64
generator from the seed named in the test. Every image passes the checker and
the family's exact-state oracle, and both the fixture and the final
publication must occur. The test output reports the seed, sample size,
prefix, tear, exhaustive and sampled counts per profile. A sampled campaign
qualifies the drawn subsets together with the complete prefix and tear sets;
every reordering of a longer segment stays outside its evidence.

## Orphan family matrix

[family_matrix_orphans.rs](family_matrix_orphans.rs) drives orphan insertion
and budgeted cleanup at 2/4/8/unlimited with a 12-write cut budget; 36 tests
pass. Both operations publish two checkpoints, so every image selects delta
0, 1 or 2 and intermediate images retry to the final state.

| Family and part | Oracle | Modeled count per profile | Deliberate limit |
|---|---|---|---|
| Insertion, cuts and faults | Payload bytes exact at every generation; the application name visible below delta 2; the preparatory orphan directory (object 2) present from delta 1; orphan count and orphan flag set only at delta 2; literal root entry count | 4,925 images (622/4,299/4); 25 faults (21 writes, 4 flushes) and 21 same-handle retries | Plain 512-block fixture |
| Insertion, retained snapshot and ambiguous publication | The snapshot taken before insertion keeps the payload's captured metadata and bytes through every image; both ambiguous modes poison with no I/O and remount to exactly one publication, then retry to the final state | 4,925 images, 25 faults, 21 same-handle retries, 2 ambiguous cases, plus 2 plain ambiguous cases | Memory device |
| Insertion with ordinary allocation exhausted | A pressure file reserves the largest range ordinary allocation admits and a one-block create returns `NoSpace`; insertion then succeeds with the same oracle plus the pressure file present | 4,925 images, 25 faults, 21 same-handle retries | Orphan insertion reaches no `NoSpace` refusal with ordinary allocation exhausted, so this variant qualifies cuts and faults under exhaustion |
| Insertion, forced eviction | 400 long root names; the root-directory and orphan-directory paths stage three nodes at unlimited with zero spills; two pages must spill; faults at every write and the sampled cut campaign with seed `0x5eed0003` | Spill writes 2/0/0/0 and peak staged nodes 2/3/3/3 at 2/4/8/unlimited; 30 faults (26 writes, 4 flushes) and 26 same-handle retries; sampled campaign of 877 images (27 prefixes, 78 tears, 516 exhaustive subsets, 256 sampled subsets; outcomes 551/322/4) | Four and eight pages exceed the three-node demand and report zero spills |
| Cleanup, cuts and faults | A three-block file with blocks 0 and 2 written and a hole between, orphaned, cleaned with an extent budget of 2: delta 0 keeps exact bytes and two allocated blocks, delta 1 exposes the empty tail-trimmed file with zero allocated bytes, delta 2 removes the object; the orphan directory stays present; orphan count and flag clear at delta 2 | 1,251 images (622/625/4); 22 faults (18 writes, 4 flushes) and 18 same-handle retries | Two extent records |
| Cleanup, retained snapshot and ambiguous publication | The snapshot taken before cleanup keeps the fragmented file's captured bytes through every image, including after the object leaves the live view; both ambiguous modes as above | 1,251 images, 22 faults, 18 same-handle retries, 2 ambiguous cases, plus 2 plain ambiguous cases | Memory device |
| Cleanup, forced eviction | 400 long root names; extent-map, object-map and orphan-directory paths stage three nodes at unlimited with zero spills; two pages must spill; exhaustive cuts with a 10-write budget and faults at every write | Spill writes 1/0/0/0 and peak staged nodes 2/3/3/3; 3,383 images (1,161/2,218/4) within the 10-write budget; 25 faults (21 writes, 4 flushes) and 21 same-handle retries | Four and eight pages exceed the three-node demand and report zero spills |

Wrong payload bytes, a wrong orphan count and a wrong allocated-byte literal
fail their tests.

## Reclaim family matrix

[family_matrix_reclaim.rs](family_matrix_reclaim.rs) drives a reclaim step
and a spilled metadata batch at 2/4/8/unlimited; 20 tests pass.

| Family and part | Oracle | Modeled count per profile | Deliberate limit |
|---|---|---|---|
| Reclaim step, cuts and faults | 256-block image with reclaim capacities of 4 inline entries, 3 segment and 8 table references; a three-block file is deleted and a two-block step runs; literal free/pending accounting (223, 7) before and (223, 14) after, empty root | 115 images (111/4); 8 faults (6 writes, 2 flushes) and 6 same-handle retries; 2 ambiguous cases | Single-region fixture |
| Reclaim step, retained snapshot | The snapshot taken between creation and deletion keeps the three captured blocks; accounting (221, 9) before and (222, 8) after; both ambiguous modes | 68 images (64/4); 7 faults (5 writes, 2 flushes), 5 same-handle retries, 2 ambiguous cases | Single-region fixture |
| Reclaim step, forced eviction | 16,384 blocks in 16-block regions, so 1,024 regions span eight allocation-root leaves; eight preallocated spans are deleted and one step with a 65,536-block batch promotes them; accounting (969, 600) before and (1,478, 172) after; promotion stages nine nodes at unlimited with zero spills, and 2/4/8 pages must spill; faults at every write and the sampled cut campaign with seed `0x5eed0001` | Spill writes 8/5/1/0 and peak staged nodes 2/4/8/9 at 2/4/8/unlimited; 140 faults (138 writes, 2 flushes) and 138 same-handle retries; sampled campaign of 811 images (139 prefixes, 414 tears, 2 exhaustive subsets, 256 sampled subsets of the 137-write tail; outcomes 807/4) | Sampled subsets for segments beyond twelve writes |
| Spilled metadata batch | 300 long root names, then one long-name create batch; the batch entry present only at delta 1 with empty content, literal root count, exactly two allocation-root nodes written; directory, object-map and allocation-root paths stage three nodes; faults at every write and the sampled cut campaign with seed `0x5eed0002` | Spill writes 2/0/0/0 and peak staged nodes 2/3/3/3; 25 faults (23 writes, 2 flushes) and 23 same-handle retries; sampled campaign of 351 images (24 prefixes, 69 tears, 2 exhaustive subsets, 256 sampled subsets of the 22-write tail; outcomes 347/4) | Two pages spill; four and eight pages exceed the three-node demand and report zero spills |

A wrong free-block literal and a wrong pending literal fail their tests.

## Low-space family matrix

[family_matrix_low_space.rs](family_matrix_low_space.rs) drives a near-full
delete and a spilled ENOSPC refusal at 2/4/8/unlimited; 16 tests pass.

| Family and part | Oracle | Modeled count per profile | Deliberate limit |
|---|---|---|---|
| Near-full delete, cuts and faults | 512-block image; a victim file is preallocated to the largest range ordinary allocation admits, then deleted with a one-block reclaim batch; delta 0 keeps the name, zero size and the literal reserved allocation, delta 1 removes it; accounting (16, 7) before and (12, 10) after; literal root count; both ambiguous modes | 626 images (622/4); 11 faults (9 writes, 2 flushes) and 9 same-handle retries; 2 ambiguous cases | Memory device |
| Near-full delete, forced eviction | 2,048-block image with 300 long root names; accounting (64, 36) before and (56, 45) after; root-directory, object-map and reclaim paths stage three nodes at unlimited with zero spills; two pages report two spill writes; exhaustive cuts with a 12-write budget and faults at every write | 8,432 images (8,428/4); 15 faults (13 writes, 2 flushes) and 13 same-handle retries | Four and eight pages exceed the three-node demand and report zero spills |
| ENOSPC during spilled metadata allocation | 2,048-block image with 300 long root names and a pressure file reserving the largest admitted range; a 64-file create batch returns `NoSpace` with zero flushes and writes only to blocks unreachable from either selectable checkpoint (provisional spill images); the fixture state holds live and after remount; deleting the pressure file and draining reclaim until 512 blocks are available admits the retry, verified live, after remount and through the checker | Refused writes 158/16/11/0 at 2/4/8/unlimited, all to unreachable targets; 8 corrective commits; retry spill writes 164/19/13/0 | Ordinary allocation is exhausted before the batch, so the refusal comes from the batch's metadata allocation; a data-block ENOSPC inside a spilled batch has separate qualification |

A wrong free-block literal and a wrong refused-state literal fail their tests.

## Commands and review boundary

```sh
export CARGO_HOME=/private/tmp/afsplus-cargo
export RUSTUP_HOME=/private/tmp/afsplus-rustup
export CARGO_TARGET_DIR=/private/tmp/afsplus-target-codex-cache
export CARGO_NET_OFFLINE=true
export PATH="$CARGO_HOME/bin:$PATH"
cargo test --offline -p afsplus-check --test tiny_cache_matrix -- --nocapture --test-threads=2
for target in reclaim data_policy data_policy_persistence allocation_pressure shared_crash orphans \
    family_matrix_data_policy family_matrix_orphans family_matrix_reclaim family_matrix_low_space; do
  cargo test --offline -p afsplus-check --all-features --test "$target" -- --test-threads=2
done
cargo test --offline -p afsplus-core --all-features --lib allocation_cache_keeps_spilled_nodes_across_checkpoint_rotation
cargo test --offline -p afsplus-check --all-features --test tiny_cache_data_matrix -- --nocapture --test-threads=2
cargo fmt --all -- --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
make rust-codec-fuzz-gate
make check-docs
git diff --check
```

The codec script uses `target/rust-codec-fuzz` inside this separate worktree;
its output cannot collide with the coordinator's primary worktree.
Execution results and commit identities belong in the local board handoff.
The omissions outside explicitly qualified combinations retain their own work:
clone-range boundary/error cases; shared unlink and final-owner faults,
retained views during shared transitions and eviction; unwritten-reservation
data policy cuts; orphan replay branches, open-target replace faults and
orphaned-file updates; registry and maintenance failures; sealing, consumption
and cursor faults; and cache split/collapse/reload failures. Interrupted fsync
publication and resource or low-space refusals beyond preallocation and the
spilled batch retain family-specific qualification. Reclaim cursor/sealing
cuts, private data policy cuts, orphan multi-checkpoint lifecycles, shared
write/replace faults, orphan and reclaim-step faults, and forced eviction with
sampled cuts have evidence in the sections above.
These bounded tests do not close `a-cache` or `roadmap-31`.

## Data-write and size-change matrix

[tiny_cache_data_matrix.rs](tiny_cache_data_matrix.rs) covers full-COW and sparse
writes, bounded writes with reservation initialization, preallocation, sparse
growth and bounded shrink. Each test applies its cache profile at fixture
creation, recording, fault injection and every recovered or remounted image,
and asserts the effective profile after each mount. A persistent snapshot
precedes every mutation. The checker wrapper matches the one described above.
Every case runs the default full-COW policy and asserts zero in-place
overwrites; the in-place policy keeps its own row and weaker contract.

The small fixture on a 512-block volume holds a three-block direct file, a
two-extent sparse tree, a two-run private reservation tree and a reservation
tree cloned to a peer. Nineteen single-transaction cases each change one object:

| Family | Cases and layout transition |
|---|---|
| Full-COW write | direct partial overwrite (direct to tree); direct full rewrite (direct); extension past EOF (direct to tree); tree overwrite (tree); full tree rewrite (tree to direct) |
| Bounded write | private reservation initialization; partial initialization splitting a reservation; shared reservation fallback; bounded write of a direct file (direct to tree) |
| Preallocation | reservation past EOF of a direct file (direct to tree); reservation of a tree hole; reservation past EOF of a reservation tree |
| Size change | sparse growth of a direct file (direct to tree) and of a tree; bounded tree shrink with tail rewrite; shrink of a tree to a direct file; bounded direct shrink with tail rewrite (direct to tree); aligned bounded direct shrink (direct); bounded shrink of a reservation tree |

Each object has a literal oracle: bytes, size, extent-tree flag, the full
allocation enumeration (offset, length, unwritten) and allocated bytes. The
changed object also has exact change and modification times and content
generation; a reservation keeps the modification time and content generation.
Unchanged objects keep their exact records and extent records. Private
reservation initialization keeps the reserved physical addresses. The shared
fallback allocates outside the shared run and leaves the peer mapping and zero
bytes intact. All five captured objects keep exact metadata, bytes with an
untouched sentinel and captured allocation ranges.

The wide fixture holds a written sparse tree with one written block at every
second logical block and a reservation tree with a one-block reservation at
every third logical block. The record count per file is chosen per profile so
that the constrained cache evicts staged extent-map nodes: 40 at two pages on
1,024 blocks, 120 at four pages on 2,048 blocks, 500 at eight pages and
unlimited on 8,192 blocks. Its eight operations are a full-COW and a bounded
one-block write in the middle of the written tree, an unbounded and a bounded
one-block reservation initialization, a reservation of every hole in the
written tree, a full and a bounded shrink to half the tree with a nine-byte
tail, and growth to four times the size. Expected live and captured bytes and
ranges derive from the record index.

Wide-fixture cut images apply three checks. Each recorded transaction first
must satisfy an ownership proof: no recorded write lands on a physical block the old
state maps as written data, and only reservation initialization writes a
reserved block, the one it initializes. Every image of an old outcome then has
exact records, extent records, live and captured ranges, captured metadata,
literal live and captured bytes of the changed window and a clean checker report.
Because each image is the base plus recorded writes, unchanged extent records
and untouched data blocks fix the remaining bytes. Full live and captured byte
reads run on every committed image and on the empty and full unflushed
subsets of every cut point; faults, remounts and recorded results always use
full byte reads.

| Test | Profiles | Oracle and modeled counts | Deliberate limit |
|---|---|---|---|
| `data_mutations_preserve_exact_live_and_retained_state_in_all_profiles` | P | 19 cases per profile; exact live and captured state before and after remount; data blocks written, reservation initializations (2, 1 and 0 for the three reservation cases) and zero in-place overwrites; commit bytes and flushes equal device accounting | Small trees; eviction evidence belongs to the wide tests |
| `data_write_cuts_two_pages`, `data_write_cuts_four_pages`, `data_write_cuts_eight_pages`, `data_write_cuts_unlimited` | P | Every modeled cut of the nine write cases with a 12-write full-subset budget; both outcomes per case; 5,741 images per profile | Longest unflushed tail of 8 writes; memory-device cut model |
| `data_reserve_and_resize_cuts_two_pages`, `data_reserve_and_resize_cuts_four_pages`, `data_reserve_and_resize_cuts_eight_pages`, `data_reserve_and_resize_cuts_unlimited` | P | Every modeled cut of the ten reservation and size-change cases with the same budget; 5,438 images per profile | As above |
| `data_mutation_write_and_barrier_failures_preserve_ownership` | P | Before-write error at every recorded write and error at every flush, 235 injections per profile; an earlier failure preserves the exact old state and admits an in-place retry; a failed checkpoint write or final barrier returns `WindowPoisoned`; after remount the exact old state followed by a retry, or the exact new state after a failed final barrier | FaultBackend fails before a write; completed-but-reported-failed writes and adoption-read errors keep separate evidence |
| `data_mutation_refusals_issue_no_writes_and_retry_in_all_profiles` | P | Directory target, missing object, offset and range overflow, invalid time, `NoSpace` for a 600-block write and reservation, block, record, zero and unbounded budgets, retirement and window-record budgets; empty write, empty and covered reservation and same-size truncate as no-ops; all 19 operations on a read-only mount; 45 calls per profile with zero writes and flushes and an exact unchanged state, then the admitted case | Explicit refusal list |
| `data_eviction_two_pages` | 2 | 40 records; all eight operations spill 1 to 3 staged nodes with at most 2 resident; 119 injected write and flush failures with the fault oracle above | Faults only; cuts in the next row |
| `data_eviction_cuts_two_pages_writes`, `data_eviction_cuts_two_pages_reserve_and_resize` | 2 | Every modeled cut of the eight spilled operations (longest tail 10 writes); 10,560 images | Early spill writes precede the data barrier inside the enumerated tail |
| `data_eviction_four_pages` | 4 | 120 records; full-COW write, bounded write, hole reservation, full and bounded shrink and growth spill 1 to 3 staged nodes with at most 4 resident; 144 injected failures over all eight operations | Reservation-initializing writes stage four nodes and do not evict |
| `data_eviction_cuts_four_pages_bounded_write`, `data_eviction_cuts_four_pages_bounded_shrink`, `data_eviction_cuts_four_pages_shrink`, `data_eviction_cuts_four_pages_growth` | 4 | Every modeled cut of the spilled bounded write (12-write tail), bounded shrink, full shrink and growth (11-write tails); 12,826 images for the bounded pair and 8,661 for full shrink and growth | Full-COW write, hole reservation and reservation writes exceed the budget (13 to 15 writes); a 13-write budget for the full-COW write models about 17,000 images, beyond the per-test time budget |
| `data_eviction_eight_pages_writes_and_reservations`, `data_eviction_eight_pages_size_changes` | 8 | 500 records; full-COW write, unbounded reservation initialization, hole reservation, full and bounded shrink spill 3, 3, 5, 7 and 7 staged nodes with at most 8 resident; 150 injected failures over these five operations | No cuts: 16- to 27-write tails; bounded one-block writes and growth stage at most six nodes and do not evict |
| `data_eviction_unlimited_has_no_spills` | U | 500 records; zero spill writes and reloads; the five eight-page spilling operations stage 10, 10, 12, 15 and 15 resident nodes | Unlimited has no eviction by definition; its faults and cuts use the small fixture |

## Batch and directory-structure family matrix

[family_matrix_structure.rs](family_matrix_structure.rs) qualifies batch create
with payload tails, batch delete, the staged directory split and delete, the
cross-directory rename, the three symlink publication paths and the two
preserved-metadata paths through the family-matrix driver in
[common/family_matrix.rs](common/family_matrix.rs). Every fixture is formatted
with persistent snapshots, the profile applies from fixture creation through
every mount, and each mount asserts the effective profile.

The bounded fixtures carry the structural transitions. Seven 250-byte names
fill one directory leaf, so the eighth entry splits it and raises a directory
root: the split transaction records one split and one root split, and deleting
the entry records one merge and one root collapse. The cross-directory rename
moves a 250-byte name out of a full source leaf into a full target leaf, so one
transaction records one split, one root split, one merge and one root collapse.
Batch payload lengths are 0, 1, one block and two blocks less seven bytes, and
every entry is read back with a sentinel past the end.

The eviction fixtures spread twelve entries through the key space of 150 long
root names or 120 long directory names, so each entry lands in a different leaf.
Their staged demand is 17 nodes for the batch create, 16 for the batch delete
and 16 for both directory transactions, which forces spills at two, four and
eight pages. The single-entry transactions over a wide root have a staged demand
of three nodes and spill at two pages; a metadata change has a demand of one
node, which is the limit of those two fixtures.

| Test | Profiles | Oracle and modeled counts | Deliberate limit |
|---|---|---|---|
| `batch_create`, `batch_create_retained` | P | Exact root entries, the anchor bytes, every payload tail with an EOF sentinel and the captured anchor; 8,493 cut images per profile with a 12-write budget, 20 injected faults and 18 same-handle retries per profile; two ambiguous cases per profile | Five entries in one leaf; one anchor object |
| `batch_create_refusal` | P | A colliding last entry cancels the batch: zero writes, zero flushes, unchanged generation and blocker bytes, one corrective commit, then the complete batch | One collision shape |
| `batch_create_eviction` | P | 26, 13 and 9 spill writes at two, four and eight pages with peaks of 2, 4 and 8, zero spills and a peak of 17 at unlimited; 63, 52, 52 and 52 injected faults; seeded sampled cut campaigns of 307, 263, 263 and 4,327 images | Seed `0x5eedba7c0001`, sample 32 |
| `batch_create_spilled_ambiguous` | P | Completed checkpoint write and adoption-read errors on the spilled batch; the same and an independent mutation refuse without writes or flushes | Two cases per profile |
| `batch_delete`, `batch_delete_retained` | P | Removed names absent from the namespace and the object map, kept names with exact bytes, and every removed object read back through the retained view; 626 cut images, 11 faults and two ambiguous cases per profile | Five removals |
| `batch_delete_eviction`, `batch_delete_spilled_ambiguous` | P | 32, 13 and 8 spill writes with peaks of 2, 4 and 8, zero spills and a peak of 16 at unlimited; 45, 29, 28 and 28 faults; sampled campaigns of 207, 143, 139 and 139 images; two ambiguous cases per profile | Seed `0x5eedba7c0002`, sample 32 |
| `directory_split`, `directory_split_retained` | P | One split and one root split; the full directory listing compared as one enumeration, the subject bytes and the unchanged root; 2,235 images at two pages and 4,306 at the others, 16 faults and two ambiguous cases per profile | One subject entry |
| `directory_split_eviction` | P | 25, 12 and 8 spill writes with peaks of 2, 4 and 8, zero spills and a peak of 16 at unlimited; 62, 51, 51 and 51 faults; sampled campaigns of 303, 259, 259 and 4,323 images; the spread batch records no split | Seed `0x5eedba7c0003`, sample 32 |
| `directory_collapse`, `directory_collapse_eviction` | P | One merge and one root collapse in the bounded fixture with 626 images and 11 faults per profile; 36, 13 and 8 spill writes with peaks of 2, 4 and 8 in the spread fixture, 48, 27, 26 and 26 faults and sampled campaigns of 219, 135, 131 and 131 images | Seed `0x5eedba7c0004`, sample 32 |
| `cross_rename`, `cross_rename_retained` | P | One split, one root split, one merge and one root collapse; both directory listings as full enumerations, the moved bytes, a link count of one and the captured bytes; sampled campaigns of 91 images, 16 faults and two ambiguous cases per profile | 13-write tail, above the exhaustive budget; seed `0x5eedba7c0005`, sample 32 |
| `cross_rename_eviction` | P | 3 spill writes with a peak of 2 at two pages, zero spills and a peak of 3 elsewhere; 20 faults and 107-image sampled campaigns per profile | Measured staged demand of three nodes |
| `symlink_create`, `symlink_create_eviction`, `symlink_rename_eviction`, `symlink_unlink_eviction` | P | Opaque targets read with a sentinel past the end, exact root and directory entries; 1,165 cut images and 12 faults for the bounded create; one spill write with a peak of 2 at two pages and 14, 16 and 13 faults for the three wide publication paths | Measured staged demand of three nodes, so four and eight pages record no spill |
| `protection_retained`, `restore_retained`, `protection_eviction`, `restore_eviction` | P | Protection with unchanged creation and modification times and the change time set, the complete restored record, exact live bytes and captured bytes with an EOF sentinel; 346 cut images, 10 faults and two ambiguous cases per profile | Measured staged demand of one node, so no profile evicts |

## Residual batch and directory-structure family matrix

[family_matrix_structure_residuals.rs](family_matrix_structure_residuals.rs)
closes the batch, directory, namespace and symlink combinations the baseline
structure matrix left open, through the driver in
[common/family_matrix.rs](common/family_matrix.rs). Every fixture is formatted
with persistent snapshots, the profile applies from fixture creation through
every mount, and each mount asserts the effective profile.

The mixed batch interleaves three creates and three deletes in one
transaction, so one directory leaf stages both directions. Its payload tails
are one byte above two blocks, an exact three blocks and seven bytes below four
blocks, and every entry is read back with a sentinel past the end. The deep
fixtures fill one directory with 600 names of 250 bytes: a leaf holds seven
entries, so the descent passes a root and two interior levels, which the
recorded commit asserts as a measured depth of four. Eight subjects sharing one
adjacent key range fill a full leaf of that directory and split it below an
interior node, with no directory root split. The symlink fixtures use a target
of 3,968 bytes, the longest an inline symlink record represents at this block
size.

| Test | Profiles | Oracle and modeled counts | Deliberate limit |
|---|---|---|---|
| `mixed_batch`, `mixed_batch_retained` | P | Exact root entries, every created tail and every removed payload with an EOF sentinel, removed objects absent from the object map, and the captured removed bytes through the retained view; 22 writes and 3 flushes with an 11-write tail; sampled campaigns of 3,163 images per profile (23 prefixes, 66 tears, 3,074 exhaustive subsets, outcomes 3,159/4), 25 faults and 23 same-handle retries per profile; two ambiguous cases per profile | Three creates and three removals; seed `0x5eedba7c1001`, sample 32 |
| `window_batch`, `window_batch_retained` | P | Three deferred creates and one deferred delete published by one `window_commit`; exact root entries and payloads, and the removed object as an orphan with a count of one, because a deferred-window delete of a final visible link enters the reserved orphan directory; 18 writes and 2 flushes with a 17-write tail; sampled campaigns of 107 images (19 prefixes, 54 tears, 2 exhaustive subsets, 32 sampled subsets, outcomes 103/4); 20 fault cases per profile, every one remount-required; two ambiguous cases per retained profile | The window carries no logged group |
| `window_batch_eviction`, `window_batch_spilled_ambiguous` | P | 150 long root names; the deferred commit stages eight nodes at the unlimited profile with zero spills, and two and four pages report 9 and 4 spill writes with peaks of 2 and 4; eight pages match the demand and report zero spills; 30 fault cases at two pages and 27 elsewhere; sampled campaigns of 147 images at two pages and 135 elsewhere; two ambiguous cases per profile | Eight pages meet the measured demand of eight nodes |
| `deep_split` | P | The complete 600-entry directory listing compared as one enumeration with the eight subjects and their bytes, and the unchanged root; a measured descent depth of four, two leaf splits and zero root splits; 62, 34, 32 and 32 writes with 35, 4, 0 and 0 spill writes and peaks of 2, 4, 6 and 6; 35 faults and 33 same-handle retries at eight pages with a sampled campaign of 419 images (33 prefixes, 96 tears, 258 exhaustive subsets, 32 sampled subsets, outcomes 415/4) | Seed `0x5eedba7c1003`, sample 32 |
| `deep_cross_rename` | P | Both 600-entry listings as full enumerations, the moved bytes, a link count of one and no stale entry; a measured descent depth of four; 27, 24, 24 and 24 writes with 11, 1, 0 and 0 spill writes and peaks of 2, 4, 5 and 5; 29 faults at two pages and 26 elsewhere; sampled campaigns of 143 images at two pages and 131 elsewhere | Seed `0x5eedba7c1004`, sample 32 |
| `single_create`, `single_create_refusal` | P | Exact root entries and payload with an EOF sentinel; 11 writes and 3 flushes, 1,171 cut images with a 12-write budget (outcomes 1,167/4), 14 faults and 12 same-handle retries; a colliding name refuses with `AlreadyExists`, zero writes and zero flushes, one corrective commit, then the create | One collision shape |
| `mkdir`, `mkdir_refusal` | P | The published directory is empty and the occupant keeps its bytes before the corrective step; 11 writes, 2,219 cut images (outcomes 2,215/4), 13 faults and 11 same-handle retries; zero-write refusal with one corrective commit | One collision shape |
| `rmdir`, `rmdir_refusal` | P | The removed directory leaves the namespace and the object map; before publication its listing is the literal resident entry; 9 writes, 626 cut images (outcomes 622/4), 11 faults and 9 same-handle retries; a non-empty directory refuses with `DirectoryNotEmpty`, zero writes and zero flushes, one corrective commit, then the removal | One resident entry |
| `hard_link`, `hard_link_retained`, `hard_link_refusal` | P | Both names resolve to one identity with identical bytes, a link count of one before and two after, and the captured bytes through the retained view; 10 writes, 1,165 cut images (outcomes 1,161/4), 12 faults and 10 same-handle retries; two ambiguous cases per retained profile; a colliding alias refuses with zero writes and zero flushes | One alias |
| `max_symlink_create`, `max_symlink_create_refusal` | P | A 3,968-byte opaque target read with a sentinel past the end, exact root entries; 10 writes, 1,165 cut images (outcomes 1,161/4), 12 faults and 10 same-handle retries; a colliding name refuses with zero writes and zero flushes | The maximum inline target at a 4,096-byte block size |
| `max_symlink_rename`, `max_symlink_unlink` | P | The retained 3,968-byte target through the move and the removal, exact root and directory entries; 12 and 9 writes, 4,300 and 626 cut images, 14 and 11 faults | Bounded fixtures |
| `over_long_symlink_target` | P | A target of 3,969 bytes refuses with `InvalidMetadata`, zero writes, zero flushes and an unchanged generation; the maximum-length target then publishes, survives a remount and passes the checker | One length above the limit |

Negative controls, one per fixture family: a shifted created payload byte in the
mixed batch, an orphan count of zero after the deferred-window delete, three
expected leaf splits in the deep fixture, a link count of one after the hard
link, and a 3,967-byte expected symlink target. All five fail their test, and
the sources are restored from the commit afterwards.

## Reload failures during spilled transactions

`spilled_directory_split_survives_reload_read_failures`,
`spilled_directory_collapse_survives_reload_read_failures` and
`spilled_batch_create_survives_reload_read_failures` in
[family_matrix_structure.rs](family_matrix_structure.rs) fail one read inside a
transaction that has already written provisional staged images. Sixteen read
positions are spread through each transaction at two, four and eight pages, and
the successful run reports the spill and reload counters the injected window
covers: 25, 12 and 8 spills for the split with 11 reloads at two pages; 36, 13
and 8 spills for the collapse with 20 reloads at two pages and one at four; 26,
13 and 9 spills for the batch with 11 reloads at two pages.

Each failed call leaves a mount that selects either the fixture generation or
the published one. Both are checked with the family oracle, both pass the
checker, and a fixture-generation image publishes the complete new state on a
retry after remount. Read positions past the publication barrier leave the
published state, which the same oracle accepts. These tests use a memory device,
and they inject one read failure per case.

## Clone family matrix

[family_matrix_clone.rs](family_matrix_clone.rs) qualifies CloneFile, an
unaligned CloneRange and a write into a published clone. The source holds three
blocks with one byte value each. The range clone copies 2 blocks and 50 bytes
from byte 100 of the source to byte 100 of a four-block destination, so both
boundaries are partial: the reference records hold exactly one one-block
two-reference run, and the two boundary blocks are private copies. Reference
records are read from the selected checkpoint through
`shared_extents::load_all`.

| Test | Profiles | Oracle and modeled counts | Deliberate limit |
|---|---|---|---|
| `clone_file`, `clone_file_retained` | P | Root entries, an identity distinct from the source, identical source and clone bytes with an EOF sentinel, one three-block two-reference run when published and no record before; sampled campaigns of 91 images, 16 faults, 14 same-handle retries and two ambiguous cases per profile | 13-write tail, above the exhaustive budget; seed `0x5eedc10e0001`, sample 32 |
| `clone_file_refusal` | P | A file already at the clone's name refuses with zero writes and flushes, an unchanged generation and the occupant's bytes; one corrective commit, then the clone | One collision shape |
| `clone_file_eviction` | P | One spill write with a peak of 2 at two pages, zero spills and a peak of 3 elsewhere; 18 faults and 99-image sampled campaigns per profile | Measured staged demand of three nodes |
| `clone_range`, `clone_range_retained` | P | Destination bytes composed from the two literals, unchanged source bytes, one one-block two-reference run when published; 4,316 cut images with a 12-write budget, 17 faults, 15 same-handle retries and two ambiguous cases per profile | One unaligned range over a four-block destination |
| `clone_range_refusal` | P | A volume with at most one available block refuses the boundary copies with zero writes and flushes; the corrective step deletes the filler and drains the reclaim queue in two commits, then the range clone publishes, with 3 spill writes at two pages | Low-space refusal only |
| `clone_range_eviction`, `clone_mutation_eviction` | P | 17 and 14 faults per profile over a wide root | Measured staged demand of one node, so no profile evicts |
| `clone_mutation_retained` | P | A snapshot taken after the clone publishes keeps the captured bytes of source and clone; the write moves the clone off the first shared block, leaving one two-block two-reference run; 1,171 cut images, 14 faults and two ambiguous cases per profile | One partial-block range |

## Shared-ownership family matrix

[family_matrix_shared.rs](family_matrix_shared.rs) qualifies the replace rename
of an unshared target, a write that splits a shared run, the unlink of one of
three owners and the removal of a final owner. The origin holds two blocks with
one byte value each, and the clones are its peers.

| Test | Profiles | Oracle and modeled counts | Deliberate limit |
|---|---|---|---|
| `replace_unshared`, `replace_unshared_retained` | P | The target name always resolves: to the victim with its bytes before publication and to the incoming object with its bytes after, the replaced object absent from the object map, no reference record, and the captured victim bytes; 1,165 cut images, 12 faults and two ambiguous cases per profile | Both objects are unshared |
| `replace_unshared_refusal` | P | A missing source refuses with zero writes and flushes and an unchanged victim; one corrective commit creates the source, then the replacement publishes | One refusal shape |
| `replace_unshared_eviction` | P | 3 spill writes with a peak of 2 at two pages, zero spills and a peak of 3 elsewhere; 16, 14, 14 and 14 faults | Measured staged demand of three nodes |
| `shared_write_retained` | P | The origin moves off the first block of the run, the peer keeps its exact bytes, one one-block two-reference run when published and one two-block run before; captured bytes of both objects; 1,171 cut images, 14 faults and two ambiguous cases per profile | One partial-block range |
| `shared_write_refusal` | P | A volume filled until at most one block is available refuses the shared write with zero writes and flushes; the corrective step deletes the filler and alternates reclaim with a metadata-only publication over five commits, then the write publishes, with 5 spill writes at two pages | Low-space refusal only |
| `shared_write_eviction` | P | 14 faults per profile over a wide root | Measured staged demand of one node, so no profile evicts |
| `shared_unlink`, `shared_unlink_retained` | P | Three owners drop to two: the run keeps both blocks, the removed object leaves the namespace and the object map, the two survivors keep exact bytes and all three are read through the retained view; 1,165 cut images, 12 faults and two ambiguous cases per profile | One run of two blocks |
| `final_owner`, `final_owner_retained` | P | Two owners drop to one: the reference record disappears and the survivor keeps its exact bytes, while the retained view reads both captured objects; 1,165 cut images, 12 faults and two ambiguous cases per profile | Storage reuse after the final owner keeps its own evidence |
| `shared_unlink_eviction`, `final_owner_eviction` | P | One spill write with a peak of 2 at two pages, zero spills and a peak of 3 elsewhere; 14 faults per profile | Measured staged demand of three nodes |

Negative controls, one per fixture family: a shifted batch payload byte, a link
count of two after the cross-directory rename, one extra expected directory
split, a spill-counter threshold above the observed count, a three-reference
first clone, a two-block shared run for the unaligned range, an unchanged owner
count after the unlink and rewritten survivor bytes after the final owner. All
eight fail their test, and the sources are restored from the commit afterwards.

```sh
export CARGO_HOME=/private/tmp/afsplus-cargo
export RUSTUP_HOME=/private/tmp/afsplus-rustup
export CARGO_TARGET_DIR=/private/tmp/afsplus-wt-cache-structure/target
export CARGO_NET_OFFLINE=true
export PATH="$CARGO_HOME/bin:$PATH"
for target in family_matrix_structure family_matrix_clone family_matrix_shared; do
  cargo test --offline -p afsplus-check --all-features --test "$target" -- --test-threads=2
done
cargo fmt --all -- --check
cargo clippy --offline -p afsplus-check --all-features --tests -- -D warnings
make check-docs
```

## Deferred window family matrix

[family_matrix_deferred.rs](family_matrix_deferred.rs) drives four deferred
families through the replay runner at 2/4/8/unlimited; 40 tests pass. Every
fixture formats eight log slots and persistent snapshots on a 1,024-block
volume, and the profile applies from formatting through logging, every
no-changes inspection and every recovery mount. The acknowledged record count
of each image names the allowed state, so an image keeps the acknowledged
groups, may drop later staged work, and admits nothing else. Recorded
transactions report zero spill writes and one resident staged node, which
limits these fixtures to profile coverage.

| Family and part | Oracle | Modeled count per profile | Deliberate limit |
|---|---|---|---|
| Namespace, three groups | A create, a rename, and a create paired with a delete inside one group: exact root listing, literal payload bytes of the anchor and of the group's own entry, and the absence of every name the count does not admit | 5 log writes and 3 log flushes; 10 recovery writes, 2 recovery flushes and a 9-write recovery tail; 39 logging-cut images with acknowledged counts 13/6/16/4; 1,165 exhaustive recovery-cut images (1,161 pre and 4 post); 12 recovery faults (10 writes, 2 flushes) | Three groups over one anchor |
| Namespace, retained snapshot | The snapshot taken before logging keeps the anchor's captured metadata and bytes through every logging cut and every recovery cut | 39 logging-cut and 1,165 recovery-cut images | Faults belong to the plain part |
| Existing-file write, three groups | Three writes at 73, 4,106 and 2, the last extending past one block: the byte image after a given number of acknowledged writes, with the file's other bytes literal | 7 log writes and 6 log flushes; 9 recovery writes and an 8-write tail; 47 logging-cut images with counts 9/12/22/4; sampled recovery campaign of 167 images (10 prefixes, 27 tears, 130 subsets, 163 pre and 4 post); 11 recovery faults | Seeds `0x5eedde110001` and `0x5eedde110002`, sample 128 |
| Truncate, three groups | A partial shrink to 4,307, a sparse growth to 12,338 and an aligned shrink to 4,096: exact bytes, including the zeros the growth exposes, and the exact size | 4 log writes and 4 log flushes; 8 recovery writes and a 7-write tail; 25 logging-cut images with counts 9/6/6/4; campaign of 163 images (9 prefixes, 24 tears, 130 subsets, 159 pre and 4 post); 10 recovery faults | Seeds `0x5eedde110003` and `0x5eedde110004` |
| Mixed window, two groups | One group with a create, a write and a truncate, and a second with a rename and a write: root listing, created payload, exact bytes and size of the subject after each count | 6 log writes and 4 log flushes; 12 recovery writes and an 11-write tail; 52 logging-cut images with counts 36/12/4; campaign of 179 images (13 prefixes, 36 tears, 130 subsets, 175 pre and 4 post); 14 recovery faults | Seeds `0x5eedde110005` and `0x5eedde110006` |

`window_refusals` enumerates the refusal and read-failure paths of the window
entry points of [core src/volume.rs](../../afsplus-core/src/volume.rs) over a
window that holds one acknowledged group and one staged group: an invalid time,
a duplicate name, a missing parent, a parent that is a file, an invalid name
and a preflight read failure for `window_op`; an invalid time, an offset
overflow, a missing object, a directory target and a layout read failure for
`window_write_file_at`; an invalid time, a missing object, a directory target
and a record read failure for `window_truncate_file`; an invalid time for
`window_commit`; and all five entry points on a no-changes mount. The 21
refusals per profile issue no write and no flush, keep the pending operation
count and the generation, and the two admitted no-ops (an empty write and an
unchanged truncate) behave the same way. The staged group then becomes durable
and recovery exposes both groups with literal bytes. `window_group_limit`
fills a one-slot log, so the second `window_fsync` refuses with
`PrototypeLimit` and no I/O while `window_commit` publishes both groups.

Every oracle constant of this file is spelled out separately from the value the
operation supplies, so a changed expectation fails the test. Negative controls:
a shifted namespace payload byte, a wrong write patch value, a wrong truncate
size and a wrong mixed-window size each fail their test, and the source is
restored from the commit afterwards with a matching hash.

## Deferred replay family matrix

[family_matrix_replay.rs](family_matrix_replay.rs) drives six replay families
at 2/4/8/unlimited through the replay runner; 56 tests pass. The plain,
retained and refusal fixtures use a 4,096-block volume with eight log slots,
and the eviction fixture uses 16,384 blocks in 16-block regions with 300 long
root names. Every recovery cut uses the seeded campaign, because the recovery
tails run from 9 to 26 writes.

| Family and part | Oracle | Modeled count per profile | Deliberate limit |
|---|---|---|---|
| Final-link delete of a fragmented file | Three one-block extents at logical blocks 0, 2 and 4: below the acknowledged group the name resolves and the orphan count is zero; at the group the name is absent, the object carries the orphan flag, the orphan count is one, and the payload and allocated bytes are literal | 1 log write and 1 log flush; 12 recovery writes and an 11-write tail; 7 logging-cut images (3 and 4 per outcome); campaign of 179 images (13 prefixes, 36 tears, 130 subsets, 175 pre and 4 post); 14 recovery faults | Seeds `0x5eedde1e0001` (plain) and `0x5eedde1e0002` (retained) |
| The same delete with ordinary allocation consumed | A preallocated filler leaves at most 32 blocks, so recovery publishes the orphan transition through the emergency headroom | Same counts as above; seed `0x5eedde1e0003` | Replay is privileged maintenance |
| The same delete over a wide root | 300 long root names: the replay commit stages three nodes at unlimited with zero spills, and two pages reports 2 spill writes with a peak of 2 | 25 recovery writes and a 24-write tail; campaign of 231 images (26 prefixes, 75 tears, 130 subsets, 227 pre and 4 post); 27 recovery faults; seed `0x5eedde1e0004` | Four and eight pages exceed the three-node demand and report zero spills |
| A durable write and a durable final delete | The orphan carries the replayed bytes: the patch at 4,079 appears at the first acknowledged group and the name disappears at the second | 4 log writes and 3 log flushes; 12 recovery writes; 29 logging-cut images (19/6/4); campaign of 179 images; 14 recovery faults; seed `0x5eedde1e0005` | Two groups |
| A durable replacing rename | The target name resolves to the victim before the group and to the incoming object at the group, the replaced object carries the orphan flag, and both payloads are literal | 13 recovery writes and a 12-write tail; 7 logging-cut images; campaign of 183 images (14 prefixes, 39 tears, 130 subsets); 15 recovery faults; seeds `0x5eedde1e0006` and `0x5eedde1e0007` | One victim |
| One group deleting sixteen files | Every name disappears together, every object carries the orphan flag with its literal byte, and the orphan count is sixteen | 27 recovery writes and a 26-write tail; campaign of 239 images (28 prefixes, 81 tears, 130 subsets, 235 pre and 4 post); 29 recovery faults; seed `0x5eedde1e000c` | Sixteen orphans |
| Durable unlink of one of three owners | The victim leaves the namespace and enters orphan state, so the two-block run keeps three references through every image, and all three objects read their exact bytes | 12 recovery writes; 7 logging-cut images; campaign of 179 images; 14 recovery faults; seeds `0x5eedde1e0008` and `0x5eedde1e0009` | One run of two blocks |
| Durable write into a shared run | The origin moves off the first block: one two-block two-reference run before the group and one one-block two-reference run at it, with the peer's bytes exact | 2 log writes and 2 log flushes; 10 recovery writes and a 9-write tail; 13 logging-cut images (9 and 4); campaign of 171 images (11 prefixes, 30 tears, 130 subsets); 12 recovery faults; seeds `0x5eedde1e000a` and `0x5eedde1e000b` | One partial-block range |

`deferred_refusal` fills a 256-block volume to at most 32 available blocks,
logs one group and stages a second, and asks for a 40-block payload: the window
reports `NoSpace` with zero writes and zero flushes, keeps the pending
operation count and the generation, and the corrective step of a commit, a
delete and reclaim rounds (5 commits) admits the same call, whose recovery
exposes all three entries with literal bytes. `repeated_recovery` recovers a
published image three times and requires the generation and the state to hold.

Negative controls: a wrong orphan-payload byte and a wrong reference-record
shape each fail their test, and the source is restored with a matching hash.

## Persistent snapshot family matrix

[family_matrix_snapshot.rs](family_matrix_snapshot.rs) drives five snapshot
families and three refusal tests at 2/4/8/unlimited; 52 tests pass. Membership
is read back through `snapshot_list` on the selected checkpoint, live and
captured bytes carry a sentinel past the end, and a deleted identity must be
unreachable through `snapshot_open`.

| Family and part | Oracle | Modeled count per profile | Deliberate limit |
|---|---|---|---|
| Registry create, cuts and faults | The registry gains exactly one identity, the subject's bytes and the root listing hold, and the published view captures the subject | 7 writes, 2 flushes and a 6-write tail; 197 cut images (193 and 4); 9 faults (7 writes, 2 flushes) and 7 same-handle retries | One subject file |
| Registry create, retained snapshot and ambiguous publication | An older view taken in setup keeps its captured metadata and bytes through every image; both ambiguous modes poison with no I/O | 115 cut images; 8 faults, 6 same-handle retries, 2 ambiguous cases | Memory device |
| Registry create, forced eviction | 300 long root names: the registry commit stages two nodes at unlimited with zero spills, which is the measured demand and the limit of this fixture | 14 writes and a 13-write tail; 16 faults, 14 same-handle retries; sampled campaign of 187 images (15 prefixes, 42 tears, 2 exhaustive subsets, 128 sampled subsets), seed `0x5eed50a00001` | A two-node demand fits every bounded profile, so no profile evicts |
| Registry delete, cuts, faults and ambiguous publication | The identity leaves the registry and its view becomes unreachable, while an older retained view keeps its captured bytes | 6 writes and a 5-write tail; 115 cut images; 8 faults and 6 same-handle retries; 2 ambiguous cases | One deletion |
| Maintenance step | A captured delete fills the ledger, a two-block reclaim batch runs one step: registry membership, the deleted name's absence, the keeper's bytes and both captured objects hold at every image | 5 writes and a 4-write tail; 68 cut images (64 and 4); 7 faults and 5 same-handle retries; 2 ambiguous cases | One ledger step |
| Release of the last view | Deleting the last view releases its ledger: membership empties, the view becomes unreachable and the keeper's bytes hold | 6 writes; 115 cut images; 8 faults and 6 same-handle retries; 2 ambiguous cases | One view |
| Snapshot-aware mount recovery | Two durable groups (a write and a truncate) over a file whose pre-write bytes a snapshot captures: every image reads the live bytes of its acknowledged prefix and the captured historical bytes | 4 log writes and 4 log flushes; 8 recovery writes and a 7-write tail; 25 logging-cut images (9/12/4); 346 exhaustive recovery-cut images (342 pre and 4 post); 10 recovery faults | One captured file |

`admission_limits` sets a two-view budget: the third creation refuses with
`PrototypeLimit`, a busy view refuses with `Busy` and a missing identity with
`NotFound`, each with zero writes and flushes and an unchanged generation and
membership; a deletion then admits the creation, whose captured bytes and
membership survive a profile remount and the checker.
`admission_before_recovery` mounts an image that holds two views and one
durable group behind a device that refuses every write and flush: five invalid
budgets in each of the four mount modes give 20 refusals with `PrototypeLimit`,
and the valid admission recovers the group.
`previous_slot_protection` makes the previous checkpoint slot hold an empty
registry and fails every read of that block: the optional in-place write
reports the injected error, publishes nothing, keeps the exact old bytes live
and after remount, and the readable registry admits the same write in place
(2 blocked reads per profile).

Negative controls: a wrong registry membership and wrong captured bytes each
fail their test, and the source is restored with a matching hash.

```sh
export CARGO_HOME=/private/tmp/afsplus-cargo
export RUSTUP_HOME=/private/tmp/afsplus-rustup
export CARGO_TARGET_DIR=/private/tmp/afsplus-wt-cache-deferred-snapshots/target
export CARGO_NET_OFFLINE=true
export PATH="$CARGO_HOME/bin:$PATH"
for target in family_matrix_deferred family_matrix_replay family_matrix_snapshot; do
  cargo test --offline -p afsplus-check --all-features --test "$target" -- --test-threads=2
done
for target in intent_log intent_replay_orphans shared_crash orphans tiny_cache_matrix; do
  cargo test --offline -p afsplus-check --all-features --test "$target" -- --test-threads=2
done
cargo test --offline -p afsplus-core --all-features --lib volume::snapshots

## Residual data family matrix

[family_matrix_data_residuals.rs](family_matrix_data_residuals.rs) drives the
spilled data cuts beyond the exhaustive budget, the ambiguous publication of
each of those operations and shared-run truncate at 2/4/8/unlimited; 61 tests
pass. [family_matrix_space_residuals.rs](family_matrix_space_residuals.rs)
drives the in-place resource refusal and the two low-space residuals; 12 tests
pass.

The forced-eviction fixture holds one written sparse tree, with a written block
at every second logical block, and one unwritten reservation tree, with a
one-block reservation at every third. The record count per file follows the
profile: 40 at two pages, 120 at four pages, 500 at eight pages and unlimited.
The reservation-initializing write takes 500 records from four pages up,
because four staged nodes at 120 and at 160 records fit the four-page profile.
The plain and ambiguous fixtures hold four records per file.

Every image checks both objects: the logical size, the direct/extent-tree flag,
the complete allocation enumeration, and literal bytes over the first block,
the window the operation changes and the block at end of file read with a
sentinel past it, live and through the snapshot with its exact captured
metadata and captured enumeration. The complete allocation enumeration is
exact, so the extent records the operation leaves alone fix every byte outside
those windows. Each recording adds the complete literal bytes of every object,
live and captured, and asserts zero in-place overwrites. The driver's checker
wrapper and effective-profile assertion apply from formatting through every
recovered mount. Counts below run 2/4/8/unlimited.

| Family and part | Oracle | Modeled count per profile | Deliberate limit |
|---|---|---|---|
| Spilled full-COW write, spill evidence and faults | 100 bytes at offset 7 of the middle written block; staged demand 3, 6, 10 and 10 with 1, 2, 3 and 0 spill writes and peaks of 2, 4, 8 and 10; 12, 17, 27 and 27 writes over 3 flushes with 9-, 13-, 22- and 25-write tails | 15, 20, 30 and 30 faults; 13, 18, 28 and 28 same-handle retries | Memory device |
| Spilled full-COW write, sampled cut campaign | Seed `0xc00d17e001`, sample 64: 13, 18, 28 and 28 prefixes, 36, 51, 81 and 81 tears, 518, 10, 18 and 4 exhaustive subsets, 0, 64, 64 and 64 sampled subsets | 567, 143, 191 and 177 images | Reorderings of a segment longer than twelve writes stay outside the evidence |
| Spilled reservation initialization, spill evidence and faults | 100 bytes at offset 7 of the middle reservation, which the initialization clears to written; demand 3, 10, 10 and 10 with 1, 11, 3 and 0 spill writes; 12, 27, 27 and 27 writes with 9-, 14-, 22- and 25-write tails | 15, 30, 30 and 30 faults; 13, 28, 28 and 28 same-handle retries | Reservation trees of 120 and 160 records stage four nodes, so `reservation_initialization_stages_four_nodes_at_120_and_160_records` records that measured limit |
| Spilled reservation initialization, sampled cut campaign | Seed `0xc00d17e002`, sample 64: 518, 4,098, 18 and 4 exhaustive subsets and 0, 64, 64 and 64 sampled subsets | 567, 4,271, 191 and 177 images | As above |
| Bounded write over eight extent leaves | One leaf holds 100 extent records, so the window covers 1,600 logical blocks of an 1,100-record tree and publishes one 1,600-block extent with the sparse tail behind it; demand 24 with 24, 22, 18 and 0 spill writes; 1,635, 1,635, 1,635 and 1,633 writes with 1,624-, 1,622-, 1,618- and 1,600-write tails | Four recordings through `eviction_recorded` | Nine staged nodes need a window of at least six leaves, so the fault matrix and every cut campaign of this transaction fall outside the per-test time budget |
| Spilled hole reservation, spill evidence and faults | Every hole of the written tree reserved, giving 2*n alternating written and unwritten one-block ranges with unchanged bytes, modification time and content generation; demand 3, 5, 12 and 12 with 1, 1, 5 and 0 spill writes; 11, 16, 28 and 28 writes with 10-, 15-, 27- and 27-write tails | 13, 18, 30 and 30 faults; 11, 16, 28 and 28 same-handle retries | Memory device |
| Spilled hole reservation, sampled cut campaign | Seed `0xc00d17e003`, sample 64: 1,026, 2, 2 and 2 exhaustive subsets and 0, 64, 64 and 64 sampled subsets | 1,071, 131, 179 and 179 images | As the full-COW campaign |
| Spilled shrink and bounded shrink, spill evidence and faults | Half the tree with a nine-byte tail: the truncated byte literal, the truncated enumeration and the retained full captured view; demand 3, 7, 15 and 15 for both, with 3, 3, 7 and 0 spill writes; the shrink records 14, 16, 29 and 29 writes with 9-, 11-, 20- and 27-write tails and the bounded shrink 14, 16, 25 and 25 with 9-, 11-, 16- and 23-write tails | Shrink 17, 19, 32 and 32 faults with 15, 17, 30 and 30 retries; bounded shrink 17, 19, 28 and 28 faults with 15, 17, 26 and 26 retries | Memory device |
| Spilled shrink and bounded shrink, sampled cut campaigns | Seeds `0xc00d17e004` and `0xc00d17e005`, sample 64 | Shrink 587, 2,131, 439 and 185 images; bounded shrink 587, 2,131, 423 and 169 images | As the full-COW campaign |
| Sparse growth, exhaustive cuts and faults | Growth to four times the size with the enumeration held and the tail zeros literal; demand 3, 5, 4 and 4 with 1, 1, 0 and 0 spill writes; 10, 12, 11 and 11 writes with 9-, 11-, 10- and 10-write tails | 1,165, 4,300, 2,219 and 2,219 cut images with a 12-write budget; 12, 14, 13 and 13 faults with 10, 12, 11 and 11 retries | The staged demand of 3, 5 and 4 at 40, 120 and 500 records holds below five, so profiles of four pages and above report zero spills |
| Ambiguous publication of the full-COW write, the reservation initialization, the hole reservation, the shrink and the growth | Completed checkpoint write reported failed and adoption reads failing after it; the same operation and an independent create return `WindowPoisoned` with no writes or flushes; remount exposes exactly one publication | 2 cases per family per profile, 40 cases | Four-record fixtures |
| Shared-run truncate, cuts, faults and retained snapshot | A four-block file cloned to a peer whose rewrite of block 1 splits the run; the source truncated to two blocks and nine bytes publishes a two-block and a one-block range, the peer keeps its three ranges and exact bytes, and the snapshot keeps both captured objects | 1,171 cut images with a 12-write budget; 14 faults (11 writes, 3 flushes) and 12 same-handle retries per profile; 2 ambiguous cases | One private peer block |
| Shared-run truncate, retirement budget refusal | A one-block retirement budget refuses the truncate, which retires the removed tail block and the rewrite of the partial block; zero writes and zero flushes, the fixture holds live and after remount, and the budget of two admits the retry | 4 refusals, one per profile | The corrective step raises the budget and publishes nothing |
| In-place write resource refusal | A flagged two-block file and a pressure file reserving the largest admitted range; an extending write of 128 blocks falls back to full COW and returns `NoSpace` with zero writes and zero flushes; deleting the pressure file and draining reclaim admits the retry, which overwrites zero blocks in place | 4 refusals with 4 corrective commits and zero retry spills | The block and record budgets refuse before the policy decision, so the fallback's ENOSPC carries this refusal |
| Near-full delete under a retained snapshot | A 512-block image with a two-block survivor, a one-block victim and a pressure file; the snapshot captures both files, accounting moves from (16, 14) to (12, 17), and every image keeps the survivor's bytes, the reserved layout and both captured objects | 626 images (622/4); 11 faults (9 writes, 2 flushes) and 9 same-handle retries; 2 ambiguous cases | Memory device |
| Data-block ENOSPC inside a spilled batch | A 2,048-block image with 300 long root names; a batch of 64 long-name entries without content measures a metadata demand of 89 blocks, the pressure file leaves 115 blocks of capacity, and the same batch with two-block payloads returns `NoSpace` for its 128 data blocks with zero flushes and writes only to blocks unreachable from either selectable checkpoint | Refused writes 158, 16, 11 and 0, all to unreachable targets; 9 corrective commits; retry spill writes 164, 19, 13 and 0 | The measured 115 blocks exceed the metadata demand and fall short of the data demand |

Negative controls, one per fixture family: a shifted full-COW byte, a shifted
reservation byte, a shifted window byte, an inverted reservation parity, a
one-byte longer shrink tail, a growth to five times the size, one merged range
after the shared-run truncate, a shifted extension byte, one extra pending
block after the near-full delete and a metadata demand of 90 blocks. All ten
fail their test, and the sources are restored from the commit and hash-checked
afterwards.

```sh
export CARGO_HOME=$HOME/.cargo
export RUSTUP_HOME=$HOME/.rustup
export CARGO_TARGET_DIR=/private/tmp/afsplus-wt-cache-residual-data/target
export CARGO_NET_OFFLINE=true
export CARGO_BUILD_JOBS=2
export PATH="$CARGO_HOME/bin:$PATH"
for target in family_matrix_data_residuals family_matrix_space_residuals; do
  cargo test --offline -p afsplus-check --all-features --test "$target" -- --test-threads=2
done
cargo fmt --all -- --check
cargo clippy --offline -p afsplus-check --all-features --tests -- -D warnings
make check-docs
```
