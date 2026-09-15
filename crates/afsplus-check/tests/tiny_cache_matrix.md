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
| In-place policy flag and private data | [data_policy_persistence.rs](data_policy_persistence.rs): `opt_in_persists_across_remount_and_takes_the_in_place_path`, `policy_flag_publication_cuts_preserve_exact_choice_and_bytes_in_all_profiles`, `policy_flag_io_failures_preserve_exact_choice_and_retry_in_all_profiles`, `policy_flag_ambiguous_publication_blocks_mutations_until_remount_in_all_profiles`, `policy_flag_applicable_refusals_issue_no_writes_and_preserve_state_in_all_profiles`, `retained_snapshot_forces_cow_for_flagged_private_writes_in_all_profiles`, `in_place_write_after_a_crash_keeps_metadata_clean`; [data_policy.rs](data_policy.rs): `in_place_crash_matrix_keeps_metadata_clean_but_allows_torn_old_data` | P: persistent flag, actual reuse, flag cuts, flag write/flush faults, ambiguous flag publication, no-write refusals, retained-snapshot COW fallback and both private-data tear matrices; opted-in in-place writes use the weaker torn-old oracle and never full-COW old/new bytes | Forced eviction (zero spill writes observed); in-place data write faults beyond the single metadata-write error; ambiguous publication of an in-place data write; cuts of the shared-block and extending-write fallbacks |
| CloneFile / CloneRange | [shared_crash.rs](shared_crash.rs): first-clone and aligned range-boundary profile tests, `first_clone_io_failures_preserve_ownership_in_all_profiles`; [faults.rs](faults.rs): `completed_checkpoint_write_and_adoption_read_errors_block_mutations`; [tiny_cache_matrix.rs](tiny_cache_matrix.rs): `clone_publication_cuts_preserve_snapshot_namespace_in_all_profiles`; [shared_clone.rs](shared_clone.rs): partial-boundary and refusal tests | P: first CloneFile and aligned CloneRange cuts with exact bytes/refcounts; CloneFile I/O and ambiguous-publication faults; retained-snapshot CloneFile cuts; U: unaligned range boundaries and range refusals | P unaligned CloneRange, range-specific errors/retry and retained snapshots; forced eviction and resource refusals; retained clone mutation |
| Shared write/delete/final owner reuse | [shared_crash.rs](shared_crash.rs): `shared_write_split_is_crash_atomic`, `unlink_at_count_three_is_crash_atomic`, `unlink_at_count_two_never_reclaims_the_survivor`, `shared_storage_is_reused_only_after_the_last_owner_disappears`, `shared_write_io_failures_preserve_exact_ownership_and_allow_retry_in_all_profiles`, `shared_write_ambiguous_publication_requires_remount_in_all_profiles` | P: shared-write and reference-count cuts, exact survivor bytes, quarantine and final-owner storage reuse; shared-write write/flush faults with retry and ambiguous publication | Unlink and final-owner I/O-error retry and ambiguous publication; retained snapshots during shared transitions; forced eviction/reload failures (zero spill writes observed); deterministic resource and low-space refusal of the shared write itself |
| Orphan setup/move/open-target replace/cleanup | [orphans.rs](orphans.rs): `every_orphan_lifecycle_checkpoint_cut_recovers_to_an_allowed_state`, `every_open_target_replace_cut_is_old_or_new_namespace`, `fragmented_orphan_cleanup_is_extent_bounded_and_resumes_after_remount` | P: insertion/update/cleanup and open-target replace cuts with the preparatory orphan-directory checkpoint as an allowed state, retry from every cut, extent-bounded cleanup with exact tail-trimmed bytes | Orphan insertion/cleanup I/O-error retry and ambiguous publication; retained snapshots; spill observation and forced eviction; resource refusals |
| Deferred namespace/write/truncate fsync | [cache_profiles.rs](cache_profiles.rs): `durable_window_recovery_honors_the_mount_profile`; [intent_log.rs](intent_log.rs): `existing_file_write_and_truncate_replay_in_order`, `successive_existing_writes_recover_only_monotone_prefixes` | P create replay, spills and idempotence; U data/truncate ordering | P durable data/truncate cut and replay-restart oracles |
| Deferred cancellation admission/ownership | [intent_log.rs](intent_log.rs): `cancellation_preflight_refusals_preserve_the_pending_window`, `cancellation_preflight_read_failure_keeps_the_window_retryable` | P: acknowledged/unlogged ownership, no-write refusal and read-failure retry | Other window entry failures; explicit mixed-family cut oracles |
| Shared deferred replay / orphan replay | [shared_crash.rs](shared_crash.rs): `durable_shared_unlink_replay_is_crash_atomic_and_idempotent`, `logged_write_replay_splits_shared_data_and_survives_replay_crashes`; [intent_replay_orphans.rs](intent_replay_orphans.rs) | P: shared unlink/replacement/write replay cuts, exact survivor/orphan bytes and repeat-remount idempotence; U: other orphan branches | P remaining orphan branches; interrupted fsync; I/O errors/retry, retained snapshots, forced eviction and resource refusals |
| Snapshot registry create/delete | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `snapshot_create_and_delete_publication_cuts_preserve_exact_membership_and_bytes`, `uncertain_snapshot_publication_blocks_mutation_and_remount_resolves_membership` | U: membership/bytes/cuts/poison; P normal create/delete and busy refusal in flight test | P registry faults/cuts and exhausted/admission limits |
| Snapshot lifetime / maintenance / selectable older view | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `constrained_tree_profiles_preserve_snapshots_and_shared_survivors`, `last_snapshot_deletion_preserves_older_selectable_view_during_the_next_write` | P: 192-entry spilled batch, exact captured bytes, clone survivor, both checkpoints; U: last-view deletion safety | P maintenance/release cuts and previous-slot protection failures |
| Snapshot mount/recovery | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `public_snapshot_mount_recovery_cuts_preserve_acknowledged_live_and_historical_bytes`, `public_snapshot_mount_validates_admission_before_pending_recovery_writes` | U: exact acknowledged live/historical state, bounded admission | P recovery interruption/idempotence and no-write refusal |
| Reclaim queue seal/consume/cursor and allocation rotation | [reclaim.rs](reclaim.rs): `crash_matrix_over_a_sealing_transaction`, `crash_matrix_over_segment_consumption_and_disappearance`, `crash_matrix_over_a_mid_run_cursor_advance`; [core src/volume.rs](../../afsplus-core/src/volume.rs): `allocation_cache_keeps_spilled_nodes_across_checkpoint_rotation` | P: sealing/consumption/cursor cuts with literal free/pending accounting and namespace bytes; P: allocation rotation with a retained snapshot and a shared run, exact live/captured/per-checkpoint bytes, spills at 2/4/8 | Forced eviction during reclaim transitions (zero spill writes observed); reclaim-step I/O errors, retry and ambiguous publication; retained snapshots across reclaim transitions; modeled cuts of the spilled rotation batch |
| Low-space refusal and progress | [allocation_pressure.rs](allocation_pressure.rs): `near_full_enospc_publishes_nothing_and_delete_can_recover_space`, `repeated_near_full_cow_and_reclaim_preserve_shared_survivors`, `near_full_delete_survives_every_modeled_power_cut` | P: deterministic no-write ENOSPC, same-size retry after reclaim, near-full COW/reclaim cycles with shared survivors; U: near-full delete cuts | P near-full delete cuts; forced eviction under low space (zero spill writes observed); ENOSPC during spilled metadata allocation |
| COW tree spill/reload component | [core src/cow_tree.rs](../../afsplus-core/src/cow_tree.rs) staged-tree tests | 2/4/8 component: staged-node bounds/spill/reload | Not evidence for every mounted publication family or total-heap limits |
| Common uncertain commit tail | [faults.rs](faults.rs): `uncertain_checkpoint_publication_requires_remount_before_more_writes`, `completed_checkpoint_write_and_adoption_read_errors_block_mutations`; [core tests/flight.rs](../../afsplus-core/tests/flight.rs) | U semantic poisoning/reconciliation; P observed/unobserved failure equality | Family-specific P exact semantic oracle, especially retained state |

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

## Commands and review boundary

```sh
export CARGO_HOME=/private/tmp/afsplus-cargo
export RUSTUP_HOME=/private/tmp/afsplus-rustup
export CARGO_TARGET_DIR=/private/tmp/afsplus-target-codex-cache
export CARGO_NET_OFFLINE=true
export PATH="$CARGO_HOME/bin:$PATH"
cargo test --offline -p afsplus-check --test tiny_cache_matrix -- --nocapture --test-threads=2
for target in reclaim data_policy data_policy_persistence allocation_pressure shared_crash orphans; do
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
data policy cuts; orphan replay branches and orphan I/O faults; registry and
maintenance failures; reclaim-step faults and eviction during reclaim; and cache
split/collapse/reload failures. Interrupted fsync publication and resource or
low-space refusals beyond preallocation retain family-specific qualification.
Reclaim cursor/sealing cuts, private data policy cuts, orphan multi-checkpoint
lifecycles and shared write/replace faults have evidence in the sections above.
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
