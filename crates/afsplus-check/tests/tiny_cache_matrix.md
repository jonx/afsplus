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
| Batch create/delete, cancellation, payload tails | [cache_profiles.rs](cache_profiles.rs): `batch_create_delete_and_remount_match_at_two_four_eight_and_unlimited_pages`; [batch_payloads.rs](batch_payloads.rs): `surviving_payloads_borrow_full_blocks_and_zero_each_tail_in_all_profiles`, `cancelled_and_invalid_batches_do_not_issue_payload_writes` | P: exact namespace/bytes, checker, real spills, peak staged nodes, zero payload writes on refusal | Per-family retained-view failure oracles; large spilled batch cuts beyond bounded exhaustive tail |
| Batch publication failures | [batch_payloads.rs](batch_payloads.rs): `borrowed_write_and_data_barrier_errors_preserve_old_state_and_allow_retry`, `borrowed_payload_crash_matrix_preserves_exact_old_or_new_state` | P: borrowed data faults and modeled cuts, exact old/new contents | Spilled metadata failures at 4/8; completed-write/adoption errors by family |
| Staged directory split | [cache_profiles.rs](cache_profiles.rs): `spilled_directory_split_is_atomic_at_every_modeled_cut`, `failed_provisional_spills_leave_the_committed_view_and_allow_retry` | 2: real spill, all modeled split cuts; writes 0/1/7/31 fail then retry | 4/8/unlimited split cuts; 4/8 spill failures and retries |
| Single create, mkdir, rmdir, hard links | [basic.rs](basic.rs); [crash_matrix.rs](crash_matrix.rs): `every_crash_state_of_a_create_transaction_recovers_to_an_allowed_state` | U: functional namespace/lifetime and create cuts | P named semantic/cut/fault/refusal oracles for each distinct path |
| Cross-directory rename, root split/collapse | [crash_matrix.rs](crash_matrix.rs): `every_crash_state_of_cross_directory_rename_is_atomic`, `directory_root_split_and_collapse_are_crash_atomic` | U: atomic namespace and checker | P structural transitions, failure/retry and retained views |
| Replace rename, shared target | [faults.rs](faults.rs): `replacement_write_and_flush_failures_preserve_complete_namespace_and_bytes`; [shared_crash.rs](shared_crash.rs): `rename_replace_of_a_shared_target_is_crash_atomic`, `shared_replace_io_failures_preserve_exact_ownership_and_allow_retry_in_all_profiles`, `shared_replace_ambiguous_publication_requires_remount_in_all_profiles` | P: shared-target cuts with exact victim/incoming/peer bytes and ownership; shared-target write/flush faults with pre-retry remount, same-handle retry and ambiguous publication; U: replacement I/O faults | P write/flush faults for unshared replacement; retained snapshots; forced eviction (zero spill writes observed); resource refusal |
| Symlink create/rename/unlink | [metadata.rs](metadata.rs): `symlink_namespace_preserves_target_through_metadata_rename_and_remount`, `symlink_publication_cuts_preserve_namespace_and_captured_target`, `symlink_refusals_and_short_reads_do_not_write` | U: exact opaque target, retained target, cuts and no-write refusals | P all three publication paths; forced eviction |
| Protection and preserved metadata | [metadata.rs](metadata.rs): `metadata_publication_cuts_preserve_exact_old_or_new_state_and_snapshot`, `metadata_refuses_open_windows_and_uncertain_publication_requires_remount`; [core tests/flight.rs](../../afsplus-core/tests/flight.rs): `api_snapshot_and_metadata_calls_preserve_captured_state_and_busy_refusals` | U: exact metadata/cuts/poison; P: protection and busy snapshot deletion with observed/unobserved image equality | P metadata restore cuts/faults; protection cuts; exact retained bytes in addition to metadata |
| Full-COW write, sparse write | [streaming_api.rs](streaming_api.rs): `seeded_mixed_io_preserves_bytes_across_remounts_and_clones`; [crash_matrix.rs](crash_matrix.rs): `every_crash_state_of_a_sparse_write_is_atomic`; [tiny_cache_data_matrix.rs](tiny_cache_data_matrix.rs): full-COW cases and wide-fixture writes | U: independent mixed-byte oracle, sparse cuts; P: direct partial and full rewrite, sparse extension, tree overwrite and tree-to-direct rewrite with exact bytes, ranges, layout flag and captured snapshot at every modeled cut and write/flush fault, no-write refusals; spilled full-COW writes at 2/4/8 with faults, 2-page spilled cuts; zero spills at unlimited | Spilled full-COW cuts at 4 pages (13-write tail) and 8 pages (22-write tail) beyond the 12-write budget; completed-but-reported-failed writes and adoption-read errors |
| Bounded write, reservation initialization | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `bounded_writes_crash_to_exact_bytes_and_preserve_captured_zeros`, `reservation_write_crashes_preserve_old_zeros_or_complete_new_bytes`, `reservation_write_io_errors_preserve_old_logical_zeros_and_require_reconciliation`; [tiny_cache_data_matrix.rs](tiny_cache_data_matrix.rs): bounded-write cases and wide-fixture reservation writes | U: captured zeros, exact bytes, reconciliation; P: admitted budgets and block, result-record, zero and unbounded budget refusals with zero writes and retry; initialization at the reserved physical addresses; shared-reservation fallback to a fresh block with an unchanged peer; cuts, faults and captured snapshot; spilled bounded writes at 2/4, spilled reservation initialization at 2 (bounded and unbounded) and 8 (unbounded) with faults; 2-page spilled cuts and 4-page bounded-write cuts | Reservation-initializing writes evicting at 4 pages (four staged nodes with 120 and 160 records); bounded writes evicting at 8 pages (a window spanning at least eight extent leaves); spilled reservation-initialization cuts at 4/8 pages; completed-but-reported-failed initialization writes at P |
| Preallocation / bounded reservation | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `bounded_reservation_refusal_and_boundary_retry_preserve_layout`, `bounded_reservation_tree_publication_preserves_snapshot_at_every_cut`; [tiny_cache_data_matrix.rs](tiny_cache_data_matrix.rs): preallocation cases and wide-fixture hole reservation | U: allocation layout, refusal/retry, snapshot cuts; P: exact ranges, bytes and size for direct past-EOF, tree-hole and reservation-tree past-EOF reservations at every cut and fault; block, record, invalid-limit, range-overflow, time, directory and `NoSpace` refusals plus empty and covered no-ops with zero writes and retry; spilled multi-leaf reservation at 2/4/8 with faults and 2-page cuts; headroom-violating and near-full preallocation refusals with retry after reclaim in [Low-space refusal and retry profiles](#low-space-refusal-and-retry-profiles) | Spilled reservation cuts at 4 pages (15-write tail) and 8 pages (27-write tail); completed-but-reported-failed writes |
| Truncate / sparse growth / bounded shrink | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `sparse_growth_publication_is_atomic_with_retained_reservations`, `bounded_shrink_crash_preserves_shared_and_captured_bytes`; [shared_crash.rs](shared_crash.rs): `truncate_across_private_and_shared_subruns_is_crash_atomic`; [tiny_cache_data_matrix.rs](tiny_cache_data_matrix.rs): size-change cases and wide-fixture shrink/growth | P: private/shared truncate cuts with independent survivor bytes; U: sparse growth, bounded shrink and retained-state cuts; P: direct-to-tree and tree sparse growth, bounded tree, direct and reservation shrink, tree-to-direct and direct-to-tree shrink transitions with exact bytes, ranges and captured snapshot at every cut and fault; retirement, window-record, zero-record, directory and time refusals plus a same-size no-op; spilled full and bounded shrink at 2/4/8 and growth at 2/4 with faults; 2-page spilled cuts and 4-page bounded shrink, full shrink and growth cuts | Growth evicting at 8 pages; spilled cuts of every spilled size change at 8 pages (16- to 20-write tails); faults, refusals and retained snapshots for shared-run truncate; completed-but-reported-failed writes |
| In-place policy flag and private data | [data_policy_persistence.rs](data_policy_persistence.rs): `opt_in_persists_across_remount_and_takes_the_in_place_path`, `policy_flag_publication_cuts_preserve_exact_choice_and_bytes_in_all_profiles`, `policy_flag_io_failures_preserve_exact_choice_and_retry_in_all_profiles`, `policy_flag_ambiguous_publication_blocks_mutations_until_remount_in_all_profiles`, `policy_flag_applicable_refusals_issue_no_writes_and_preserve_state_in_all_profiles`, `retained_snapshot_forces_cow_for_flagged_private_writes_in_all_profiles`, `in_place_write_after_a_crash_keeps_metadata_clean`; [data_policy.rs](data_policy.rs): `in_place_crash_matrix_keeps_metadata_clean_but_allows_torn_old_data` | P: persistent flag, actual reuse, flag cuts, flag write/flush faults, ambiguous flag publication, no-write refusals, retained-snapshot COW fallback and both private-data tear matrices; opted-in in-place writes use the weaker torn-old oracle and never full-COW old/new bytes; [family_matrix_data_policy.rs](family_matrix_data_policy.rs): in-place write faults and ambiguous publication, retained-snapshot COW fallback, extending and shared-block fallback cuts and faults, staged demand of two nodes with 400 long names | Eviction of the in-place write's staged nodes, whose demand of two fits every bounded profile; a resource refusal of the in-place write |
| CloneFile / CloneRange | [shared_crash.rs](shared_crash.rs): first-clone and aligned range-boundary profile tests, `first_clone_io_failures_preserve_ownership_in_all_profiles`; [faults.rs](faults.rs): `completed_checkpoint_write_and_adoption_read_errors_block_mutations`; [tiny_cache_matrix.rs](tiny_cache_matrix.rs): `clone_publication_cuts_preserve_snapshot_namespace_in_all_profiles`; [shared_clone.rs](shared_clone.rs): partial-boundary and refusal tests | P: first CloneFile and aligned CloneRange cuts with exact bytes/refcounts; CloneFile I/O and ambiguous-publication faults; retained-snapshot CloneFile cuts; U: unaligned range boundaries and range refusals | P unaligned CloneRange, range-specific errors/retry and retained snapshots; forced eviction and resource refusals; retained clone mutation |
| Shared write/delete/final owner reuse | [shared_crash.rs](shared_crash.rs): `shared_write_split_is_crash_atomic`, `unlink_at_count_three_is_crash_atomic`, `unlink_at_count_two_never_reclaims_the_survivor`, `shared_storage_is_reused_only_after_the_last_owner_disappears`, `shared_write_io_failures_preserve_exact_ownership_and_allow_retry_in_all_profiles`, `shared_write_ambiguous_publication_requires_remount_in_all_profiles` | P: shared-write and reference-count cuts, exact survivor bytes, quarantine and final-owner storage reuse; shared-write write/flush faults with retry and ambiguous publication | Unlink and final-owner I/O-error retry and ambiguous publication; retained snapshots during shared transitions; forced eviction/reload failures (zero spill writes observed); deterministic resource and low-space refusal of the shared write itself |
| Orphan setup/move/open-target replace/cleanup | [orphans.rs](orphans.rs): `every_orphan_lifecycle_checkpoint_cut_recovers_to_an_allowed_state`, `every_open_target_replace_cut_is_old_or_new_namespace`, `fragmented_orphan_cleanup_is_extent_bounded_and_resumes_after_remount` | P: insertion/update/cleanup and open-target replace cuts with the preparatory orphan-directory checkpoint as an allowed state, retry from every cut, extent-bounded cleanup with exact tail-trimmed bytes; [family_matrix_orphans.rs](family_matrix_orphans.rs): insertion and cleanup faults with same-handle retry, ambiguous publication, retained snapshots, forced eviction with sampled cuts, insertion under exhausted ordinary allocation | Open-target replace and orphaned-file update faults, ambiguous publication, retained snapshots and eviction; cleanup under exhausted ordinary allocation; a `NoSpace` refusal of insertion is unreachable |
| Deferred namespace/write/truncate fsync | [cache_profiles.rs](cache_profiles.rs): `durable_window_recovery_honors_the_mount_profile`; [intent_log.rs](intent_log.rs): `existing_file_write_and_truncate_replay_in_order`, `successive_existing_writes_recover_only_monotone_prefixes` | P create replay, spills and idempotence; U data/truncate ordering | P durable data/truncate cut and replay-restart oracles |
| Deferred cancellation admission/ownership | [intent_log.rs](intent_log.rs): `cancellation_preflight_refusals_preserve_the_pending_window`, `cancellation_preflight_read_failure_keeps_the_window_retryable` | P: acknowledged/unlogged ownership, no-write refusal and read-failure retry | Other window entry failures; explicit mixed-family cut oracles |
| Shared deferred replay / orphan replay | [shared_crash.rs](shared_crash.rs): `durable_shared_unlink_replay_is_crash_atomic_and_idempotent`, `logged_write_replay_splits_shared_data_and_survives_replay_crashes`; [intent_replay_orphans.rs](intent_replay_orphans.rs) | P: shared unlink/replacement/write replay cuts, exact survivor/orphan bytes and repeat-remount idempotence; U: other orphan branches | P remaining orphan branches; interrupted fsync; I/O errors/retry, retained snapshots, forced eviction and resource refusals |
| Snapshot registry create/delete | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `snapshot_create_and_delete_publication_cuts_preserve_exact_membership_and_bytes`, `uncertain_snapshot_publication_blocks_mutation_and_remount_resolves_membership` | U: membership/bytes/cuts/poison; P normal create/delete and busy refusal in flight test | P registry faults/cuts and exhausted/admission limits |
| Snapshot lifetime / maintenance / selectable older view | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `constrained_tree_profiles_preserve_snapshots_and_shared_survivors`, `last_snapshot_deletion_preserves_older_selectable_view_during_the_next_write` | P: 192-entry spilled batch, exact captured bytes, clone survivor, both checkpoints; U: last-view deletion safety | P maintenance/release cuts and previous-slot protection failures |
| Snapshot mount/recovery | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `public_snapshot_mount_recovery_cuts_preserve_acknowledged_live_and_historical_bytes`, `public_snapshot_mount_validates_admission_before_pending_recovery_writes` | U: exact acknowledged live/historical state, bounded admission | P recovery interruption/idempotence and no-write refusal |
| Reclaim queue seal/consume/cursor and allocation rotation | [reclaim.rs](reclaim.rs): `crash_matrix_over_a_sealing_transaction`, `crash_matrix_over_segment_consumption_and_disappearance`, `crash_matrix_over_a_mid_run_cursor_advance`; [core src/volume.rs](../../afsplus-core/src/volume.rs): `allocation_cache_keeps_spilled_nodes_across_checkpoint_rotation` | P: sealing/consumption/cursor cuts with literal free/pending accounting and namespace bytes; P: allocation rotation with a retained snapshot and a shared run, exact live/captured/per-checkpoint bytes, spills at 2/4/8; [family_matrix_reclaim.rs](family_matrix_reclaim.rs): reclaim-step faults with same-handle retry, ambiguous publication and retained snapshot, promotion across eight allocation-root leaves with spills at 2/4/8 and sampled cuts, spilled create batch with two allocation-root nodes and sampled cuts | Faults, ambiguous publication and retained snapshots for the sealing, segment-consumption and cursor transitions; cuts of the multi-round rotation batch beyond its checkpoint-set comparison |
| Low-space refusal and progress | [allocation_pressure.rs](allocation_pressure.rs): `near_full_enospc_publishes_nothing_and_delete_can_recover_space`, `repeated_near_full_cow_and_reclaim_preserve_shared_survivors`, `near_full_delete_survives_every_modeled_power_cut` | P: deterministic no-write ENOSPC, same-size retry after reclaim, near-full COW/reclaim cycles with shared survivors; U: near-full delete cuts; [family_matrix_low_space.rs](family_matrix_low_space.rs): P near-full delete cuts, faults and ambiguous publication, forced eviction with two-page spills, ENOSPC of a spilled 64-file batch with unreachable provisional writes and a reclaim-drained retry | Eviction at four and eight pages, above the three-node demand; retained snapshots under low space; a data-block ENOSPC inside a spilled batch |
| COW tree spill/reload component | [core src/cow_tree.rs](../../afsplus-core/src/cow_tree.rs) staged-tree tests | 2/4/8 component: staged-node bounds/spill/reload | Not evidence for every mounted publication family or total-heap limits |
| Common uncertain commit tail | [faults.rs](faults.rs): `uncertain_checkpoint_publication_requires_remount_before_more_writes`, `completed_checkpoint_write_and_adoption_read_errors_block_mutations`; [core tests/flight.rs](../../afsplus-core/tests/flight.rs) | U semantic poisoning/reconciliation; P observed/unobserved failure equality; P family-matrix ambiguous modes for the in-place write, orphan insertion and cleanup, reclaim step and near-full delete with exact live, remounted and captured state | Family-specific P exact semantic oracle for the families outside the family-matrix runners, especially retained state |

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
cuts, faults and ambiguous publication), `eviction`, `ambiguous` and
`refusal`. Every cut budget is explicit, and an unflushed tail beyond it fails
the test.

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
