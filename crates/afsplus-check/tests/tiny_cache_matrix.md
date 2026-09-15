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
| Replace rename, shared target | [faults.rs](faults.rs): `replacement_write_and_flush_failures_preserve_complete_namespace_and_bytes`; [shared_crash.rs](shared_crash.rs): `rename_replace_of_a_shared_target_is_crash_atomic` | U: replacement bytes, sharing, faults/cuts | P same family oracles; spill/refusal cases |
| Symlink create/rename/unlink | [metadata.rs](metadata.rs): `symlink_namespace_preserves_target_through_metadata_rename_and_remount`, `symlink_publication_cuts_preserve_namespace_and_captured_target`, `symlink_refusals_and_short_reads_do_not_write` | U: exact opaque target, retained target, cuts and no-write refusals | P all three publication paths; forced eviction |
| Protection and preserved metadata | [metadata.rs](metadata.rs): `metadata_publication_cuts_preserve_exact_old_or_new_state_and_snapshot`, `metadata_refuses_open_windows_and_uncertain_publication_requires_remount`; [core tests/flight.rs](../../afsplus-core/tests/flight.rs): `api_snapshot_and_metadata_calls_preserve_captured_state_and_busy_refusals` | U: exact metadata/cuts/poison; P: protection and busy snapshot deletion with observed/unobserved image equality | P metadata restore cuts/faults; protection cuts; exact retained bytes in addition to metadata |
| Full-COW write, sparse write | [streaming_api.rs](streaming_api.rs): `seeded_mixed_io_preserves_bytes_across_remounts_and_clones`; [crash_matrix.rs](crash_matrix.rs): `every_crash_state_of_a_sparse_write_is_atomic` | U: independent mixed-byte oracle, sparse cuts | P direct/tree layouts, cuts/faults/refusals |
| Bounded write, reservation initialization | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `bounded_writes_crash_to_exact_bytes_and_preserve_captured_zeros`, `reservation_write_crashes_preserve_old_zeros_or_complete_new_bytes`, `reservation_write_io_errors_preserve_old_logical_zeros_and_require_reconciliation` | U: captured zeros, exact bytes, reconciliation | P admitted and rejected budgets, private-unwritten reuse, shared fallback, spill/recovery |
| Preallocation / bounded reservation | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `bounded_reservation_refusal_and_boundary_retry_preserve_layout`, `bounded_reservation_tree_publication_preserves_snapshot_at_every_cut` | U: allocation layout, refusal/retry, snapshot cuts | P exact layout/bytes and resource/fault cases |
| Truncate / sparse growth / bounded shrink | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `sparse_growth_publication_is_atomic_with_retained_reservations`, `bounded_shrink_crash_preserves_shared_and_captured_bytes`; [shared_crash.rs](shared_crash.rs): `truncate_across_private_and_shared_subruns_is_crash_atomic` | U: exact size/bytes/shared ownership and retained state | P direct/tree transitions, bounded refusal and cuts |
| In-place policy flag and private data | [data_policy_persistence.rs](data_policy_persistence.rs): `opt_in_persists_across_remount_and_takes_the_in_place_path`; [data_policy.rs](data_policy.rs): `in_place_crash_matrix_keeps_metadata_clean_but_allows_torn_old_data` | U: persistent flag, actual reuse, explicit weaker data contract | P flag cuts and private-data tear oracle; never apply full-COW old/new bytes to opted-in in-place writes |
| CloneFile / CloneRange | [shared_crash.rs](shared_crash.rs): `first_clone_is_crash_atomic`, `clone_range_with_multiple_reference_boundaries_is_crash_atomic`; [shared_clone.rs](shared_clone.rs): `clone_range_copies_partial_boundaries_and_shares_the_interior`, `unrepresentable_or_same_file_clone_range_is_rejected_without_a_commit` | U: exact bytes/refcounts, partial boundaries, refusals/cuts | P range boundaries, shared peers, all failures; component shared-survivor P test does not cover CloneRange |
| Shared write/delete/final owner reuse | [shared_crash.rs](shared_crash.rs): `shared_write_split_is_crash_atomic`, `unlink_at_count_three_is_crash_atomic`, `unlink_at_count_two_never_reclaims_the_survivor`, `shared_storage_is_reused_only_after_the_last_owner_disappears` | U: ownership and survivor bytes under cuts | P reference transitions and reclaim failures |
| Orphan setup/move/open-target replace/cleanup | [orphans.rs](orphans.rs): `every_orphan_lifecycle_checkpoint_cut_recovers_to_an_allowed_state`, `every_open_target_replace_cut_is_old_or_new_namespace`, `fragmented_orphan_cleanup_is_extent_bounded_and_resumes_after_remount` | U: allowed multi-checkpoint lifecycle and bounded cleanup | P lifecycle cuts and retries; preserve preparatory checkpoint as an allowed state |
| Deferred namespace/write/truncate fsync | [cache_profiles.rs](cache_profiles.rs): `durable_window_recovery_honors_the_mount_profile`; [intent_log.rs](intent_log.rs): `existing_file_write_and_truncate_replay_in_order`, `successive_existing_writes_recover_only_monotone_prefixes` | P create replay, spills and idempotence; U data/truncate ordering | P durable data/truncate cut and replay-restart oracles |
| Deferred cancellation admission/ownership | [intent_log.rs](intent_log.rs): `cancellation_preflight_refusals_preserve_the_pending_window`, `cancellation_preflight_read_failure_keeps_the_window_retryable` | P: acknowledged/unlogged ownership, no-write refusal and read-failure retry | Other window entry failures; explicit mixed-family cut oracles |
| Shared deferred replay / orphan replay | [shared_crash.rs](shared_crash.rs): `durable_shared_unlink_replay_is_crash_atomic_and_idempotent`, `logged_write_replay_splits_shared_data_and_survives_replay_crashes`; [intent_replay_orphans.rs](intent_replay_orphans.rs) | U: replay restart, shared survivor/orphan semantics | P replay cuts for each shared/orphan branch |
| Snapshot registry create/delete | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `snapshot_create_and_delete_publication_cuts_preserve_exact_membership_and_bytes`, `uncertain_snapshot_publication_blocks_mutation_and_remount_resolves_membership` | U: membership/bytes/cuts/poison; P normal create/delete and busy refusal in flight test | P registry faults/cuts and exhausted/admission limits |
| Snapshot lifetime / maintenance / selectable older view | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `constrained_tree_profiles_preserve_snapshots_and_shared_survivors`, `last_snapshot_deletion_preserves_older_selectable_view_during_the_next_write` | P: 192-entry spilled batch, exact captured bytes, clone survivor, both checkpoints; U: last-view deletion safety | P maintenance/release cuts and previous-slot protection failures |
| Snapshot mount/recovery | [core src/volume/snapshots/tests.rs](../../afsplus-core/src/volume/snapshots/tests.rs): `public_snapshot_mount_recovery_cuts_preserve_acknowledged_live_and_historical_bytes`, `public_snapshot_mount_validates_admission_before_pending_recovery_writes` | U: exact acknowledged live/historical state, bounded admission | P recovery interruption/idempotence and no-write refusal |
| Reclaim queue seal/consume/cursor and allocation rotation | [reclaim.rs](reclaim.rs): `crash_matrix_over_a_sealing_transaction`, `crash_matrix_over_segment_consumption_and_disappearance`, `crash_matrix_over_a_mid_run_cursor_advance`; [core src/volume.rs](../../afsplus-core/src/volume.rs): `allocation_cache_keeps_spilled_nodes_across_checkpoint_rotation` | U: reclaim cuts; 2: cache rotation regression | P queue transitions/cuts; retained/shared bytes across rotation |
| Low-space refusal and progress | [allocation_pressure.rs](allocation_pressure.rs): `near_full_enospc_publishes_nothing_and_delete_can_recover_space`, `repeated_near_full_cow_and_reclaim_preserve_shared_survivors` | U: ENOSPC and survivor/reclaim cycles | P deterministic ENOSPC and retry after reclaim |
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

## Commands and review boundary

```sh
export CARGO_HOME=/private/tmp/afsplus-cargo
export RUSTUP_HOME=/private/tmp/afsplus-rustup
export CARGO_TARGET_DIR=/private/tmp/afsplus-target-codex-cache
export CARGO_NET_OFFLINE=true
export PATH="$CARGO_HOME/bin:$PATH"
cargo test --offline -p afsplus-check --test tiny_cache_matrix -- --nocapture --test-threads=2
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
The baseline omissions outside the explicit added rows retain their own
qualification work: shared/range-clone transitions, private/unwritten data
policy cuts, orphan multi-checkpoint lifecycles, registry and maintenance
failures, reclaim cursor/sealing and cache split/collapse/reload failures.
This bounded test unit does not close `a-cache` or `roadmap-31`.
