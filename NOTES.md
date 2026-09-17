# Notes

The project journal: what was decided, tried and delivered, newest first.
This is the only document that narrates. Everything else states the finished
state and links here for the story; see
[docs/DOCUMENTATION.md](docs/DOCUMENTATION.md) for the rules.

Entry format: `## YYYY-MM-DD — title`.

<!-- toc -->

- [2026-09-17 — Read today's ADRs as a stranger: which claims nothing held](#2026-09-17--read-todays-adrs-as-a-stranger-which-claims-nothing-held)
- [2026-09-17 — Hold the claim ADR-115 made about explain](#2026-09-17--hold-the-claim-adr-115-made-about-explain)

- [2026-09-17 — A round of handler work costs nothing](#2026-09-17--a-round-of-handler-work-costs-nothing)
- [2026-09-17 — The feature registry lists what exists (ADR-116)](#2026-09-17--the-feature-registry-lists-what-exists-adr-116)
- [2026-09-17 — Retire what nothing writes (ADR-115)](#2026-09-17--retire-what-nothing-writes-adr-115)
- [2026-09-17 — The reserved header fields of the last five kinds (ADR-114)](#2026-09-17--the-reserved-header-fields-of-the-last-five-kinds-adr-114)
- [2026-09-17 — A finite acceptance inventory for Stage B](#2026-09-17--a-finite-acceptance-inventory-for-stage-b)
- [2026-09-17 — Carry extended attributes to FUSE and to AROS](#2026-09-17--carry-extended-attributes-to-fuse-and-to-aros)
- [2026-09-17 — The checkpoint flags word is zero (ADR-113); Q10 closed](#2026-09-17--the-checkpoint-flags-word-is-zero-adr-113-q10-closed)
- [2026-09-17 — A second reader for the snapshot records; Q15 closed](#2026-09-17--a-second-reader-for-the-snapshot-records-q15-closed)
- [2026-09-17 — The zero tail, once, in the header verification (ADR-112)](#2026-09-17--the-zero-tail-once-in-the-header-verification-adr-112)
- [2026-09-17 — Run the handler on AROS for the first time](#2026-09-17--run-the-handler-on-aros-for-the-first-time)
- [2026-09-17 — A selected checkpoint in the wrong form refuses the volume, in C too](#2026-09-17--a-selected-checkpoint-in-the-wrong-form-refuses-the-volume-in-c-too)
- [2026-09-17 — A second reader for the snapshot checkpoint payload (ADR-111)](#2026-09-17--a-second-reader-for-the-snapshot-checkpoint-payload-adr-111)
- [2026-09-17 — Exact admission for the reclaim queue blocks (ADR-110)](#2026-09-17--exact-admission-for-the-reclaim-queue-blocks-adr-110)
- [2026-09-17 — A second reader for the reclaim queue blocks](#2026-09-17--a-second-reader-for-the-reclaim-queue-blocks)
- [2026-09-17 — The remaining explain operations](#2026-09-17--the-remaining-explain-operations)
- [2026-09-17 — The afsplus-explain command](#2026-09-17--the-afsplus-explain-command)
- [2026-09-17 — Owned chains under persistent snapshots (ADR-109)](#2026-09-17--owned-chains-under-persistent-snapshots-adr-109)
- [2026-09-17 — Read extended attributes in the portable C reader](#2026-09-17--read-extended-attributes-in-the-portable-c-reader)
- [2026-09-17 — Extended attributes in the core (ADR-108)](#2026-09-17--extended-attributes-in-the-core-adr-108)
- [2026-09-17 — Let a record lock wait](#2026-09-17--let-a-record-lock-wait)
- [2026-09-17 — Reach the 64-bit groups from an application](#2026-09-17--reach-the-64-bit-groups-from-an-application)
- [2026-09-17 — Codecs for the extended attribute set and its reference](#2026-09-17--codecs-for-the-extended-attribute-set-and-its-reference)
- [2026-09-17 — Generalise the descriptor chain into an owned chain](#2026-09-17--generalise-the-descriptor-chain-into-an-owned-chain)
- [2026-09-17 — Explain one object and one path](#2026-09-17--explain-one-object-and-one-path)
- [2026-09-17 — Diff two images in filesystem terms](#2026-09-17--diff-two-images-in-filesystem-terms)
- [2026-09-17 — Read the object comment in the portable C reader](#2026-09-17--read-the-object-comment-in-the-portable-c-reader)
- [2026-09-17 — Carry the object comment to DOS](#2026-09-17--carry-the-object-comment-to-dos)
- [2026-09-17 — Store the object comment in the object record](#2026-09-17--store-the-object-comment-in-the-object-record)
- [2026-09-17 — Admit a well-formed security reference wherever it points](#2026-09-17--admit-a-well-formed-security-reference-wherever-it-points)
- [2026-09-17 — Pass a relabel's label as a value, never as volume state](#2026-09-17--pass-a-relabels-label-as-a-value-never-as-volume-state)
- [2026-09-17 — Make the volume label committed checkpoint state](#2026-09-17--make-the-volume-label-committed-checkpoint-state)
- [2026-09-17 — Pin the C statements of the format against the Rust codecs](#2026-09-17--pin-the-c-statements-of-the-format-against-the-rust-codecs)
- [2026-09-17 — Move the placement of the permanent areas into geometry](#2026-09-17--move-the-placement-of-the-permanent-areas-into-geometry)
- [2026-09-17 — Give the extent-map item one codec](#2026-09-17--give-the-extent-map-item-one-codec)
- [2026-09-17 — Explain one block with a walk that shares nothing with the checker](#2026-09-17--explain-one-block-with-a-walk-that-shares-nothing-with-the-checker)
- [2026-09-17 — Accept the four Stage B decisions as ADR-100 to ADR-103](#2026-09-17--accept-the-four-stage-b-decisions-as-adr-100-to-adr-103)
- [2026-09-17 — Bring the portable C reader to exact admission and the security container](#2026-09-17--bring-the-portable-c-reader-to-exact-admission-and-the-security-container)
- [2026-09-17 — Grow the AROS C boundary to interface revision 7](#2026-09-17--grow-the-aros-c-boundary-to-interface-revision-7)
- [2026-09-17 — Decide clone metadata inheritance and leave the clone source untouched](#2026-09-17--decide-clone-metadata-inheritance-and-leave-the-clone-source-untouched)
- [2026-09-17 — Carry opaque security descriptors through every object rewrite](#2026-09-17--carry-opaque-security-descriptors-through-every-object-rewrite)
- [2026-09-17 — Reserve an actor field in the epoch-1 change record](#2026-09-17--reserve-an-actor-field-in-the-epoch-1-change-record)
- [2026-09-17 — Admit object records only in their canonical image](#2026-09-17--admit-object-records-only-in-their-canonical-image)
- [2026-09-17 — Close a-cache and Stage A](#2026-09-17--close-a-cache-and-stage-a)
- [2026-09-17 — Fix window refusals that poisoned an open deferred window](#2026-09-17--fix-window-refusals-that-poisoned-an-open-deferred-window)
- [2026-09-16 — Fix intent-log replay reusing a logged data run](#2026-09-16--fix-intent-log-replay-reusing-a-logged-data-run)
- [2026-09-16 — Close a-fuzz and a-flight](#2026-09-16--close-a-fuzz-and-a-flight)
- [2026-09-16 — Integrate lifecycle observation, six generated families and the structure matrix](#2026-09-16--integrate-lifecycle-observation-six-generated-families-and-the-structure-matrix)
- [2026-09-15 — Integrate the family-matrix driver and four more cache families](#2026-09-15--integrate-the-family-matrix-driver-and-four-more-cache-families)
- [2026-09-15 — Integrate data-write cache families and generated operation families](#2026-09-15--integrate-data-write-cache-families-and-generated-operation-families)
- [2026-09-15 — Qualify reclaim, policy, low-space, shared-fault, orphan and rotation cache profiles](#2026-09-15--qualify-reclaim-policy-low-space-shared-fault-orphan-and-rotation-cache-profiles)
- [2026-09-15 — Complete typed caller and Unicode admission fixtures](#2026-09-15--complete-typed-caller-and-unicode-admission-fixtures)
- [2026-09-15 — Integrate subsystem observations and caller-property tests](#2026-09-15--integrate-subsystem-observations-and-caller-property-tests)
- [2026-09-15 — Identify allocator recorder ownership prerequisite](#2026-09-15--identify-allocator-recorder-ownership-prerequisite)
- [2026-09-15 — Qualify legacy one-block codec mutation targets](#2026-09-15--qualify-legacy-one-block-codec-mutation-targets)
- [2026-09-15 — Qualify captured state through clone publication](#2026-09-15--qualify-captured-state-through-clone-publication)
- [2026-09-15 — Enforce legacy codec reserved-zero fields](#2026-09-15--enforce-legacy-codec-reserved-zero-fields)
- [2026-09-15 — Add explicit exhaustive power-cut budgets](#2026-09-15--add-explicit-exhaustive-power-cut-budgets)
- [2026-09-15 — Qualify ambiguous clone publication across cache profiles](#2026-09-15--qualify-ambiguous-clone-publication-across-cache-profiles)
- [2026-09-15 — Qualify first-clone I/O failure recovery](#2026-09-15--qualify-first-clone-io-failure-recovery)
- [2026-09-15 — Qualify first-sharing CloneFile cache profiles](#2026-09-15--qualify-first-sharing-clonefile-cache-profiles)
- [2026-09-15 — Qualify CloneRange reference boundaries across cache profiles](#2026-09-15--qualify-clonerange-reference-boundaries-across-cache-profiles)
- [2026-09-15 — Qualify symlink and object metadata codecs](#2026-09-15--qualify-symlink-and-object-metadata-codecs)
- [2026-09-15 — Reject the reserved object payload byte](#2026-09-15--reject-the-reserved-object-payload-byte)
- [2026-09-15 — Qualify snapshot-bearing checkpoint admission](#2026-09-15--qualify-snapshot-bearing-checkpoint-admission)
- [2026-09-15 — Qualify reclaim codec mutation and replay](#2026-09-15--qualify-reclaim-codec-mutation-and-replay)
- [2026-09-15 — Enforce reclaim reserved-byte admission](#2026-09-15--enforce-reclaim-reserved-byte-admission)
- [2026-09-15 — Preserve and replay captured snapshot state](#2026-09-15--preserve-and-replay-captured-snapshot-state)
- [2026-09-15 — Qualify snapshot leaf and key codecs](#2026-09-15--qualify-snapshot-leaf-and-key-codecs)
- [2026-09-15 — Export and replay object-map diagnostics](#2026-09-15--export-and-replay-object-map-diagnostics)
- [2026-09-15 — Integrate the first cache-family qualification matrix](#2026-09-15--integrate-the-first-cache-family-qualification-matrix)
- [2026-09-15 — Review object resolution and preserve returned IDs](#2026-09-15--review-object-resolution-and-preserve-returned-ids)
- [2026-09-15 — Extend publication-family diagnostic comparisons](#2026-09-15--extend-publication-family-diagnostic-comparisons)
- [2026-09-14 - Export and replay API and deferred-window diagnostics](#2026-09-14---export-and-replay-api-and-deferred-window-diagnostics)
- [2026-09-14 - Map Stage A acceptance gates to visible roadmap entries](#2026-09-14---map-stage-a-acceptance-gates-to-visible-roadmap-entries)
- [2026-09-14 - Correlate deferred windows and qualify the real FSKit boundary](#2026-09-14---correlate-deferred-windows-and-qualify-the-real-fskit-boundary)
- [2026-09-14 - Preserve open transaction windows after preflight refusals](#2026-09-14---preserve-open-transaction-windows-after-preflight-refusals)
- [2026-09-14 - Compute stage progress from scoped acceptance gates](#2026-09-14---compute-stage-progress-from-scoped-acceptance-gates)
- [2026-09-14 - Correlate core API calls with checkpoint attempts](#2026-09-14---correlate-core-api-calls-with-checkpoint-attempts)
- [2026-09-14 — Replay selected diagnostics and bounded consumer delivery](#2026-09-14--replay-selected-diagnostics-and-bounded-consumer-delivery)
- [2026-09-14 — Filter commit diagnostics and attach a bounded live consumer](#2026-09-14--filter-commit-diagnostics-and-attach-a-bounded-live-consumer)
- [2026-09-14 — Exercise allocation codecs and reject undersized encoder output](#2026-09-14--exercise-allocation-codecs-and-reject-undersized-encoder-output)
- [2026-09-14 — Qualify seeded semantic properties and reconcile Stage A accounting](#2026-09-14--qualify-seeded-semantic-properties-and-reconcile-stage-a-accounting)
- [2026-09-14 — Avoid copying caller payloads into atomic batch staging](#2026-09-14--avoid-copying-caller-payloads-into-atomic-batch-staging)
- [2026-09-14 — Attribute requested allocations to their original context](#2026-09-14--attribute-requested-allocations-to-their-original-context)
- [2026-09-14 - Measure resident memory around workload phases](#2026-09-14---measure-resident-memory-around-workload-phases)
- [2026-09-14 - Bind internal commit diagnostics to semantic replay](#2026-09-14---bind-internal-commit-diagnostics-to-semantic-replay)
- [2026-09-14 - Observe the common checkpoint publication tail](#2026-09-14---observe-the-common-checkpoint-publication-tail)
- [2026-09-14 - Automate retained-host replay reconstruction](#2026-09-14---automate-retained-host-replay-reconstruction)
- [2026-09-14 - Reconstruct replay with copied tools and SDK](#2026-09-14---reconstruct-replay-with-copied-tools-and-sdk)
- [2026-09-14 - Rebuild replay with retained dependencies and an empty Cargo cache](#2026-09-14---rebuild-replay-with-retained-dependencies-and-an-empty-cargo-cache)
- [2026-09-14 - Preserve working sources for independent replay reconstruction](#2026-09-14---preserve-working-sources-for-independent-replay-reconstruction)
- [2026-09-14 - Compare reconstructed runners without weakening strict replay](#2026-09-14---compare-reconstructed-runners-without-weakening-strict-replay)
- [2026-09-14 - Reconstruct the semantic runner in an isolated checkout](#2026-09-14---reconstruct-the-semantic-runner-in-an-isolated-checkout)
- [2026-09-14 — Bind semantic replay and reduction to cache profiles](#2026-09-14--bind-semantic-replay-and-reduction-to-cache-profiles)
- [2026-09-14 — Integrate staged-tree cache profiles into transactions and recovery](#2026-09-14--integrate-staged-tree-cache-profiles-into-transactions-and-recovery)
- [2026-09-14 — Measure requested heap across real filesystem phases](#2026-09-14--measure-requested-heap-across-real-filesystem-phases)
- [2026-09-14 — Explain each completion-table entry](#2026-09-14--explain-each-completion-table-entry)
- [2026-09-14 — Require checker evidence in replay verdicts](#2026-09-14--require-checker-evidence-in-replay-verdicts)
- [2026-09-14 — Show completion on individual stage and phase entries](#2026-09-14--show-completion-on-individual-stage-and-phase-entries)
- [2026-09-14 — Bind selected crash states to replay bundles](#2026-09-14--bind-selected-crash-states-to-replay-bundles)
- [2026-09-14 — Integrate semantic replay bundles and failure reduction](#2026-09-14--integrate-semantic-replay-bundles-and-failure-reduction)
- [2026-09-14 — Add bounded semantic runner and bundle admission components](#2026-09-14--add-bounded-semantic-runner-and-bundle-admission-components)
- [2026-09-14 - Preserve bounded block-operation replay traces](#2026-09-14---preserve-bounded-block-operation-replay-traces)
- [2026-09-14 - Add bounded memory overlay branches and cut-state replay](#2026-09-14---add-bounded-memory-overlay-branches-and-cut-state-replay)
- [2026-09-14 - Measure host commands with per-child CPU and RSS](#2026-09-14---measure-host-commands-with-per-child-cpu-and-rss)
- [2026-09-14 - Prioritize usable stage outcomes across the complete queue](#2026-09-14---prioritize-usable-stage-outcomes-across-the-complete-queue)
- [2026-09-14 - Add bounded block-device partition views](#2026-09-14---add-bounded-block-device-partition-views)
- [2026-09-14 - Audit finite executable-core stage requirements](#2026-09-14---audit-finite-executable-core-stage-requirements)
- [2026-09-14 - Preserve scoped symlinks in bound archive groups](#2026-09-14---preserve-scoped-symlinks-in-bound-archive-groups)
- [2026-09-14 - Separate ongoing review from finite stage completion](#2026-09-14---separate-ongoing-review-from-finite-stage-completion)
- [2026-09-14 — Generate visible milestone and stage progress](#2026-09-14--generate-visible-milestone-and-stage-progress)
- [2026-09-14 - Add scoped symlink transport and VFS dispatch](#2026-09-14---add-scoped-symlink-transport-and-vfs-dispatch)
- [2026-09-14 — Refresh implementation navigation and remaining work](#2026-09-14--refresh-implementation-navigation-and-remaining-work)
- [2026-09-14 - Preserve symlink targets through core namespace transactions](#2026-09-14---preserve-symlink-targets-through-core-namespace-transactions)
- [2026-09-14 — Simplify ADR decision statuses](#2026-09-14--simplify-adr-decision-statuses)
- [2026-09-14 - Cross-read inline symlink records with Rust and C](#2026-09-14---cross-read-inline-symlink-records-with-rust-and-c)
- [2026-09-14 - Reject undersized object encoder buffers](#2026-09-14---reject-undersized-object-encoder-buffers)
- [2026-09-14 - Bind directory and alias archive restoration](#2026-09-14---bind-directory-and-alias-archive-restoration)
- [2026-09-14 - Reconcile the filesystem comparison with executable support](#2026-09-14---reconcile-the-filesystem-comparison-with-executable-support)
- [2026-09-14 - Reopen created restore entries with bounded active handles](#2026-09-14---reopen-created-restore-entries-with-bounded-active-handles)
- [2026-09-14 - Bind exact regular-file metadata to allocation and opaque inventories](#2026-09-14---bind-exact-regular-file-metadata-to-allocation-and-opaque-inventories)
- [2026-09-14 - Bind archive allocation records to verified sparse restoration](#2026-09-14---bind-archive-allocation-records-to-verified-sparse-restoration)
- [2026-09-14 - Add scoped allocation readback for full restore verification](#2026-09-14---add-scoped-allocation-readback-for-full-restore-verification)
- [2026-09-14 - Transport captured sparse contents through the archive consumer](#2026-09-14---transport-captured-sparse-contents-through-the-archive-consumer)
- [2026-09-14 - Exercise repeated low-space reclamation with retained snapshots](#2026-09-14---exercise-repeated-low-space-reclamation-with-retained-snapshots)
- [2026-09-14 - Bind full object inventories to counted descriptor manifests](#2026-09-14---bind-full-object-inventories-to-counted-descriptor-manifests)
- [2026-09-14 - Replay verified scratch archives with bounded upload slots](#2026-09-14---replay-verified-scratch-archives-with-bounded-upload-slots)
- [2026-09-14 - Carry opaque values through an authorized archive consumer](#2026-09-14---carry-opaque-values-through-an-authorized-archive-consumer)
- [2026-09-14 - Stage opaque destination metadata before atomic publication](#2026-09-14---stage-opaque-destination-metadata-before-atomic-publication)
- [2026-09-14 - Stream opaque captured attributes and security metadata](#2026-09-14---stream-opaque-captured-attributes-and-security-metadata)
- [2026-09-14 - Inspect captured metadata knowledge through backup authority](#2026-09-14---inspect-captured-metadata-knowledge-through-backup-authority)
- [2026-09-14 - Preserve exact object metadata and inventory knowledge](#2026-09-14---preserve-exact-object-metadata-and-inventory-knowledge)
- [2026-09-14 - Bind local PAX records to streamed ordinary members](#2026-09-14---bind-local-pax-records-to-streamed-ordinary-members)
- [2026-09-14 - Validate effective PAX member fields before restoration](#2026-09-14---validate-effective-pax-member-fields-before-restoration)
- [2026-09-14 - Bind archive integrity and termination to an independently checked envelope](#2026-09-14---bind-archive-integrity-and-termination-to-an-independently-checked-envelope)
- [2026-09-14 - Add streamed tar framing and independent recovery checks](#2026-09-14---add-streamed-tar-framing-and-independent-recovery-checks)
- [2026-09-14 - Add bounded PAX record parsing for the archive consumer](#2026-09-14---add-bounded-pax-record-parsing-for-the-archive-consumer)
- [2026-09-14 - Bound shrink working sets and retirements](#2026-09-14---bound-shrink-working-sets-and-retirements)
- [2026-09-14 - Grow sparse files without enumerating mappings](#2026-09-14---grow-sparse-files-without-enumerating-mappings)
- [2026-09-14 - Bound atomic writes for restore consumers](#2026-09-14---bound-atomic-writes-for-restore-consumers)
- [2026-09-14 - Bound reservation edits in fragmented files](#2026-09-14---bound-reservation-edits-in-fragmented-files)
- [2026-09-14 — Cross-read initialized reservations through the portable C reader](#2026-09-14--cross-read-initialized-reservations-through-the-portable-c-reader)
- [2026-09-14 — Initialize private unwritten reservations without replacement data allocation](#2026-09-14--initialize-private-unwritten-reservations-without-replacement-data-allocation)
- [2026-09-14 — Add checked destination reservation restoration](#2026-09-14--add-checked-destination-reservation-restoration)
- [2026-09-14 — Delegate decisions while the owner is away](#2026-09-14--delegate-decisions-while-the-owner-is-away)
- [2026-09-14 — Enumerate captured allocation and decide preservation modes](#2026-09-14--enumerate-captured-allocation-and-decide-preservation-modes)
- [2026-09-14 — Enforce separate destination restore grants](#2026-09-14--enforce-separate-destination-restore-grants)
- [2026-09-14 — Restore existing metadata through the common COW tail](#2026-09-14--restore-existing-metadata-through-the-common-cow-tail)
- [2026-09-14 — Enforce revocable authority on the backup interface](#2026-09-14--enforce-revocable-authority-on-the-backup-interface)
- [2026-09-14 — Configure snapshot limits before writable mount recovery](#2026-09-14--configure-snapshot-limits-before-writable-mount-recovery)
- [2026-09-14 — Protect both checkpoint generations during reclamation](#2026-09-14--protect-both-checkpoint-generations-during-reclamation)
- [2026-09-14 — Verify snapshot ownership and select stronger recovery retention](#2026-09-14--verify-snapshot-ownership-and-select-stronger-recovery-retention)
- [2026-09-14 — Integrate persistent snapshot lifecycle into Volume](#2026-09-14--integrate-persistent-snapshot-lifecycle-into-volume)
- [2026-09-14 - Bind lifetime accounting to allocator transactions](#2026-09-14---bind-lifetime-accounting-to-allocator-transactions)
- [2026-09-14 - Prepare transactional lifetime edits](#2026-09-14---prepare-transactional-lifetime-edits)
- [2026-09-14 - Read snapshot trees with bounded contextual checks](#2026-09-14---read-snapshot-trees-with-bounded-contextual-checks)
- [2026-09-14 - Bind snapshot roots in the checkpoint codec](#2026-09-14---bind-snapshot-roots-in-the-checkpoint-codec)
- [2026-09-14 - Add bounded key pages for persistent maintenance cursors](#2026-09-14---add-bounded-key-pages-for-persistent-maintenance-cursors)
- [2026-09-13 - Add persistent snapshot record codecs](#2026-09-13---add-persistent-snapshot-record-codecs)
- [2026-09-13 - Accept the integrated lifetime experiment and handle rule](#2026-09-13---accept-the-integrated-lifetime-experiment-and-handle-rule)
- [2026-09-13 - Add checkpoint extraction with explicit loss reporting](#2026-09-13---add-checkpoint-extraction-with-explicit-loss-reporting)
- [2026-09-13 - Prepare the integrated snapshot accounting decision](#2026-09-13---prepare-the-integrated-snapshot-accounting-decision)
- [2026-09-13 - Measure snapshot retention and reclaim traversal](#2026-09-13---measure-snapshot-retention-and-reclaim-traversal)
- [2026-09-13 — Prepare the complete implementation handoff](#2026-09-13--prepare-the-complete-implementation-handoff)
- [2026-09-13 — Exercise repeated pressure and define the snapshot experiment](#2026-09-13--exercise-repeated-pressure-and-define-the-snapshot-experiment)
- [2026-09-13 — Reconcile invariants and block writes after uncertain publication](#2026-09-13--reconcile-invariants-and-block-writes-after-uncertain-publication)
- [2026-09-13 — Complete BFS book review and fragmented allocation regression](#2026-09-13--complete-bfs-book-review-and-fragmented-allocation-regression)
- [2026-09-03 — Portable C allocates and logs its first COW data block](#2026-09-03--portable-c-allocates-and-logs-its-first-cow-data-block)
- [2026-09-03 — Portable C emits its first version-3 data mutation](#2026-09-03--portable-c-emits-its-first-version-3-data-mutation)
- [2026-09-03 — Portable C creates empty files without an allocator](#2026-09-03--portable-c-creates-empty-files-without-an-allocator)
- [2026-09-03 — Portable C trades caller RAM for 7x fewer writer reads](#2026-09-03--portable-c-trades-caller-ram-for-7x-fewer-writer-reads)
- [2026-09-03 — Intent replay makes final namespace removal bounded](#2026-09-03--intent-replay-makes-final-namespace-removal-bounded)
- [2026-09-03 — Portable C performs its first durable mutation](#2026-09-03--portable-c-performs-its-first-durable-mutation)
- [2026-09-03 — Q3 allocation architecture accepted](#2026-09-03--q3-allocation-architecture-accepted)
- [2026-09-03 — Emergency headroom makes ENOSPC recoverable](#2026-09-03--emergency-headroom-makes-enospc-recoverable)
- [2026-09-03 — Open-unlinked files gain a bounded crash-restartable lifetime](#2026-09-03--open-unlinked-files-gain-a-bounded-crash-restartable-lifetime)
- [2026-09-03 — Q3 qualification exposes the missing emergency reserve](#2026-09-03--q3-qualification-exposes-the-missing-emergency-reserve)
- [2026-09-03 — Checkpoint selection regains Rust/C parity](#2026-09-03--checkpoint-selection-regains-rustc-parity)
- [2026-09-03 — Prototype commands become integration-grade tools](#2026-09-03--prototype-commands-become-integration-grade-tools)
- [2026-09-03 — Rust codec failures gain stable case identities](#2026-09-03--rust-codec-failures-gain-stable-case-identities)
- [2026-09-03 — Checker corruption becomes a replayable corpus](#2026-09-03--checker-corruption-becomes-a-replayable-corpus)
- [2026-09-03 — Portable C overlays the durable namespace](#2026-09-03--portable-c-overlays-the-durable-namespace)
- [2026-09-03 — The per-file data policy becomes persistent](#2026-09-03--the-per-file-data-policy-becomes-persistent)
- [2026-09-03 — Portable C reads the durable log view](#2026-09-03--portable-c-reads-the-durable-log-view)
- [2026-09-03 — Portable C failures become replayable artifacts](#2026-09-03--portable-c-failures-become-replayable-artifacts)
- [2026-09-03 — The independent portable C reader starts executing](#2026-09-03--the-independent-portable-c-reader-starts-executing)
- [2026-09-03 — FSKit durability moves to write replies](#2026-09-03--fskit-durability-moves-to-write-replies)
- [2026-09-03 — Qualification order follows executable targets](#2026-09-03--qualification-order-follows-executable-targets)
- [2026-09-03 — macFUSE FSKit reopens the host fsync gate](#2026-09-03--macfuse-fskit-reopens-the-host-fsync-gate)
- [2026-09-03 — Logged data fsync reaches the portable Rust adapters](#2026-09-03--logged-data-fsync-reaches-the-portable-rust-adapters)
- [2026-09-03 — Intent-log writes and truncates survive nested recovery crashes](#2026-09-03--intent-log-writes-and-truncates-survive-nested-recovery-crashes)
- [2026-09-02 — Q1 and Q2 architecture blockers closed](#2026-09-02--q1-and-q2-architecture-blockers-closed)
- [2026-09-02 — Shared extents and reflinks qualified](#2026-09-02--shared-extents-and-reflinks-qualified)
- [2026-08-31 — Team board adopted](#2026-08-31--team-board-adopted)
- [2026-08-31 — Shared extents chosen as the next epoch-1 lot](#2026-08-31--shared-extents-chosen-as-the-next-epoch-1-lot)
- [2026-08-31 — Documentation restructured around one home per fact](#2026-08-31--documentation-restructured-around-one-home-per-fact)
- [2026-08-31 — Mountable Alpha-0 closed](#2026-08-31--mountable-alpha-0-closed)
- [2026-08-29 — Region allocator, bounded mount and shared COW trees](#2026-08-29--region-allocator-bounded-mount-and-shared-cow-trees)
- [2026-08-29 — Lesson from the first crash matrix, and the hardening list](#2026-08-29--lesson-from-the-first-crash-matrix-and-the-hardening-list)
- [2026-08-29 — First executable prototype](#2026-08-29--first-executable-prototype)

<!-- /toc -->

## 2026-09-17 — A round of handler work costs nothing

The handler's memory over time had never been measured. The DOS probe grew a
`STEADY <rounds>` mode: one round creates a file, opens it, reads it, locks
and unlocks a record, closes it, locks and examines the file, starts and ends
a notification, sets the comment and the protection, and deletes the file —
every operation paired with what releases it. A warm-up round runs first, so
what a first use allocates once is not counted; then the free memory of the
system is taken, the rounds run, and it is taken again.

Twenty rounds and a hundred rounds each left the free memory equal to the
byte: 239833360 before and after, 239833280 before and after, and 239654960
in the gate. Nothing is retained per operation by the handler, the packet
layer, the adapter or the core beneath them.

The probe does not merely report the two numbers, it fails a run that loses
more than a kilobyte. The bound is not zero because another task on the
system may allocate while the probe runs; it is small enough that a leak of
one allocation per round, which cannot be less than a few bytes, fails a run
of fifty rounds or more. `check-hosted-aros-dos.sh` runs a hundred rounds and
keeps the line as `steady.txt`, so the flat line is now a gate condition and
not an observation.

Peak memory remains unmeasured, and so does the benchmark runner C13 asks
for; what is gone from that entry is the steady half.

## 2026-09-17 — Read today's ADRs as a stranger: which claims nothing held

A sweep of the twelve ADRs written today (104, 106 to 116) for statements no
test could break. Most were already held, several by construction: ADR-104's
"the identification block is never rewritten" and its unchanged-label no-op
are in `volume_label`; ADR-106's "a directory listing reads the comment with
the record it already reads" follows from the comment living inside the record
block, which the golden-byte test holds; ADR-108's 17-block bound is held by
the largest-set reference of `attributes_c`; ADR-115's explain claim was held
this morning.

Three were not, and are now, each with a negative control that fails the test.
ADR-108 decision 3: "One set has one encoding, so two implementations that
hold the same attributes write the same bytes." `object_attributes` now
encodes what each accepted image decodes to and requires the image back, byte
for byte; a single pad byte appended by the encoder breaks it. ADR-112: "explain
reports a block with a dirty tail as having a valid checksum, which it has",
the sentence that separates the diagnostic from admission; `zero_tail` now
asserts it for every kind whose damage still leaves a walkable state, and
conflating the two in `explain.rs` breaks it. ADR-106 decision 2: "A symlink's
longest target shrinks by the comment's wire length"; `object_comment` now
takes the exact room for an empty, a 4-byte and a 255-byte comment and one
byte past it.

One claim is held only indirectly and the entry says so rather than pretending:
ADR-109's "a metadata block's header generation equals its ledger birth covers
chain segments". A segment whose generation disagrees fails the view walk
before that rule is reached, so the checker errors either way; the test proves
the error, not which rule produced it.

Alignment after ADR-115 and ADR-116 removed things: the milestone row for M01
said "remaining wire surfaces open", which Q15's closure made false, and now
names what is actually open (Unicode keys, the C writer's allocation and data
emission). The Stage B inventory had two rows about byte offsets, because the
reserved-field row I rewrote became a duplicate of the first; they are one row
that says the FIELDS are decided by ADR-110 to ADR-114 and the offsets are
not. The stale codec-target counts and the removed codecs' audit row went with
their own lots.

## 2026-09-17 — Hold the claim ADR-115 made about explain

ADR-115's procedure table claims that `afsplus_check::explain` gives a block
of a retired magic no identity and that nothing claims it. Nothing held that
claim, and an ADR statement no test holds is the failure mode of the day in
another form. `retired_magic` holds it now: a well-formed block of each of the
three retired magics, right header version and valid checksum, gets no
identity, no role, and leaves the checker's verdict unchanged, because what a
free block holds is not a claim about the filesystem. Negative control: put
one magic back in explain's known list and the test fails.

The second half of that claim, that an allocated block nothing claims is
reported as owned by nothing, is the general leak rule already held by
`zero_tail` and the explain tests: `is_unowned` is decided by the allocation
bit and the roles, never by content, so a retired magic cannot change it. The
test says so rather than building a contrived allocated case.

Also checked while waiting, and no lot needed: the feature registry is tied to
the Rust bits by `feature_registry` (ADR-116) and the Rust bits to the C spec
header by `c_constants`, which pins all seven, so the third copy cannot drift
either.

## 2026-09-17 — The feature registry lists what exists (ADR-116)

The "tbd" class of `org.aros.afsplus:data-checksums` was the visible end of a
larger drift. The registry held fourteen identities; the code assigns seven
bits. Of the other seven, one had no class and no design (the class follows
from a storage decision nobody has made), four were plans on paper, and two
described behaviour that is not optional: `xattrs`, although ADR-108 put
extended attributes in the base format with nothing gating them, and `sparse`,
although a hole is an absent extent every reader handles. A line calling
shipped base-format behaviour an optional feature is worse than a missing
line: a second implementer would gate the field on a bit no image sets.

The rule: an identity is registered when its bit, its class and its code land
together; base-format behaviour gets no identity; a plan lives in its ADR or
open question. Seven identities remain, `registry_version` is 6, and ADR-116
records for each removed line where its subject now lives, so nothing is lost
by deletion, only moved. docs/06 section 7 no longer claims the extent flag
namespace "reserves" a checksum bit: extent.rs refuses every flag outside the
two it defines, so what exists is room, not a reservation. docs/09 gave three
examples of the identity syntax that were three of the removed lines; it now
gives three that exist.

Proof: `feature_registry` (3 tests) reads spec/feature-registry.toml and
compares it with the constants of afsplus_format::ident in both directions,
and checks no bit of a word is used twice. It is the point of the lot: without
it the two drift again the moment nobody looks. Negative controls, each
failing it: the class of one identity changed, and one registry line removed
while its bit stays. Nothing else reads the registry; it is linked by two
chapters and two qualification plans and parsed by no tool.

## 2026-09-17 — Retire what nothing writes (ADR-115)

Survey first, and it decided the scope. Three prototype layouts still had
admission with nothing to read: identification versions 1 and 2 beside the 3
the formatter writes; intent-log record version 0 beside the 2 and 3 the
writer emits, found while implementing and reported; and the block kinds
`"AFSD"`, `"AFSM"` and `"AFSR"`, whose codecs, fuzz targets and magics
survived the typed COW tree. Nothing writes any of them, and a scan of every
file of the repository and all of build/ in three worktrees, 78,500 files, for
a block magic at a 4 KiB boundary found no image of any of them: no retired
magic, no identification block at all, no intent record outside versions 2
and 3. ADR-036 already called the AFSR codec "transitional test coverage
only".

Two precisions the survey caught that a grep on names would have got wrong:
dir.rs is half live, so only DirBlock went and the directory entry with its
tree-leaf codec stayed; and NameKeyAlgorithm::LegacyIdentity is not a legacy
version but the case-sensitive key algorithm of version 3, so it stayed.

ADR-115 says a retired version or magic is refused rather than ignored and is
never reused. The identification decoder now judges the version before the
length, since the version is what states the length; before, a 137-byte
prototype payload was refused as "too short", which is the right verdict for
the wrong reason.

Proof: `retired_surface` (3 tests): identification versions 0, 1, 2 and 4,
each at its own prototype length and at the current length so the version and
not the length decides; intent-log versions 0, 1 and 4; and, for the three
magics, that no current kind claims one and that a well-formed block carrying
one verifies as no current kind. Both gates pass: the Rust codec fuzz gate
with 18 targets and the portable C reader gate with the m68k step. The three
retired targets held the last IDs of the enum, so every surviving row of the
pinned fingerprint table is unchanged and the seed schema stays at 2.

Assertions that moved, none because an image changed: eleven cases of
roundtrip.rs that existed only for the retired codecs and versions,
legacy_reserved.rs and the legacy fuzz oracle, deleted with them. Two stale
statements found in testing/fuzzing.md on the way and corrected: the
snapshot-bearing checkpoint target was described with a 112-byte payload,
which ADR-104 made 184, and the document claimed "the 96-byte form stays
decodable", which has been false since the label field landed.

## 2026-09-17 — The reserved header fields of the last five kinds (ADR-114)

The question ADR-113 left open. Survey first, reported before any decoder
changed: probes that reseal a valid block with a valid checksum and a zero
tail, so ADR-112 does not decide them. Eight findings over four kinds. The
identification block admitted nonzero flags, a nonzero owner although it
always writes zero, a payload longer than its version's layout, and nonzero
bytes past its last field inside that longer payload; the intent-log record
admitted nonzero flags and a nonzero owner; the bitmap page and the region
descriptor admitted nonzero flags. The tree node was already clean.

One finding was a divergence, and the portable C reader was the stricter of
the two: it refuses a flagged bitmap page and a flagged region descriptor
where the core admitted them. Fourth divergence of the day, so the survey is
kept as a test that asks BOTH readers on every probe, through the entry point
that reaches each kind: `afspr_probe`, `afspr_lookup_object`,
`afspr_scan_intent_log`, and the writer's allocation search for the two kinds
the public reader never reads. The live bitmap page and region descriptor are
chosen by their explain role, because only the bound slot of three is read,
and the first attempt edited a dead slot and proved nothing.

ADR-114: zero flags on all five, zero owner on the two that belong to the
volume, and an identification payload exactly its version's layout. It also
answers what a later layout does, since a longer payload is how one would
arrive: it arrives as a new version with its own exact length, and a reader
that meets an unknown version refuses the volume rather than reading the
prefix it recognises.

Proof: `reserved_fields` in afsplus-check, nine probes, both readers, plus
the clean volume accepted through all five entry points. No existing assertion
moved: every test of afsplus-format, the fuzz unit tests (16), the Rust codec
fuzz gate (4,096 runs per target) and the portable C reader gate pass
unchanged.

## 2026-09-17 — A finite acceptance inventory for Stage B

Stage B had no scoped inventory, so its roadmap label fell back to M03 and
M04, which stay partial for work that belongs to M13 and M14. It now declares
eight gates in ROADMAP (`stage-gates: Stage B = ...`) with their rows in
`implementation/milestones.md`: allocation state, data-update policy,
durability, core structures, security container, C parity, explain, image
diff. A section "Stage B acceptance inventory" lists what the roadmap's B
sections mention and Stage B does not own, each with its owner: byte offsets
and the wire freeze, real-device barriers, portable C data emission, target
qualification through the AROS handler and the API documents, hardware
resource claims, descriptor formats and historical-read policy, the reserved
fields of five block kinds, a threat model for crafted images, backup
transport of chains. With all eight gates Complete, `progress-markers` strikes
Stage B in ROADMAP and in the implementation plan.

What this entry does not claim: the three gates for B1 to B3 cite decisions
and tests that predate this session, and were marked Complete on what the
roadmap and the qualification documents state, without being re-run here. The
five others cite tests written or extended today and run by name. Each of
those three documents names a full-size `--ignored` release run as its
evidence, so that run, not the ordinary one, is what holds the gate.

## 2026-09-17 — Carry extended attributes to FUSE and to AROS

The attribute set of ADR-108 got its two hosts. The portable interface passes
it through and publishes a capability that a snapshot-bearing volume lacks.
FUSE needed a naming rule, because Linux demands a namespace and macOS has
none: each stored name has one spelling per host and back, so nothing is
hidden and nothing collides, and the volume's other namespaces appear under a
prefix of their own. A real macFUSE mount driven by the host's `xattr` tool
showed that this backend never sends `REMOVEXATTR`: a removal arrives as a
`SETXATTR` without bytes, indistinguishable from an empty value, so the mount
has an explicit switch that reads it as a removal, the same kind of transport
workaround as the durable replies. AROS has no attribute packets, so the
attributes ride the extension packet: C boundary revision 15 with three entry
points, three operations, three client calls. The classic side writes `user.`
and `aros.` and only shows `security.` and `system.`, as it does with the
security descriptor. On the target the DOS probe drives all of it through a
real dos.library and leaves one attribute on the volume root, which the gate
reads back from the image with `afsplus-explain`.

## 2026-09-17 — The checkpoint flags word is zero (ADR-113); Q10 closed

Q10 asked for the epoch-1 rule of the checkpoint's reserved `flags` word.
Both readers already refused a nonzero word; ADR-113 records the rule and why
the other two options lose: removing the field moves every later field for
eight bytes, and a negotiated namespace has no customer, since what a reader
must know before trusting a checkpoint belongs in the identification block's
feature words. No code changed. The rule is held by `checkpoint_c` (the
"nonzero flags word" image, both forms, both readers), by `roundtrip` and by
the checkpoint fuzz oracle. Q10's wider demand stays with the M14 review for
the identification block, the tree nodes, the bitmap pages, the region
descriptors and the intent-log records.

## 2026-09-17 — A second reader for the snapshot records; Q15 closed

Last item of Q15. The portable C reader has five pure decoders for the
snapshot records of ADR-072: the 8-byte big-endian key, the registry control
record, the snapshot record, the lifetime record and the ledger control
record, each judged in the context the Rust codec uses (checkpoint generation,
volume size, first block of the run). The reader walks neither tree.
`spec/disk-layout.md` has a "Snapshot records" table; `c_constants` pins the
two tree kinds, the value size and the key size. Q15 is closed: the reclaim
blocks, the snapshot checkpoint payload and the snapshot records each have a
decoder in C, a cross-read test, pinned constants and a layout table.

Proof: `snapshot_c` in afsplus-format, 40 cases, 80 verdicts over a strict and
a sanitized build, each compared to the Rust verdict and to a literal: keys of
four lengths, every bound of every field at its edge and one past it, a run
whose end overflows, reserved bytes, short, long and empty values. Negative
controls, each failing it: C without the transaction-above-generation rule, C
without the overflow check of a run's end.

## 2026-09-17 — The zero tail, once, in the header verification (ADR-112)

Asked by claude-main after ADR-111: stop finding the zero-tail rule one block
kind at a time. Survey first, reported before any decoder changed: three built
volumes with all twelve block kinds in use (667 blocks with a valid checksum),
the four static fixtures, and a reading of the fourteen block encoders of the
format crate and the three of the C writer. No encoder writes past its
payload, so no image flips.

The rule now lives in `BlockHeader::verify` and `afspr_verify_header`. The
per-kind scans of the object record, the symlink record, the chain segments,
the checkpoint and the reclaim blocks are gone from both readers, with the
`tail_nonzero` message of `ChainKind`. `BlockHeader::checksum_matches` answers
the diagnostic question on its own, so explain still reports a dirty-tail
block as having a valid checksum. The independent fuzz oracles call the shared
header verification and inherit the rule; its second statement is the C
reader.

Four tests changed and no image did. Two named a per-kind message. One
resealed a checkpoint shorter, which now leaves payload bytes behind as a
tail. The fourth is a finding: `roundtrip`'s
`checkpoint_rejects_every_reserved_field` sealed a payload length of 96, the
layout before the label field, and had passed only because the header fields
were judged before the length; it seals 168 now.

Proof: `zero_tail` in afsplus-check (2 tests): the survey as a permanent test
that asserts every kind was seen, and one resealed byte after the payload of
the identification block, both checkpoint slots, the object-map node and the
object record of a real volume, each refused by the core and by the C reader.
Negative controls: without the check in the C header verification that test
and the four cross-read tests (`security_c`, `reclaim_c`, `checkpoint_c`,
`attributes_c`) fail; without it in the Rust one the lookup-path test fails.
Also run and passing: every test of afsplus-format (22 binaries), the fuzz
unit tests (16), and in afsplus-check `explain`, `explain_more`,
`security_container`, `security_c`, `corruption_corpus`, `volume_label`,
`chain_snapshots`, `extended_attributes`, `object_comment`, `mount_modes`,
`reclaim`.

## 2026-09-17 — Run the handler on AROS for the first time

A hosted darwin-aarch64 AROS was built on the development Mac and the three
Hosted gates ran against it: S0, S1 and the DOS semantics gate all passed by
the end of the day. None of what stood in the way was visible from a host
test. The build had no `S` directory and no `posixc.library`, which the Rust
standard library port opens at startup; the generated handler entry answers a
missing library with a requester, so the first access hung instead of
failing. Then `fdsk.device` died with an illegal instruction at addresses
whose bytes on disk were valid code. A debugger attached to the hosted
process showed zeros there, and a logging breakpoint on `munmap` showed the
device's code page being unmapped from inside its own first open:
`CreateNewProc()` tries a 31-bit allocation that cannot succeed on this host,
the failure runs the low-memory handlers, and lddemon expunges every device
with an open count of zero, the one being opened included. aros-apple-core
had the one-line fix; the fork got it as a local patch.

The DOS gate then passed its first boot, and its packet table, the handler's
own count of what dos.library sent, showed an adapter error number that
disagreed with `dos/dos.h` (`ERROR_COMMENT_TOO_BIG` is 220). Every adapter
error is now held to the header from both sides by one list. Its second boot
found the serious one: two handler tasks on one image. `RunHandler()` does not
serialise its callers, and two tasks that make their first access together
each get an instance. AFS+ now claims the backing device unit before opening
it; a later instance forwards its caller's packets and ends with the instance
it serves. An independent review of the first version found four edge
defects, among them that failing a second instance's startup makes
dos.library clear the first one's `dn_Task`; the claim became a unit with a
host matrix for those cases. On the target the double start still happens in
every run of that boot and is harmless, and a dismount leaves no handler task
behind. The generic fix was written as a patch, built and run: one handler
task in every run. It is not applied in the gate tree, so the gates keep
exercising the defence that does not depend on it.
## 2026-09-17 — A selected checkpoint in the wrong form refuses the volume, in C too

Follow-up to ADR-111, asked by claude-main: bind the checkpoint's payload form
to the persistent-snapshots feature. The core already did, for the selected
checkpoint, and `mount_modes` holds that it never steps past such a slot to
the older one. I first moved the check into candidate selection, which makes
the core fall back, saw that test, and undid it: a valid newest checkpoint in
the wrong form is not a torn write. The portable C reader was the one that
differed: it treated the slot as corrupt and used the older checkpoint. It now
keeps the slot as a candidate and refuses the volume when selection chooses
it. `Explainer::load` refuses the same image. ADR-111, not yet pushed, states
the rule and why a short-form checkpoint cannot exist on a snapshot volume
(identification is immutable; the formatter writes the first checkpoint in the
volume's form).

Proof: `volume_label` `the_portable_c_reader_reports_the_same_label_and_the_same_fallback`
now builds both images (wrong form in the newest slot: both readers refuse;
in the older slot: both report the current state) and fails with the old C
behaviour; `explain_more` holds explain to the core's verdict; `mount_modes`
unchanged.

## 2026-09-17 — A second reader for the snapshot checkpoint payload (ADR-111)

Second item of Q15. The C reader's checkpoint decoder was private and knew
the 168-byte payload only. It is now `afspr_decode_checkpoint_block`, public,
format-level (no geometry), for both payload lengths; the volume path calls it
and still refuses a checkpoint with snapshot roots, so an image with that
feature does not probe, as before.

The cross-read found a defect, not only slack. No reader looked past the
payload, so a snapshot checkpoint resealed with length 168 was admitted by
both readers as a plain checkpoint, its registry and ledger roots left in the
tail. ADR-111 (decided by claude-main under the rule it relays from the
owner): a nonzero byte after a checkpoint's payload is corrupt. Rust, C and
the fuzz checkpoint oracle apply it. `roundtrip.rs` and the fuzz oracle's unit
test had asserted the old reading and now assert the new one. No encoder
wrote such a byte; no fixture or fingerprint moves.

Proof: `checkpoint_c` in afsplus-format, 52 images, 104 verdicts over a strict
and a sanitized build. Negative controls, each failing it: C without the tail
scan, C admitting equal roots. Also run and passing: `roundtrip` (44),
`c_constants`, fuzz unit tests (16), and in afsplus-check `corruption_corpus`
(3), `snapshots` (1), `volume_label` (7), `security_c` (1), which go through
the changed decoders. Correction to the two entries below: the checker's
`reclaim` test has 8 tests and `corruption_corpus` 3; I had the counts swapped.

## 2026-09-17 — Exact admission for the reclaim queue blocks (ADR-110)

The finding of the reclaim cross-read, decided by claude-main under the rule
it relays from the owner: both readers now refuse common-header flags, an
owner, bytes in the unused slots of a root area, a root payload that is not
exactly its areas, and bytes after a payload. No encoder ever wrote such a
byte, so no image changes. Rust `reclaim.rs`, C `afspr_decode_reclaim_*` and
the independent fuzz oracle apply the same five rules; the proposal became
ADR-110, which amends ADR-036.

Proof: `reclaim_c` now holds 62 images (124 verdicts over two builds); the
thirteen images of the five families flipped from admitted to refused in both
readers in this commit. A first negative control passed when it should have
failed: the test had an unused table slot and no unused segment or inline
slot. Both were added, and the control now fails. Three controls fail the
test: C ignoring the owner, C admitting a long root payload, Rust ignoring the
unused inline slots. Unchanged and passing: `reclaim` (3) and
`corruption_corpus` (8) of the checker, `roundtrip` (44), the fuzz unit tests
with their fingerprints (16).

## 2026-09-17 — A second reader for the reclaim queue blocks

Q15 named three layouts stated by the Rust codec alone. The first is settled:
the portable C reader has `afspr_decode_reclaim_root`,
`afspr_decode_reclaim_segment` and `afspr_decode_reclaim_table`, heap-free,
borrowing the block, with `afspr_reclaim_ref_at` and `afspr_reclaim_entry_at`
for the areas they validated. It does not walk the queue: no read path needs
it. `spec/disk-layout.md` has the layout tables, `c_constants` pins ten new
constants, the m68k step compiles the code.

Proof: `reclaim_c` in afsplus-format: 60 images, each given to the Rust
decoder, to the C decoder in a strict and a sanitized build, and compared to a
literal verdict; for an accepted image the C fields and every item are
compared to the Rust ones. Roots with and without tables, empty, at capacity;
every field rule broken one at a time; a run whose end overflows; both sealed
kinds full, empty, long, short, and read as each other. Negative controls,
each failing the test: C without the overflow check, the cursor bound off by
one, a sealed payload not exact.

Finding, written as `proposals/adr-exact-reclaim-admission.md` and not
changed: neither reader looks at the common header's flags and owner, at the
unused slots of a root area, at a root payload longer than its areas, or at
the bytes after a payload. The test states those eleven images as admitted by
both, so the two readers cannot drift apart while the owner decides.

## 2026-09-17 — The remaining explain operations

`explain_extent`, `explain_checkpoint`, `explain_reclaim`, `explain_space` and
`explain_features` in `afsplus_check::explain` (file `explain/more.rs`), all
answered from the walk `Explainer::load` already makes; the walk now keeps
each file's extents, both slot states and the pending reclaim runs. The
command gained `extent <id> <offset>`, `checkpoint`, `reclaim [<block>]`,
`space <region>` and `feature [<id>]`. `EXPLAIN_SCHEMA_VERSION` is 2: the
documents gained kinds. A slot of zeros reads "never written" instead of a
checksum mismatch. `ExplainDirectory` is `explain_object` on a directory.
Two listed operations have no answer an image can give: a checkpoint
generation no slot carries any more, and reclaim by object, since a reclaim
entry records no owner. roadmap-36 is Complete; Stage B has no open roadmap
line.

Proof: `explain_more` in afsplus-check (3 tests). Extent: every block boundary
of a direct file, a sparse file with a preallocated run and its clone, against
the byte the core reads and the byte at the explained physical block; all five
states occur. Checkpoint: the core's `select_checkpoint` and fields, the slot
flip after one commit, a damaged slot, a fresh volume. Reclaim: the checker's
runs, every block of the volume asked. Space: three regions summing to the
volume, to the checkpoint's free count and to the queue, longest free run by
brute force over the core's bitmap. Features: three formatters against the
identification bits, and an unassigned bit. `explain_cli` (4 tests now) asks
the same through the command and parses the JSON with Python. Negative
controls, each failing its test: physical block off by one, feature lookup
ignoring the class, one-block runs dropped from the queue.

Found on the way: `afsplus-info` did not name `persistent-snapshots` and
`security-descriptors` among the enabled features. Fixed in
`afsplus-tools/src/common.rs`; `info_and_explain_name_the_same_enabled_features`
fails on the old list.

## 2026-09-17 — The afsplus-explain command

`afsplus-explain [--json] <image> (block <number> | object <id> | path
<path>)` in afsplus-tools, built like `afsplus-image-diff`: read-only image,
status 0 for an answer from a complete walk, 1 for a partial walk or a
question about something absent, 2 for usage or host I/O. The renderers live
in `afsplus_check::explain_render`; the JSON carries `schema_version` 1, the
generation, `has_snapshots`, `partial` and `problems`, then the answer. A
block gets a one-word verdict; on a snapshot volume an allocated block without
a live role reads `retained-or-leaked`, because explain walks the live state
only and the checker is the one that decides. Comment, symlink target,
security summary and attribute names with value lengths are shown.

Proof: `explain_cli` in afsplus-tools (2 tests): path, object and block in
both forms over an image with a directory, a file whose name holds quotes, a
comment with a tab and non-ASCII text, and two attributes; the JSON is parsed
by Python's `json`, not by our code, and field values are compared to
literals; eight failing invocations give their status with empty stdout; a
corrupted root directory gives `partial` true and status 1. Negative control:
without the quote escape in `json_string` the test fails.

## 2026-09-17 — Owned chains under persistent snapshots (ADR-109)

ADR-101 refused to mount descriptors with snapshots and ADR-108 refused
attributes on a snapshot volume, both waiting for the lifetime ledger to own
chain segments. It already did: the allocator records every allocation and
retirement of a transaction's namespace phase as a ledger run, whatever the
block holds. A probe with the refusal lifted gave a clean checker, so the lot
is proof and readers. Both refusals are gone;
`mkfs_with_snapshots_and_security_descriptors` formats both features. The
checker's walk of a retained view (`verify/snapshots.rs`) proves both chains
of every captured record at the view's generation and claims the segments as
historical metadata. New readers: `snapshot_attribute`,
`snapshot_attribute_names`, `snapshot_security_descriptor` (ApiMethod 79 to
81, `API_METHOD_MAX` 81). No format change.

Proof: `chain_snapshots` in afsplus-check (3 tests, 1,251 crash states); the
ADR lists what it covers. Negative control: without the historical chain walk
the checker test fails. Tests that asserted the refusals were removed
(`extended_attributes`, now 5) or cut to what still holds
(`security_container`, 13). Explain keeps its rule on a snapshot volume: a
block only a view reaches has no live role, and the test checks those blocks
are ledger-owned. The image diff reports no problem on such a pair.

Open: who may read a captured descriptor after live permissions change (Q5,
host policy); backup transport of captured chains.

## 2026-09-17 — Read extended attributes in the portable C reader

The C reader refused object flag bit 4 until now. `afspr_object_shape` admits
the 16-byte attribute reference between the security reference and the
comment, under the rules of the Rust codec. New public decoders in
`api/libafsplus_reader.h`: `afspr_decode_attribute_reference`,
`afspr_decode_attribute_segment` (the `"AFSX"` segment decoder, now one
function parameterised by block type and bound), `afspr_validate_attribute_set`
and `afspr_attribute_set_next`. No heap, no `strlen`: the m68k freestanding
step of the gate compiles it. `spec/afsplus_format.h` gained the flag and the
attribute limits; `c_constants` pins ten new constants against the Rust ones.
There is no volume-level attribute read in C: the caller walks the chain.

Proof: `attributes_c` (new; 136 image verdicts over a strict and a sanitized
build: references on the three object types, the eight combinations of the
three optional fields through their three decoders, segments, sets; each equal
to the Rust verdict and to a literal). Negative controls, each failing the
test: C admits a duplicate name, C ignores the reserved field of the
reference, C admits a count below the entries present. `security_c` in
afsplus-check now carries attributed objects of a real image through the C
volume lookup (flags 28, 20 and 16) and a resealed zero first block refused by
both. `c_constants`, `tools/check-portable-c-reader.sh` PASS with the m68k
step.

While answering claude-main's question on unreadable chains: explain already
separates absent, present and unprovable for both kinds (`segments_found`
below `segments_expected` plus a problem), and its descriptor walk did not
require one format identity per chain as the core does. It does now;
`a_descriptor_chain_with_two_format_identities_is_not_proven` in the explain
test fails without the guard.

## 2026-09-17 — Extended attributes in the core (ADR-108)

`Volume::attribute`, `attribute_names` and `set_attributes` (modes `Upsert`,
`Create`, `Replace`; `None` removes) store the attribute set of an object as
one `"AFSA"` owned chain. `volume/chain.rs` gained `replace_chain`, the
one-commit replacement the descriptor path already performed; both kinds use
it, and the descriptor path writes the same blocks in the same order as
before. A batch of changes to one object is one commit, whole or nothing; a
batch that leaves the set unchanged commits nothing. `CloneFile` copies the
set, both deletion paths retire the chain, and the data path keeps the record
flag across a layout rewrite. The checker walks the chain, decodes the set and
claims the blocks. `afsplus_check::explain` has the `AttributeSegment` role
and an `AttributeSummary` (names and value lengths) from its own walk. Errors
reuse `AlreadyExists`, `NotFound`, `InvalidMetadata` and `FeatureDisabled`,
so no adapter mapping changes. A volume with persistent snapshots refuses
`set_attributes`; ADR-108 names ledger ownership of owned chains as the next
core lot, owner claude-b. ApiMethod 76 to 78; `API_METHOD_MAX` is 78.

Proof: `extended_attributes` in afsplus-check (6 tests, 1,988 crash states,
each old set or new set with a clean checker verdict). Negative control: with
the delete retirement, the flag mask of the data path and the clone copy
removed, 4 of 6 fail. Also run: `explain` (4), `security_container` (13) and
`clone_metadata` for the refactored descriptor path, `api_coverage`.

Not in this lot: snapshot readers (nothing to read while snapshot volumes
refuse attributes), the portable C reader and the format header constants
(next lot), `diff.rs` (claude-main's file: it does not yet compare attribute
sets), the VFS and AROS surfaces (Stage C).
## 2026-09-17 — Let a record lock wait

A waiting `ACTION_LOCK_RECORD` used to answer `ERROR_LOCK_TIMEOUT` at once,
because a handler that blocks on one packet serves no other, the packet that
would free the range included. The packet layer now keeps such a packet
instead of answering it (packet ABI 4): `process` returns
`AFSPLUS_AROS_PACKET_DEFERRED`, the shell skips the reply, and the packet
comes back through a `complete` callback when a retry after a freed record or
a closed file grants it, when its ticks have passed, when its own file closes
or when the context is destroyed. Time stays outside the layer, which calls
neither Exec nor a device: the shell owns `timer.device` and reports elapsed
ticks, in steps of five, only while something waits. The wait is opt-in by
the callback, so a shell that could not open the timer keeps the old answer.
The stub pins the states: result fields untouched while deferred, expiry at
the exact tick, a retry that keeps its place and time, grant before the
freeing packet returns, sixteen waiters and the seventeenth refused; removing
the retry after a free fails the matrix. Not run on a target: a grant after a
release needs two tasks.
## 2026-09-17 — Reach the 64-bit groups from an application

The 64-bit entry points existed behind the C boundary with nobody able to
call them: an application talks to a handler in DOS packets. One packet type,
`ACTION_AFSPLUS_EXT`, now carries a request block to the packet layer, which
maps fifteen operations onto the existing entry points. One block for all
operations was preferred to one struct per operation because the envelope
checks, the 32- and 64-bit layout and the growth rule are then written once;
pointers take eight bytes everywhere so the layout is asserted as 112 bytes
on three targets. Objects travel as the application holds them, which keeps
the client free of handler knowledge and lets the packet layer refuse an
object that is not its own with the lookup it already had. The unknown-packet
answer of every existing handler is the capability probe, so nothing had to
be added for older handlers. The client library falls back only for
positioned I/O, where seek, transfer and seek back gives the same bytes; a
clone or a report that silently became something else would be a lie, so
those return the error. `AFSPlusInfo` is the first consumer. The packet
number is provisional and named as such in the header. Proven by the packet
and client host matrices, with two mutations that fail them, and
cross-compiled into the package; not run on a target.

## 2026-09-17 — Codecs for the extended attribute set and its reference

Format half of extended attributes, no writer yet. `afsplus_format::attrs`
holds the set codec and `ATTRIBUTE_CHAIN`, the owned chain of `"AFSA"` blocks
(bound 64 KiB, segment format 1 version 0). A set is one blob: entry count,
then entries in strictly ascending order of name bytes, each with a one-byte
name length, a two-byte value length, the name and the opaque value. Names are
1 to 255 bytes of NUL-free UTF-8 in one of `user.`, `system.`, `security.`,
`aros.`, with something after the namespace. An empty set is never stored.
The object record gains `attributes: Option<AttributeRef>` behind object flag
bit 4: 16 bytes (first block, set length, segment count, zero reserved) after
the security reference and before the comment, on every object type. The
independent fuzz oracle for the object payload learned the field.

Proof: `object_attributes` (6 tests: golden bytes of the reference and of the
set, order of the three optional fields, symlink target after the reference,
every malformed reference refused by encoder and decoder, every proper prefix
and extension of a set refused, order, duplicates, namespaces and bounds,
`"AFSA"` refused by the `"AFSX"` decoder). Negative control: with the reserved
check of the reference disabled and the order check relaxed to admit
duplicates, 2 of the 6 tests fail. Unchanged and passing: `object_admission`,
`object_comment`, `roundtrip` (44), fuzz unit tests with fingerprints (16).

Left for the core lot: `stage_file_layout` in `volume.rs` keeps a fixed mask
of record flags across a data rewrite and must add the attribute flag.

## 2026-09-17 — Generalise the descriptor chain into an owned chain

The security descriptor chain was the only chain of immutable blocks owned by
one object, and its codec, walk, staging and retirement named security
throughout. Extended attributes need the same container under another block
magic. The segment codec now lives in `afsplus_format::chain` and takes a
`ChainKind`: block magic, content bound, a label for damage reports and the
refusal messages. `SecuritySegment` delegates to it through `SECURITY_CHAIN`.
In the core, `volume/chain.rs` holds `walk_chain`, `load_chain`, `stage_chain`
and `retire_chain`; `volume/security.rs` keeps the descriptor API, the
projection rule and the reference flags.

No byte and no message changes. Proof: `owned_chain` (new, 3 tests: the
generic encoder equals the security encoder byte for byte, one kind refuses
the blocks of another, bound and messages belong to the kind), and unchanged
`security_container`, `security_c`, `c_constants` (format), `security_container`
(13), `security_c`, `clone_metadata`, `explain` (check), the fuzz unit tests
with their fingerprints (16), and `tools/check-portable-c-reader.sh` PASS.

## 2026-09-17 — Explain one object and one path

The leaf value of a directory tree existed in three private copies: the core's
directory adapter, the portable C reader and the image diff. It is one codec
now, `encode_tree_entry_value` and `decode_tree_entry_value` in
`afsplus_format::dir`, with a literal-byte test and nine malformed values; the
core keeps its key check and its error kinds and delegates the shape, and the
image diff decodes through it.

On that codec the explain walk keeps, for every object of the object map, its
record fields, comment, symlink target, tree node count, entry count, data
summary and descriptor chain state, and the names that reach it.
`explain_object` returns them and `explain_path` resolves a path component by
component under the stored spelling. The walk's chain check follows ADR-105:
one generation per chain. The test builds a file with an extent tree, a
comment, a two-segment descriptor, two names and a clone, a symlink and an
empty file, and compares every statement with a literal taken from how the
image was built and with the core's own `stat` and comment; a segment resealed
under a foreign owner gives one consistent segment of two and one reported
problem.

## 2026-09-17 — Diff two images in filesystem terms

The last open item of B4, the semantic image diff, answers what an operation
really did to a volume: `afsplus_check::diff` compares two committed states and
reports objects created and removed, names added, removed, retargeted and
moved, hard-link counts, type, size, allocation, protection, timestamps,
content generation, symlink target, the security descriptor down to its bytes,
the logical byte ranges whose content changed, the allocation changes that
change no content, and the volume facts: label, generation, free space,
reclaim quarantine, orphan directory and snapshot registry. `--metadata` skips
content. `afsplus-image-diff` prints the short human form or the versioned JSON
form of ADR-025.

It reads like the explain walk and not like the checker: checkpoint selection
and every tree descended with the `afsplus-format` codecs alone, so the
expectations below cross one implementation against the other. Trees are read
through a cursor holding one leaf and the pending child block numbers above it,
content one logical block per side, which keeps a directory or an extent map of
any size at one block of items per image and leaves the report as the only term
that grows with the number of changes. Two design questions were settled the
simple way, with the reasons in
[docs/28](docs/28-virtual-images-and-viewports.md): content is compared by
reading bytes rather than by trusting equal physical mappings, since two images
need not be copies of one another, and one removed name plus one added name for
the same object is a move whatever the sequence that produced it.

What a damaged tree hides stays uncompared. A zeroed object map or directory
first reported a volume of removals; the stream now marks itself broken and the
diff reports the problem instead of concluding anything one-sided from it, and
the command returns its media status rather than "no difference".

`crates/afsplus-check/tests/image_diff.rs`, 21 tests: image pairs built with
the real core for create, write, truncate, rename across directories, hard link
and unlink, symlink, `clone_file`, `clone_range`, preallocate, protection,
security descriptor and its replacement, relabel, a deferred window, an orphan
and a snapshot, each against the literal consequence of the operations. Every
pair also proves that an image does not differ from itself, that the reversed
diff is the mirror of the diff, and that the reported byte ranges equal a
brute-force comparison of the two files read through the core. A record
resealed under another size is reported as the size change and the seven bytes
behind it. With one range end moved by a single byte the write expectation
fails, which is what makes the passing run mean something.
`crates/afsplus-tools/tests/image_diff_cli.rs`, 2 tests, covers the command,
its JSON and its three exit statuses.



## 2026-09-17 — Read the object comment in the portable C reader

The shared shape check of the portable C reader parses the comment field of
[ADR-106](adr/ADR-106-stored-object-comment.md) after the security reference:
length byte of 1 to 255, NUL-free UTF-8, and the exact payload length the
comment implies, with a symlink's target behind it. The volume lookup admits
commented objects and carries flag bit 3; `afspr_decode_object_comment` is the
standalone decoder.

`crates/afsplus-format/tests/security_c.rs` adds 25 comment images, read by
both codecs in a strict and a sanitizer build: files and directories with a
short and a 255-byte comment, with and without a security reference, seven
resealed corruptions per type, and one symlink block filled to its last byte
by a security reference, a 255-byte comment and a 3,696-byte target, for
which one more target byte has no encoding. The first run found a
disagreement: the standalone C decoders admitted a record with an unassigned
object flag bit, which the Rust decoder and the C volume path refuse. The
flag-namespace check now sits in the shared shape function.
`crates/afsplus-check/tests/security_c.rs` requires the C lookup and the Rust
core to agree on a commented file and on a file with both a descriptor and a
comment. The comment in the explain output belongs to `ExplainObject`.

## 2026-09-17 — Carry the object comment to DOS

`ACTION_SET_COMMENT` and the comment of `FileInfoBlock` and `ExAllData` run
through `Vfs::comment` / `set_comment`, the AROS adapter, C boundary revision
14 (group `DOS_COMMENT`, three entry points) and the packet layer. Two bounds
meet: DOS holds 79 characters, the record 255 UTF-8 bytes. The packet layer
owns the DOS bound because it knows the width of `fib_Comment`; the adapter
owns the stored one. Reading was made total on content: a comment written
through FUSE or the portable API may be longer or carry characters outside
Latin-1, and a directory scan that failed on one such entry would hide the
whole directory, so the report is cut at a character boundary and substitutes
`?`. A read error is a different matter and fails the request. `ExamineFH`
has no lock to name the object by, hence a separate by-file entry point. The
stub proves the fit arithmetic of `ED_COMMENT` records with a buffer one byte
short, and removing the comment length from that arithmetic fails exactly
that assertion. Proven on the host and cross-compiled for AROS AArch64; no
target run yet.

## 2026-09-17 — Store the object comment in the object record

[ADR-106](adr/ADR-106-stored-object-comment.md) gives the comment a home:
object flag bit 3 and, after the fixed payload and any security reference, one
length byte and up to 255 bytes of NUL-free UTF-8, before a symlink's target.
A directory listing returns the comment of every entry, so it sits in the
record the listing already reads. `Comment` is a `Copy` field of
`ObjectRecord`, so every read-modify-write carries it; layout staging keeps the
flag bit. `set_object_comment` is one metadata commit with the value passed as
an argument, `object_comment` and `snapshot_object_comment` read it, and
`CloneFile` copies it. [ADR-107](adr/ADR-107-twelve-byte-timestamps.md) records
the 12-byte timestamp.

Proof: three wire tests in `crates/afsplus-format/tests/object_comment.rs`
(literal bytes at 96 and at 112, the symlink target after the comment, the
bound, congruence, six resealed corruptions); four tests in
`crates/afsplus-check/tests/object_comment.rs`: ten rewrite paths on a file
with a descriptor beside the comment, a directory, a symlink and the root
across remount, the clone copy and its independence, bounds and refusals, a
snapshot that keeps the captured comment, and 394 modeled power cuts over
setting and removing a comment, each mounting to the old or the new comment
with a clean checker verdict. The fuzz oracle models the field and the
constant-pinning test holds the two new header constants. The portable C
reader refuses a commented record until its parity lot, which is the fail-closed
verdict of the validated flag namespace.

## 2026-09-17 — Admit a well-formed security reference wherever it points

An independent reading of the security follow-ups found two things, recorded
in [ADR-105](adr/ADR-105-security-reference-admission.md). The lookup bound on
the first segment block made an object with a reference one block past the
volume undeletable, while a reference into in-range garbage left it deletable.
Record admission now judges the shape of the reference only, in the Rust read
path and in the portable C reader; where it points is chain state. And the
chain walk accepted a stale segment of an earlier descriptor of the same
object and size, since every field it compared matched: through a forged,
resealed next pointer the descriptor read returned a mixture of two
descriptors and retirement freed blocks the replacement had already retired.
Every segment of a chain now carries the generation of its first segment,
which one commit guarantees.

`crates/afsplus-check/tests/security_container.rs`: the out-of-volume
reference is admitted, the object stats, reads and deletes, its descriptor
read is corrupt, and the one real segment is the checker's only leak; the
stale-segment image ends the walk at the forged link, deletion frees the first
live segment and leaks the two behind it. With the generation comparison
disabled that test fails on the descriptor read, which returns data.
`crates/afsplus-check/tests/security_c.rs` requires both readers to admit the
out-of-volume reference. The remaining limit, a data block that holds a valid
segment image behind a forged link, is Q16 in
[open questions](implementation/open-questions.md).

## 2026-09-17 — Pass a relabel's label as a value, never as volume state

An independent read of the label change found that `set_volume_label` parked
the new label in a `Volume` field for the commit tail to pick up and cleared it
on the returned result. An unwind inside the commit skips that clearing, and
the next unrelated commit of the same `Volume` value would have published a
label nobody asked for. The field is gone: the label is an argument of the
commit tail, beside the snapshot registry change, and every other caller
passes none. The four remaining transaction-scoped `pending_*` counters are
statistics that `next_generation` zeroes at the start of every transaction, so
none of them carries intent across an unwind.

`crates/afsplus-check/tests/volume_label.rs` gains three tests: a device that
panics on the next write unwinds out of a relabel, after which a further
commit on the same value and a remount both show the committed label; a
relabel followed by intent-logged writes, a cut and replay at mount keeps the
label; and a relabel through `Volume` on a snapshot-bearing volume keeps the
snapshot roots, the captured bytes and a clean checker verdict. The stale
label reader in the FUSE mount tool, found by the same review, is fixed in its
own lot.

## 2026-09-17 — Make the volume label committed checkpoint state

The label had one home, the identification block, which exists in one copy
and is written once, so a relabel had no state with two legal outcomes under
a power cut. [ADR-104](adr/ADR-104-volume-label-in-checkpoint.md) puts the
current label in the checkpoint payload at offset 96 (length, seven zero
bytes, 64 bytes of NUL-free UTF-8, zero padded); the payload is 168 bytes, or
184 with the snapshot roots after the label. The identification block keeps
the format-time label and is never rewritten. `Volume::set_volume_label` is
one commit that changes nothing else, and every commit carries the label
forward. The checker, `afsplus-info`, `afsplus-dump` and the JSON volume
summary report the committed label. The portable C reader decodes the field
under the same canonical rule and reports the committed label in its probe
result.

Proof: literal bytes, bounds, seven resealed non-canonical fields and six
refused payload lengths in `crates/afsplus-format/tests/checkpoint_label.rs`;
durability across remount and later commits, refusals, a read-only mount and
68 modeled power cuts that each mount to the old or the new label with a clean
checker verdict in `crates/afsplus-check/tests/volume_label.rs`, where the C
reader also reports the same generation and label as the core, including the
fallback to the older slot for four resealed label fields. The independent
Python generator assembles the two new checkpoint images byte for byte equal
to the Rust encoder; the two images of the retired layout are refused
negatives. `tools/check-portable-c-reader.sh` passes on the new layout.

The fuzz crate's checkpoint oracle models the field. Its mutation run found
that the identification decoder admitted a label with an embedded NUL that
the encoder, now sharing the label rule, refused to write back; the decoder
and the C reader apply the rule too. The changed checkpoint seeds moved the
seed fingerprints of the two checkpoint targets, so the seed schema version
is 2 and all fingerprints are pinned again.

## 2026-09-17 — Pin the C statements of the format against the Rust codecs

`struct afsp_timespec_wire` in the format header was 16 bytes with a reserved
field that no codec writes: the executable format, both readers and every
image use 12-byte times, three of them in 36 bytes of the object record. The
structure was referenced by nothing, so nothing noticed. It is 12 bytes now;
the wire is unchanged.

`crates/afsplus-format/tests/c_constants.rs` compiles two probes and requires
the printed map to equal the Rust side exactly, so an unpaired constant on
either side fails too. `spec_probe.c` prints the 24 constants and both
wire-structure sizes of the header. `reader_constants_probe.c` includes
`reader.c` and prints the 43 format constants the portable reader compiles
with as private macros: header size, checksum offset, eight block magics,
payload sizes, region bounds, slot counts, tree kinds and limits, log limits
and the security sizes. Rust values come from public constants, or from the
payload length of a block the codec just encoded where the constant is
private. The header had that one mismatch; the 43 macros all agree and were
pinned by nothing before. The object record now has an offset table in
[docs/04](docs/04-object-model.md). The layouts no C code reads (snapshot
checkpoint payload, reclaim structures, snapshot records) are Q15 in
[open questions](implementation/open-questions.md).

## 2026-09-17 — Move the placement of the permanent areas into geometry

The positions of the bootstrap metadata, the allocation-root pool and the
intent-log slots are derived in `afsplus_format::geometry`
(`allocation_root_logical_nodes`, `reserved_run`, `allocation_root_pool_lbas`,
`intent_log_slot_lbas`) and written as a rule of the
[disk layout](spec/disk-layout.md). ADR-035, ADR-036 and ADR-037 state the
rule in prose and agree with the code: four bootstrap blocks since the
reclaim-queue root, the `3N` pool after them, the log slots directly after the
pool. The core's allocation-root and intent-log modules delegate to geometry
and keep their error kinds; the explain walk dropped its import from the core
and depends on `afsplus-format` only. The portable C reader keeps its own
derivation of the log slots, which its gate compares against Rust images.

`crates/afsplus-format/tests/placement.rs` pins literal positions: blocks 9 to
12, 13 to 15 and 16 to 23 on a four-region volume of 512-block regions; a log
run that leaves region 0 at block 15 and continues at 22, then at 38, on
16-block regions; a pool of 9 blocks once the regions need a second leaf; and
refusal on a volume too small. The pool-bounds and snapshot-allocator tests of
the core, the 18 intent-log tests and the explain tests pass on it.

## 2026-09-17 — Give the extent-map item one codec

The extent-map item (eight-byte big-endian logical start; 24-byte value with
physical start, nonzero block count, flags and four zero reserved bytes) is
defined once, in `afsplus_format::extent` and as
`struct afsp_extent_value_wire` in the format header, whose flag enum already
named unwritten as bit 0 and shared as bit 1. The core's extent adapter keeps
its contextual range checks and its error messages and delegates the shape to
the codec; the explain walk decodes through it; the portable C reader takes
the value size and the two flag bits from the header and exposes its shape
check as `afspr_decode_extent_item`. The wire image is unchanged.

`crates/afsplus-format/tests/extent_c.rs` pins the literal image and drives a
sanitizer-built C probe over 16 items: the four flag combinations, the last
representable block, zero count, two unknown flag bits, each reserved byte,
short and long values, a short key and an overflowing run. The C verdict, the
Rust verdict and the literal expectation agree on each, and a wrong
expectation makes the probe report a mismatch. The explain tests, the 17
shared-clone tests and `tools/check-portable-c-reader.sh` pass on the changed
reader; that script skips its m68k compile step on this host, where no m68k
compiler is installed.

## 2026-09-17 — Explain one block with a walk that shares nothing with the checker

`afsplus_check::explain` answers `ExplainBlock` for the committed state: the
allocation bit, every role of the block and the block's own identity. Its walk
selects the checkpoint, descends the allocation root, the object map, every
directory and extent tree, the shared-extent and snapshot trees, every
security descriptor chain segment by segment, and the reclaim queue with its
head cursor, using the format codecs only.

Three tests in `crates/afsplus-check/tests/explain.rs`. On an image holding a
direct file, a sparse extent-tree file, a 300-entry directory, a symlink, a
clone with shared data and a copied three-segment descriptor chain, an orphan
in directory 2, a reclaim quarantine and a file unlinked over a damaged chain,
the explanation of each of the 2,048 blocks agrees with the checker's data,
metadata and quarantine sets, with the live bitmap, and the unowned blocks are
exactly the checker's two leak findings, both still `"AFSX"` blocks; the 12
written data blocks of four files hold the bytes the core reads at the
attributed offsets; both owners appear on each of the four shared blocks. A
record resealed under a foreign owner makes the walk report one problem and
leave the lost directory tree unowned. On a snapshot volume the unowned blocks
are a subset of the blocks the checker says a retained view owns.

The first run of the comparison found two facts about where format knowledge
lives, and no defect in the checker. The positions of the allocation-root pool
and of the intent-log slots are derived by formulas in `afsplus-core` and
appear in no codec and no specification table, so explain takes them from the
core. The extent-map value (physical start, block count, flags, with unwritten
as bit 0 and shared as bit 1) is encoded in `afsplus-core` and in the portable
C reader and has no codec in `afsplus-format`; the walk first had the two bits
reversed, and the shared-block assertion of its own test caught it.

## 2026-09-17 — Accept the four Stage B decisions as ADR-100 to ADR-103

The four Stage B proposals are accepted and numbered:
[ADR-100](adr/ADR-100-exact-object-record-admission.md) exact admission of
object records, [ADR-101](adr/ADR-101-security-preservation-container.md) the
security preservation container,
[ADR-102](adr/ADR-102-clone-metadata-inheritance.md) clone metadata
inheritance and [ADR-103](adr/ADR-103-change-record-actor.md) the reserved
actor field of the change record. ADR-101 amends ADR-031, ADR-102 amends
ADR-027 and ADR-103 amends ADR-013; ADR-031 keeps its Proposed status for the
canonical ACL candidate it still describes.

The accepted texts carry the state at acceptance: `CloneFile` copies the
descriptor chain, a damaged chain never makes its object undeletable, the
AROS handler defaults to the preserve projection policy with strict as a
mount flag, both readers bound the first segment where they admit the record,
and the portable C reader shares the admission rule, so the C parity items
left the open lists. Q13 and Q14 are closed and the format half of Q5 is
closed in [open questions](implementation/open-questions.md). The decisions
landed in their target documents: the admission rule and the security
reference in [docs/04](docs/04-object-model.md) and the
[disk layout](spec/disk-layout.md), the projection rule and the clone copy in
[docs/30](docs/30-portable-security-model.md), the clone metadata table in
[docs/32](docs/32-reflink-clone-semantics.md), the actor header in
[docs/11](docs/11-change-stream.md) and ROADMAP B5. The C volume test checks
the out-of-range reference against both readers. The filesystem API documents
are unchanged; their consequences are listed in ADR-101, ADR-102 and ADR-103.

## 2026-09-17 — Bring the portable C reader to exact admission and the security container

The portable C reader shares the object admission rule of the Rust decoder.
One shape check serves its generic and its symlink decoder: exact payload
length by the security-reference flag, a well-formed 16-byte reference and a
zero tail. Before it the C reader refused common-header flags and admitted a
longer payload and a nonzero tail, and it failed closed on object flag bit 2.
The volume path requires INCOMPAT bit 3 and an allocatable first segment for a
reference. Two standalone heap-free decoders return the reference of a record
and one `"AFSX"` segment. The C writer appends intent records and rewrites no
object record, so it has no path that drops a reference.

`crates/afsplus-format/tests/security_c.rs` drives a C probe over 67 object
images and 10 segment images in a strict and a sanitizer build; the C
verdict, the Rust verdict and a literal expectation agree on each, every
shorter buffer is refused, outputs stay untouched on error, and a wrong
expectation makes the probe report a mismatch.
`crates/afsplus-check/tests/security_c.rs` checks the same agreement through
`afspr_lookup_object` on real images: a secured file, directory, symlink and
root, a plain file beside them, resealed 104-byte payload and nonzero tail,
and a forged reference on a volume without the feature. One difference
remains by design of the two lookups: the C volume path bounds the first
segment block at lookup, the Rust lookup carries the reference unread and
bounds it when the chain is walked.

## 2026-09-17 — Grow the AROS C boundary to interface revision 7

The Stage C inventory is [implementation/stage-c-gap.md](implementation/stage-c-gap.md):
per item, what the external handler carries at each of four layers (adapter,
C boundary, packet translation, target runtime) and what it lacks. The
development host that built this step has stable Rust, Clang and the AROS
source tree and no built AROS SDK, QEMU, FS-UAE or target Rust toolchain, so
every result below is a host result and the target gates were not run.
[`tools/dev-packet-matrix.sh`](tools/dev-packet-matrix.sh) keeps the packet
layer testable there by compiling the host matrices against the AROS source
headers with a synthesised `aros/config.h`; it makes no qualification claim.

ABI version 1 had no way to ask a static library what it carries. The
boundary grows by entry-point groups: `afsplus_aros_interface` answers
without a mount, the packet layer asks it at creation and turns an action of
a missing group into `ERROR_ACTION_NOT_KNOWN`, and query structures negotiate
their size. Revision 7 carries capability query, DOS metadata, soft links,
the API v2 group (positioned 64-bit I/O, clone, preallocate, atomic replace,
advise), watches, health and trace sink, the versioned info document and
counters. The Rust capability mask and the published `FSV2_CAP_*` identities
are separate numberings joined by one table.

Two defects surfaced. The VFS directory cookie is bound to one checkpoint
generation, so `ExNext` answered `ERROR_INVALID_LOCK` after any commit and
`Delete #?` could not work; `resume_directory_after` finds the position after
the last returned name by a galloping binary search over single-entry pages.
Five one-line C entry points bypassed the status wrapper, so their failures
never reached the health log; a literal call count in the counters test
exposed it.

File handles hold their object like DOS locks (`MODE_NEWFILE` exclusive), a
held object is not deletable and `DupLockFromFH` on an exclusive handle is
refused, the behavior `NameFromFH` in dos.library is written around. Two host
matrices that deleted a name while holding its lock assert
`ERROR_OBJECT_IN_USE` first. The Hosted S0 and S1 probes have not run against
this rule.

The classic security adapter is a seam: one probe question, refusal with
`ERROR_WRITE_PROTECTED`, an explicit downgrade mount flag. Its on-disk answer
waits on the security preservation container. File comments wait on a stored
attribute record, clone metadata inheritance on Q14.

Generic AROS findings: `fdsk.device` answers `CMD_UPDATE` inside `BeginIO`,
so the barrier reply overtakes queued writes and never reaches the backing
file, and the hosted `emul-handler` implements no `ACTION_FLUSH`.
[`native/aros/upstream`](native/aros/upstream/README.md) holds the focused
`fdsk` patch, which applies to the AROS tree, and its probe; neither has been
compiled.

## 2026-09-17 — Decide clone metadata inheritance and leave the clone source untouched

`CloneFile` gives the destination the source's modification time and
protection word, a new identity, creation and change time at the clone time,
one link, full COW and no security descriptor. `CloneRange` moves only the
destination's modification and change time. Both leave every user-visible
field of the source, including its change time: before this entry both set
the source's change time to the clone time when they rewrote its record to
mark shared runs, so a reader's clone made backup and scanning tools revisit
an unchanged file.

`crates/afsplus-check/tests/clone_metadata.rs` pins the contract with literal
times; its two source assertions failed with the clone times 40 and 50 before
the correction. The 17 shared-clone tests and the two data-policy clone tests
pass with it. The decision, its build-cache consumer and the API consequences
are in
[clone metadata inheritance](adr/ADR-102-clone-metadata-inheritance.md).

## 2026-09-17 — Carry opaque security descriptors through every object rewrite

The object record gains an optional 16-byte security reference behind object
flag bit 2: first segment block, descriptor length, segment count and a flags
word whose bit 0 marks a diverged projection. The descriptor itself is an
opaque byte string of up to 65,536 bytes with a format identity and a format
version, stored in a chain of `"AFSX"` segments that the object owns
exclusively. The volume identity is INCOMPAT bit 3,
`org.aros.afsplus:security-descriptors`, set by
`mkfs_with_security_descriptors`; mount refuses it together with persistent
snapshots.

The reference is a field of `ObjectRecord`, so every read-modify-write path
carries it; the one path that rebuilt the flags word, layout staging, keeps
the bit. Objects die in two places, `remove_entry` and the window engine's
`unlink_in_batch`, and both retire the chain. A protection edit on a
descriptor-bearing object is refused under the default strict policy and,
under the preserve policy, lands together with the divergence mark in one
transaction. The checker claims every segment in the ownership set.

Proof: 8 wire tests in `crates/afsplus-format/tests/security_container.rs`, 8
end-to-end tests in `crates/afsplus-check/tests/security_container.rs`
(size classes across remount, eleven rewrite paths, strict and preserve
projection, five removal paths, orphan cleanup and intent-log replay, three
resealed segment corruptions, feature absence and the refused snapshot
combination) and a power-cut matrix of 2,334 modeled states over attach (626),
replace (1,165), preserve-mode edit (197) and delete (346), each mounting to
the state before or after the transition with a clean checker verdict. With
the retirement call removed from `remove_entry`, the removal test fails on
the checker's leak findings. The fuzz oracle models the reference
independently. Clippy is clean on the format, core and VFS crates.

The decision is proposed in
[security preservation container](adr/ADR-101-security-preservation-container.md).
Portable C parity, conformance images, the format-identity registry, backup
transport, descriptors under persistent snapshots and every evaluation
semantic are open there. The filesystem API consequences (three descriptor
operations, a fidelity query and a new refusal of the protection setter)
belong to the API work.
## 2026-09-17 — Reserve an actor field in the epoch-1 change record

A change record names the object, the sequence and the event, and never the
cause. The enclosing operating-system project lists the actor question among
the format decisions that precede the epoch-1 freeze, and its integrity and
provenance service is the consumer with a stated need: it takes the change
stream as its incremental scan queue and cannot turn an event into a finding
without a cause. Its packaging feature states the opposite need, because an
atomically published staging tree attributes itself.

A grep of `crates/` for change-record and change-stream identifiers returns no
encoder, no decoder, no record type and no test, and M10 is not started, so
reserving the field changes no code and no image while adding it after the
freeze would change the epoch.

The proposal
[reserve an actor field in the change record](adr/ADR-103-change-record-actor.md)
places a fixed 16-byte actor in the common record header, as a 16-bit host
actor class, an 8-bit host trust claim, a zero reserved byte and twelve opaque
bytes that the class defines, with an all-zero field meaning unattributed. The
value is advisory evidence supplied by the host adapter, authenticated by
nothing in the format, and read by no filesystem decision. The layout carries
no AROS concept: the AROS adapter maps an Exec-owned sender table into one
class and a POSIX adapter maps a pid, a uid and a process start time into
another. An actor is a runtime subject observed by the host; a principal is the
realm, kind and UUID security identity of docs/30, and epoch 1 reserves no
principal field, because a later host adds one through a negotiated record type
or actor class.

Admission follows the object-record rule for every byte no writer may choose:
the reserved byte is zero and a zero class forces the remaining thirteen bytes
to zero. An unassigned trust value reads as advisory with its raw value, and an
unassigned class is admitted with the actor reported as unreadable, which the append-only stream affords because a
committed change record is never re-encoded. The M10 change-stream lot owns the
codec and its admission tests, the class registry and the wire offsets, and the
Stage C API session owns the iterator field.

## 2026-09-17 — Admit object records only in their canonical image

The Q13 experiment ran against the object readers: after a CRC reseal, the
generic, generation-returning and metadata readers admitted all 16 common
header flag bits, payload lengths of 97, 104, 112 and the full block, and a
nonzero byte at four tail positions, on files and directories. The symlink
reader refused all three; the portable C reader refused header flags and
admitted the other two. Every mutation path re-encodes decoded fields into a
zeroed block, so an admitted byte without a field is dropped by the next
rewrite of that object.

The object decoder now checks header flags, exact payload length and the zero
tail in the one function all four readers share.
`crates/afsplus-format/tests/object_admission.rs` holds 56 resealed negative
images, each refused by three readers with an exact reason, a canonical
control and a symlink image; five tests pass, alongside the existing
reserved-byte, roundtrip and C symlink tests. The rule and its extension path
are proposed in
[exact admission of object records](adr/ADR-100-exact-object-record-admission.md).
Portable C parity for payload length and tail, and the resealed conformance
images, are open under that proposal.

## 2026-09-17 — Close a-cache and Stage A

Four residual lots finished the tiny-cache inventory: the data families gained
sampled cut campaigns at four and eight pages with measured spills; the
structure, clone and shared families gained their remaining evictions,
refusals, retained views and reload failures; the deferred-window and
snapshot families gained evictions of their recovery commits, the two update
refusals, publication read failures, logging-phase faults, a planted registry
exhaustion and a wrapping ledger scan; the tail lot added the mid-run reclaim
cursor, a rotation-batch cut campaign in the core crate and the ambiguous
runner over every remaining family. Every inventory row names either no open
combination or a measured staged-node demand with its number.

The final qualification in `build/stage-a-qualification-9918fe3` (1505/0/13), run once on the complete sources with
`--no-fail-fast`, passed with every other gate; the nine generated families,
eighteen controls and the version-1 baseline were rerun in
`build/fuzz-campaigns-9918fe3`. With `a-cache` complete the eight finite gates
of Stage A have their evidence. Native adapters, sustained workloads,
platform qualification and the epoch-1 freeze keep their later owners.

## 2026-09-17 — Fix window refusals that poisoned an open deferred window

The deferred-window residual families exposed a second defect. Any error
returned by `window_write_file_at` or `window_truncate_file` poisoned the whole
open window, including a `NoSpace` refusal that had reached neither a device
write nor the staged bookkeeping; a window holding one acknowledged group and
22 staged operations lost all of them, while `window_op` preserved the window
for the same error class. That contradicts the ownership rule of
[the intent-log qualification](testing/intent-log-write-truncate-qualification.md)
for preflight refusals.

Both calls build their replacement blocks and allocate their extents before
the first device write, so that write is the mutation boundary: each records
whether it crossed it and restores the window when it did not, and failures
after it keep the remount requirement. Eighteen lines in `volume.rs`, no
on-disk or API change; the qualification document states the boundary. The
`update_refusals` family reproduces the loss before the fix at every profile.

## 2026-09-16 — Fix intent-log replay reusing a logged data run

The deferred-window cache families exposed a crash-safety defect in
recovery. Replay rebuilt its pending batch with an empty `logged_created`
set, so a replayed prefix that created a file and later deleted it took the
"never logged" branch, released the create's data run inside the replay
transaction, and allocated replay metadata over blocks a durable record's
content CRC still covered. A power cut during that recovery left the record
invalid, and the next mount dropped every acknowledged group. One durable
create with content followed by one durable delete reproduces it: the first
recovery write lands on the create's data block.

ADR-037 makes a record's referenced data claimable only while its fsync has
not completed, ADR-063 requires replay to stay idempotent when recovery itself
crashes, and the live window already kept logged runs out of reuse through
`unlink_in_batch`. The fix marks every replayed create as logged, so a
cancelling delete in the same prefix quarantines the run until the replay
checkpoint publishes; one line in `apply_replay_create`, no on-disk or API
change. The same commit refuses `snapshot_delete` after an ambiguous
publication before it reads the registry, the rule the other entry points
follow. The deferred, replay and snapshot family matrices (148 tests) and the
intent-log, orphan and shared crash suites pass on the fix; the family that
found it keeps the reproducer.

## 2026-09-16 — Close a-fuzz and a-flight

Version 8 of the semantic replay profile exports every diagnostic kind the
recorder can emit, with a fixed payload area and an admission rule per
payload class. Producing the failure kinds required three scenario
commands: single-shot device faults counted from the operation they precede,
a format fault whose partially written device is discarded before a fresh
format, and one-byte image edits, plain or resealed through the block
checksum so an invariant violation reaches the verifier. Sixty-three kinds
come from host scenarios; `ApiUnwound` needs a panic through the API guard
and stays with the core unwind test.

The generator gained staged final unlinks inside deferred windows (the
reserved directory receives every final unlink of a committed object, and a
create cancelled by the same window leaves nothing), an exact model of
CloneRange destination coverage and source change time, bounded write and
truncation surfaces, and an audit of all 66 API methods against the
generators. No filesystem bug appeared in either lot.

The integrated qualification in `build/stage-a-qualification-9918fe3` (1505/0/13) with the nine generated families, eighteen controls and the version-1 baseline rerun in `build/fuzz-campaigns-9918fe3` passed with every other gate. With
these lots `a-flight` and `a-fuzz` are complete; `a-cache` keeps Stage A open.

## 2026-09-16 — Integrate lifecycle observation, six generated families and the structure matrix

Three lots landed together. The recorder observes mount selection and intent
recovery, formatting and standalone verification through opt-in entry points,
plus staged data writes, intent barriers and read-only view descents; every
direct publication caller and all 66 API methods run under observation with
observed/unobserved equality of results, traces and images, and the emission
mechanism is measured (zero heap bytes per event, 26.6 ns per ring emission,
232-byte events on this host). Six more operation families are generated under
runner version 9 with independent models and negative controls. The batch,
directory-structure, clone and shared-ownership cache families run through the
driver, including read failures while reloading spilled nodes.

Two documentary findings: no normative text decides CloneFile metadata
inheritance ([Q14](implementation/open-questions.md)), and a recorder call
path spends most of its cost before category admission because scope flags
live inside the locked recorder; publishing the masks as atomics is a cheap
later change. The integrated qualification in
`build/stage-a-qualification-58fee08` passed 982/0/13 with every other gate,
and all nine generated families, fourteen controls and the version-1 baseline
were rerun on the integrated sources in `build/fuzz-campaigns-58fee08`.

## 2026-09-15 — Integrate the family-matrix driver and four more cache families

A reusable driver replaces per-family copies of the cut, fault, ambiguous
publication, retained-snapshot, forced-eviction and resource-refusal runners.
Transactions whose unflushed tail exceeds the exhaustive budget get a sampled
campaign: every in-order prefix, representative tears, exhaustive subsets of
short flush segments and 256 seeded subsets of longer ones, with the same
exact-state oracle and both outcomes required. The reclaim step under
eviction has a 137-write tail; the campaign covers 811 images per profile.

Two limits were measured and recorded as such: the in-place private write
stages two nodes, so no bounded profile evicts it, and orphan insertion
succeeds after ordinary allocation is exhausted, so that variant qualifies
cuts and faults under exhaustion. The integrated qualification in
`build/family-matrix-qualification-16b5c7a` passed 747 tests with no failures
and ten explicit ignores, plus every other gate, with unchanged sources.

## 2026-09-15 — Integrate data-write cache families and generated operation families

Two lots prepared on isolated branches were reviewed and integrated together.
The data-write matrix closes the write, reservation, preallocation and
truncate rows of the tiny-cache inventory at four profiles; its wide fixtures
force extent-map eviction with measured spills, and its cut oracles were
reduced to an ownership proof plus exact records after full-byte reads made two
four-page tests run 22 and 44 minutes. The generator adds window, snapshot and
linked-namespace families with independent models computed before execution,
runner version 9 for links, symlinks, clones and protection, fresh-process
replay and seven negative controls.

The integrated qualification in `build/data-fuzz-qualification-238c25e`
passed 651 tests with no failures and ten explicit ignores, plus formatting,
Clippy, codec, nine Python replay/measurement suites, documentation and
whitespace gates; the three campaigns, their replays, the seven controls and
the version-1 baseline were rerun on the integrated sources in
`build/fuzz-campaigns-238c25e`. `test-allocation-origins.py` takes a
measurement input and stays outside the automatic gate. Operation families
without a generator are listed in [the milestones](implementation/milestones.md#fuzzing-and-property-test-tasks);
`a-fuzz` and `a-cache` stay partial.

## 2026-09-15 — Qualify reclaim, policy, low-space, shared-fault, orphan and rotation cache profiles

Four fixture patches prepared in isolated copies were reviewed before
integration. Review replaced accounting expectations captured from the writer's
own run with literal free/pending counts, compared complete object records for
policy-flag cuts, required literal clone bytes, rejected every checker warning
except the stopped intent-log tail, and added staged-node bounds.

The orphan-profile draft failed its fragmented cleanup test. ADR-066 cleanup
removes whole extent records from the logical end and publishes a size ending at
the lowest removed extent, so the draft's expected five-block and one-block files
were wrong; the filesystem published six and two blocks. The test pins those
literal prefixes and checks the image after each step. The allocation-rotation
draft did not compile; the qualified test adds a persistent snapshot and a
two-block shared run, and uses longer names after an eight-page run showed zero
spills.

Every new family has a deliberately wrong expectation that fails. The integrated
qualification in `build/cache-ready-qualification-ee21aa8` passed 627 tests with
no failures and ten explicit ignores, plus formatting, Clippy, codec,
documentation and whitespace gates, with unchanged sources. No production code
changed. Four fixture groups observed no spills, so their eviction combinations
stay open in [the inventory](crates/afsplus-check/tests/tiny_cache_matrix.md).

## 2026-09-15 — Complete typed caller and Unicode admission fixtures

Added deterministic caller tests for middle-record intent termination, resealed
reclaim cross-block relations, typed object/allocation/extent payload admission
and spelling-preserving directory reads. The direct fixtures check exact accepted
prefixes, decoded values, read bounds and no-write refusals; the reclaim cases
also require the expected diagnostic reason. They complement existing independent
reference-count, tree traversal, snapshot ownership and recovery models.

Reviewing the target matrix exposed a remaining Unicode corpus obligation beyond
the literal naming vectors. Vendored the official Unicode 16.0.0 normalization and
case-folding data with URL/digest provenance and its license. The public name API
passes 99,825 normalization checks, including ten required reserved-name refusals,
and NFC/default full case-folding checks over 1,112,062 admitted scalar names.
An initial normalization test incorrectly expected reserved slash-containing
names to succeed; correcting that test preserved the filesystem admission rule.
No production-code or on-disk-format change was needed.

The twelve new tests passed in the isolated source copy, whose core/format/block
sources matched the main repository. Integration uses the same test and corpus
bytes. The integrated qualification in `build/caller-property-qualification-m5dx8kq8`
passed 618 tests with no failures and ten explicit ignores, plus formatting, Clippy,
codec, documentation and whitespace gates; its source manifest matches every
committed code, test and corpus byte. A read-only closure review of the target
matrix found no further finite caller-admission gap.
Native/C interoperability, proposed directory overrides and format freeze remain
separate obligations. The gate verdict and remaining work live in
[the milestones](implementation/milestones.md#fuzzing-and-property-test-tasks).

## 2026-09-15 — Integrate subsystem observations and caller-property tests

We connected allocation, mutable-tree I/O and reclaim transitions to the
volume recorder through weak transaction observers. Preparation events remain
ordered with API calls rather than being buffered until checkpoint publication.
Recorder replacement also updates an open deferred transaction.

The initial `Rc` owner broke the all-features FUSE build. An intermediate mutex
restored transferability but changed immutable access into exclusive mutable
access. The revised read/write-lock owner preserves simultaneous read-only
access, refuses conflicts without waiting, and recovers diagnostic access after
provider unwinding. The focused suite passed 23 tests, including a captured-read
panic followed by retry with an unchanged image. A separate borrow test checks
conflicts and poisoned-lock recovery. Full integration qualification passed
606 Rust tests with no failures and ten explicit ignores across 97 test groups.
Formatting, all-features Clippy, the 21-target codec gate (4,096 cases each),
documentation checks, checker tests and whitespace validation passed. Sources
and logs are retained in `build/recorder-rw-qualification-gj8dyh3r`. Only
documentation changed during qualification; production and test source hashes
remained identical. The ignored gates do not establish native or live-mount
qualification.

The shared-ownership crash fixtures gained explicit 2/4/8/unlimited profiles,
independent survivor bytes and repeat-recovery checks. Nine expanded fixtures
covered 45,808 modeled images in the agent run; the complete target's 88,412
also includes earlier clone cases. Independent per-block reference-count and
typed-tree traversal properties accompany these tests. The cache inventory now
distinguishes this coverage from remaining fault, eviction and retention work.

The measured extended event layout is 192 bytes on this macOS AArch64 host,
versus 104 previously. The ring's memory cost and shared-owner allocation must
be included in qualification. Existing replay versions do not opt into the new
subsystem events; extended export, remaining diagnostic paths and resource
measurements retain their Stage A gates. No stage completion follows from
these focused results alone; current state remains in
[the milestones](implementation/milestones.md).

## 2026-09-15 — Identify allocator recorder ownership prerequisite

Inspected Volume's scoped API guard and recorder accessors, TxAllocator
construction/allocation methods, and the scenario drain consumer. Recorder
ownership crosses the allocator's lifetime and commit boundary; buffering events
until commit would lose chronological and failure evidence. Recorded the
integration experiment and acceptance requirements under the existing a-flight
owner, comparing explicit observer threading with a bounded shared handle.
No allocator event integration or architecture selection is claimed by this
source audit. It identifies the next implementation prerequisite rather than
creating a separate feature goal.

## 2026-09-15 — Qualify legacy one-block codec mutation targets

Added stable targets 19–21 for directory, object-map and retired-list readers.
Independent payload oracles check decoded fields, bounds, ordering, reserved
bytes, names and ID/generation admission. Explicit truncation, resealed-field
and length controls supplement mutation; short encoders refuse and minimum
valid buffers roundtrip. Common-header verification is shared and legacy
key admission is kept distinct from current typed-tree Unicode rules.

The gate passed 21 × 4096 cases (86,016), sixteen fuzz tests and seventeen
saved-input replay controls. The first eighteen seed/case fingerprints are
unchanged. Fuzz formatting/clippy and documentation checks pass; retained
proof: `build/legacy-codec-yxmcj7yk`. The preceding 45 legacy corruptions and 44 format roundtrip
tests qualify the reader correction separately. No full-workspace or native
run is claimed; core cross-record relations and generated operations remain
Stage A requirements.

## 2026-09-15 — Qualify captured state through clone publication

The retained-snapshot clone campaign passed 66,652 modeled states across
2/4/8/unlimited page caches (16,659 old and four new states per profile), in
408.97 seconds. Each state checks exact live source/clone contents and
old/new generation, captured source metadata and bytes, absence of the clone
from historical metadata, and complete historical root enumeration with EOF.
The checker rejects errors and older-checkpoint warnings.

The fixture explicitly opts into a sixteen-write exhaustive budget after the
initial twelve-write limit correctly refused its larger tail. No sampling was
introduced. Block/check all-target clippy and documentation checks pass.
Retained evidence is in `build/snapshot-clone-profiles-daqrolub`. This campaign
used the source captured there before the independent legacy-codec correction;
no full combined workspace or native claim follows. Forced eviction and
post-clone mutation of retained/shared data require their separate matrices.

## 2026-09-15 — Enforce legacy codec reserved-zero fields

The executable legacy directory, object-map and retired-list readers accepted
nonzero bytes documented as reserved-zero in their wire layouts. A CRC-resealed
regression failed before the fix on the directory header's first reserved byte.
All three readers now reject their reserved payload fields; directory entries
also reject nonzero reserved bytes after the type hint. Bounds checks precede
all new reads. Generic header/tail policy is not changed by this correction.

The 45-corruption regression passes, along with 44 roundtrip/hostile-input tests
and format all-target clippy, formatting and document checks. This is focused
legacy-codec evidence, not new native or full-workspace qualification. Direct
fuzz targets for these executable readers remain in the Stage A inventory.

## 2026-09-15 — Add explicit exhaustive power-cut budgets

A retained-snapshot clone fixture reached thirteen unflushed writes and correctly
tripped the simulator's default twelve-write guard. Added an explicit streaming
budget API, capped at twenty writes, preserving all full-write subsets and the
existing representative tears. The default and overlay limits are unchanged;
no workload silently falls back to sampling.

Two focused tests pass: 8,192 distinct full subsets plus 39 tears at thirteen
writes, and default refusal before any callback. All eighteen other block tests
pass separately, including memory/overlay parity. Block/check all-target clippy,
formatting and documentation checks pass. The filesystem consumer campaign is
separate and still running; these results qualify the simulator only. Retained
results and source are in `build/snapshot-clone-profiles-daqrolub`.

## 2026-09-15 — Qualify ambiguous clone publication across cache profiles

Extended the completed-checkpoint-write/adoption-read regression to create and
CloneFile at 2/4/8/unlimited pages. Sixteen combinations require mutation
poisoning, the complete published state after remount, unchanged source bytes,
exact clone sharing, and safe subsequent mutation. The checker validates before
remount and after the follow-up mutation.

The focused test passed in 0.29 seconds; targeted clippy, formatting and document
checks pass. This deliberately models a completed write returning an error and
post-publication read failure on a memory device. It does not qualify native
flush behavior, eviction or retained snapshots; those requirements remain in
the matrix. No new full-workspace run is claimed.

## 2026-09-15 — Qualify first-clone I/O failure recovery

Injected each of 13 recorded writes and two flushes under each cache profile:
60 failures in total. Earlier errors preserve the old live namespace and source;
final-barrier uncertainty blocks further mutation. Remount selects exact old or
complete new state, and old-state retry produces the expected clone and sharing
records. The exhaustive checker validates before and after retry.

The focused test passed in 1.23 seconds, with targeted clippy, formatting and
document checks passing. The fixture qualifies before-write and flush errors,
not completed-but-reported-failed writes, adoption reads, forced eviction or
retained snapshots. No full-workspace or native campaign is claimed.

## 2026-09-15 — Qualify first-sharing CloneFile cache profiles

Applied 2/4/8/unlimited profiles to first-clone setup, recording and cut-image
recovery. The oracle checks unchanged source contents, clone visibility and
contents, exact two-block/two-reference sharing, and old/new generations.
All four tests passed in 29.06 seconds; targeted clippy, formatting and document
checks pass. The common matrix is unchanged from the preceding qualified unit;
this run makes no new full-workspace claim. Eviction, retained views and explicit
error-return retries remain separate cache-matrix requirements.

## 2026-09-15 — Qualify CloneRange reference boundaries across cache profiles

Extended the shared-range crash matrix to explicit 2/4/8/unlimited page
profiles during fixture construction, transaction recording and recovery.
Every modeled cut checks old/new generation and destination bytes, unchanged
source and peer bytes, and exact reference-count transitions. The checker
validates every cut image; both old and new outcomes must occur.

All four range-profile tests passed (13.95 seconds). The ten other shared-crash
tests passed separately (66.71 seconds), qualifying the refactored common
matrix without rerunning those four tests. Targeted clippy, formatting and
document checks pass. Retained source and result summaries are in
`build/clone-range-profiles-gn6rezpx`. This is host test qualification, not a
new full-workspace or native campaign. Eviction, unaligned boundary copying,
retained snapshots and explicit I/O-error retry remain separate combinations;
the Stage A cache gate stays open.

## 2026-09-15 — Qualify symlink and object metadata codecs

Integrated stable targets 17 and 18 with explicit directory, regular-file,
empty-file, extent-tree, policy and symlink fixtures. Independent payload
oracles compare fields and admission across generic, metadata and borrowed
symlink readers, including UTF-8, lengths, short buffers and borrowed pointers.
Common-header verification is shared; Q13 retains the generic extension-policy
question separately from symlink rules.

The main gate passed 18 × 4096 cases (73,728), 15 fuzz unit tests and 14 saved
replay controls. Earlier target fingerprints are preserved. Proof is retained
in `build/object-codec-integration-ojy8u7x0/integration-fuzz.log`, alongside the
sealed isolated source campaign (575 workspace passes, zero failures, ten
ignored). This composes isolated workspace and main integration evidence;
no new full combined workspace or native run is claimed. Other executable
codec surfaces and semantic-operation families keep Stage A fuzz closure open.

## 2026-09-15 — Reject the reserved object payload byte

Generic and metadata object readers reject a nonzero byte at payload offset 9,
as required by the existing wire layout comment. Six CRC-resealed file and
directory corruptions exercise all three reader entry points and exact byte
restoration. The focused main regression and formatting pass; the retained
isolated source campaign passed 575 workspace tests, zero failures and ten
ignored tests, with clippy and formatting passing. Evidence lives in
`build/object-codec-integration-ojy8u7x0`; this is composed evidence, not a new
full main-worktree campaign.

The separate generic header-flags and payload/tail admission ambiguity is
recorded as Q13, owned by the Stage A codec inventory and the format-freeze
review. Tightening those fields requires an explicit decision and cross-reader
experiments; the documented reserved-byte correction does not settle them.

## 2026-09-15 — Qualify snapshot-bearing checkpoint admission

Integrated target 16 with an explicit 112-byte checkpoint payload containing
snapshot registry and lifetime roots. Its independent payload oracle checks
fields, UUID/generation binding and structural bounds for the fixture geometry;
common-header verification is shared. Volume feature negotiation and referenced
root ownership require separate caller tests and are not claimed by this gate.

The main integration gate passed 16 × 4096 cases (65,536), 13 fuzz tests and
12 saved replay controls, preserving earlier target identities. Retained proof:
`build/snapshot-checkpoint-integration-wjhshbg2`. The isolated qualification
reuses 574 passed / 0 failed / 10 ignored workspace evidence after comparing
581 unchanged inputs; no new full workspace or native run is claimed.

## 2026-09-15 — Qualify reclaim codec mutation and replay

Integrated direct root, segment and table targets with independent payload
admission and field checks. Existing target identities and fingerprints are
preserved; shared common-header verification is an explicit oracle limitation.
The main-worktree gate passed 15 × 4096 mutations (61,440 cases), 11 fuzz
unit tests and 11 saved replay controls. Evidence is retained in
`build/reclaim-codec-integration-3ugcpze0/integration-fuzz.log` alongside the
reviewed isolated source and full-workspace evidence.

This fuzz-only integration reuses the isolated full-workspace result
(574 passed, 0 failed, 10 ignored) and the preceding correction's affected
main tests; it does not claim a new full combined workspace run. Stage A's
codec gate stays partial: other wire surfaces, caller invariants and semantic
operation coverage require their own evidence. No native qualification is implied.

## 2026-09-15 — Enforce reclaim reserved-byte admission

The reclaim readers reject nonzero documented reserved fields after validating
minimum lengths: root payload bytes 4–7 and 50–51, and segment/table bytes 4–7.
The regression reseals checksums for 42 corruptions and restores the exact valid
encoding. Physical address and cross-block ownership checks stay with callers.

The reviewed correction originated as `384b4d3`. Its isolated combined
qualification passed 574 workspace tests, failed none and ignored ten; the
sealed proof is copied unchanged to
`build/reclaim-codec-integration-3ugcpze0/agent-proof`. Main integration additionally
passed the format regression and 12 checker/captured-snapshot tests, with one
explicit scale qualification ignored. This composes the isolated format evidence
with the independently qualified captured-replay implementation; it does not
claim another full combined workspace run. Reclaim fuzz targets follow separately.

## 2026-09-15 — Preserve and replay captured snapshot state

The [version-7 replay profile](testing/developer-harness.md#captured-snapshot-replay-bundles)
adds persistent snapshot lifecycle operations and explicit limits on formatting,
mount, remount and recovered inspection. Its final-state oracle compares the
complete supplied snapshot registry, metadata, file contents, symlink targets
and allocation ranges independently of live labels. Open readers prevent deletion;
remount closes handles and persistent IDs can be opened again.

Qualification retained 48 version-7 bundles, a minimized negative and a separate
2 MiB sparse-snapshot failure. Wrong expected contents, metadata, allocation and
registry membership all fail and replay as failures. Six artifacts match across
142 historical cases, with originals unchanged. Four cache profiles cover
create/delete publication boundaries and snapshots surviving live unlink.
Direct inspector tests also cover aliases, symlinks and aggregate output budgets.

The source-stable Rust campaign passed 576 tests, failed none and ignored ten
in 91 groups; fmt, Clippy, codec fuzz, documentation and checker fixtures passed.
A separate reviewed Python correction stopped applying the 1 MiB scenario-input
limit to observed contents: observations retain the 16 MiB aggregate budget.
All 34 Python tests passed on the corrected files, whose exact hashes were checked
on integration; Rust sources were unchanged. The corrected source capsule restores
the exact revision/worktree identity and reproduces the retained large sparse
failure using the preserved runner. This is not an independent compiler rebuild.

Evidence is retained in `build/captured-replay-k_2r6p2h`, including original
qualification sources, corrected-source capsule, logs and composed evidence.
The semantic oracle covers the final recovered registry; an intermediate view
deleted before inspection has no separate expected-content claim. Other subsystem
diagnostics and native qualification remain open; this does not close Stage A.

## 2026-09-15 — Qualify snapshot leaf and key codecs

Integrated the reviewed agent commit `9ae581f`: five deterministic snapshot
codec targets append IDs 8–12 without changing the seven historical seed
fingerprints. Independent byte-field oracles check admission and decoded
values; explicit dispatch prevents later targets from being misclassified.
The checks include exact lengths, reserved bytes, numeric boundaries, ID
exhaustion and half-open lifetime membership.

The agent's source-bound full workspace passed 572 tests, zero failures and ten
ignored tests on base `97c564f`. Its final fuzz campaign passed 12 x 4096 cases
and eight unit tests, with saved artifact replays, formatting and Clippy.
I verified its 602-file evidence manifest and the reviewed corrections.
Integration into the export-qualified main tree reran all 49,152 fuzz cases,
eight unit tests and replay controls successfully. The format crate and fuzz
manifests had no intervening changes. These are separate qualification scopes,
not a claimed new combined full-workspace run.

Evidence is retained in `build/snapshot-codec-integration-ijm6odz7`, including
the unchanged sealed agent proof. The fixed generation/geometry context is
explicit in the test plan. Enclosing tree ownership, cross-record accounting,
other codecs and semantic operation generators remain open under Stage A.

## 2026-09-15 — Export and replay object-map diagnostics

Added explicit semantic scenario version 6 and `AFSFLT05` object payloads.
The independent reader checks event presence, kind, zero fields and truncation
without treating a mapped address as an integrity verdict. Earlier profiles
keep their event vocabulary and byte representation. Board review 9 approved
the encoding and admission code after independent inspection.

The source-stable Rust gate passed 573 tests with zero failures and ten ignored
across 90 groups. Clippy, formatting, seven codec targets with 4096 cases each,
scenario admission and documentation checks passed. The retained campaign has
99 historical six-artifact comparisons, 24 new profile/filter/ring bundles,
twelve deferred-window cases, a failure-preserving reduction and five selected
create cuts. Original historical artifacts were unchanged. The source capsule
restored all 581 tracked files with identical hashes.

After the Rust gate, two permanent Python wrappers were added for the already
retained v6 deferred and minimization cases. The final Python suite passed all
28 tests; production and Rust sources were unchanged. The source capsule
records the initial tested tree; the final patch includes these test wrappers
and result documentation. Evidence is `build/object-export-i1lx25ko`.

This is live-view replay qualification. Captured-view scenario commands,
independent expected historical state and mount-specific lease handling are
explicit prerequisites for snapshot replay. Allocator/tree/cache/reclaim and
mount-time observation remain separate open diagnostic tasks. No native
hardware or full Stage A completion is claimed.

## 2026-09-15 — Integrate the first cache-family qualification matrix

Integrated codex-cache's reviewed commit `cdd96b4` into the object-observation
core at `be97dbe`. The ten-test target passed with zero failures and zero
ignored tests in the isolated combined checkout. All 162 Rust source hashes
match the integrated sources. The campaign includes 72,220 namespace cuts,
576 write/barrier faults, 1,384 recovery cuts and twelve early spill faults.
Its captured-directory oracle enumerates every page and verifies final EOF.

The retained evidence is `build/cache-matrix-review-0vur3dno`, including the
agent's raw results and the combined-checkout Clippy and target logs. The
main core's separate full suite passed 562/0/10; the original agent's full
suite predates the final pagination strengthening. Neither is described as a
full-suite run of this final combined checkout. Qualification composes that
core gate with the ten integration tests on hash-identical sources.

The source inventory names 26 families and their omitted combinations.
Stage A cache qualification stays open for clone/shared transitions, orphan
lifecycles, snapshot maintenance, reclaim and the other explicit omissions.
No native memory-pressure or hardware durability claim follows from this lot.

## 2026-09-15 — Review object resolution and preserve returned IDs

The next diagnostic unit adds separately enabled object-map lookup, mapping
and missing-object events. Their payload contains object ID, metadata-block
address and live/snapshot view ID; the event generation identifies the viewed
checkpoint. Captured reads borrow an explicit observer, and live root-cache
hits identify the cached record block without fetching it again. The first
four-profile test found equal image/I/O results and distinct live/captured
metadata addresses. The expanded test covers all six captured-read API entry
points. Eighteen flight tests and two recorder unit tests passed, including
read failure/retry, live delivery loss and the ID-preserving family oracle.
The arm64 layout probe measured Event at 104 bytes versus 72, while recorder
and optional-recorder layout stayed at 176 bytes. This increases ring storage
by 32 bytes per requested event; an uninstalled recorder allocates no ring.
All 99 historical cases matched six artifacts exactly and replayed with the
new runner. Full workspace qualification completed with 562 passed, zero failed
and ten ignored tests across 89 groups. Formatting, all-target/all-feature
Clippy and seven codec targets with 4096 cases each passed. Evidence is retained
in `build/object-observation-icc3feor`; these are hosted results, not native
hardware qualification.

Review of `be9b621` found that its directory-create, symlink-create and clone
branches mapped successful returned IDs to unit values before comparison.
Thus its evidence supports exact image/I/O and success/error comparisons,
but the description overstated return-value equality for these three branches.
The family helper was corrected to preserve IDs, for both legacy and expanded
observation. The baseline row was reopened for qualification and closed after the corrected
oracle passed the full gate; the original sealed evidence is preserved unchanged.

## 2026-09-15 — Extend publication-family diagnostic comparisons

The Stage A source audit identified fourteen callers of the shared publication
tail and separated routing evidence from executed observation tests. It also
identified writes before that tail (window payloads and tree-cache spills),
mount-time recovery before recorder attachment, and standalone formatter and
verifier entry points. These findings are tracked under `a-flight` in
[the diagnostic inventory](implementation/milestones.md#core-diagnostic-path-inventory).

A new [flight integration test](crates/afsplus-core/tests/flight.rs) exercises
eight publication families at all four cache profiles, two ring capacities,
and successful execution or failures at flush index zero or one. All 192
observed/unobserved pairs passed the targeted run, comparing results, I/O and
every image block. The first compile exposed a test-only accessor error: the
fault backend has a consuming `into_inner`, not a borrowed `inner`; the test
was corrected to consume the wrappers after comparing their traces.

The [test procedure](testing/developer-harness.md#publication-family-observation-equivalence)
limits this evidence to observation equivalence and the selected failure
indices. It does not claim power-cut coverage, every publication family, or
new object/block/view events. The full quality gate passed: 557 workspace tests, zero failures and ten
explicit ignores across 89 groups; formatting, Clippy and seven codec targets
with 4096 cases each also passed. The retained evidence directory is
`build/publication-families-92wlsaqm`; source fingerprints matched through
validation. This is hosted evidence, without a new hardware qualification.

While the Rust sources stayed fixed, the codec audit identified distinct
snapshot/reclaim/symlink/legacy decoder surfaces and tree-payload validators
that the seven-target dispatcher does not directly cover. Their requirements
are recorded in the [codec inventory](implementation/milestones.md#codec-surface-inventory),
without closing the fuzzing gate.

## 2026-09-14 - Export and replay API and deferred-window diagnostics

Semantic profile 5 exports the core API/window context in explicit 64-byte
wire events. The profile admits deferred write/truncate/fsync/commit steps;
tests join two acknowledged groups across distinct API roots and verify exact
old or acknowledged bytes after cuts around the first group's durable boundary.
The scope remains optional, bounded and separate from mount/recovery coverage.

Evidence in `build/api-window-export-eplft3wz` retains 60 version-5 cases,
including filtering, ring overwrite, consumer disconnection, cuts and wrong-byte
negative controls. All 39 historical cases match six artifacts byte for byte.
The 24-test Python replay run had 23 successes and one source/runner-change
refusal while Cargo rebuilt the executable; the refused test passed using the
retained fixed runner. The refusal was preserved, not counted as a semantic
success. Nine admission tests and 15 Rust scenario tests pass. Formatting,
Clippy and the seven-target 28,672-case codec gate pass; the full workspace
reports 556 passed, zero failed and 10 ignored across 89 result groups.

At the owner's request, the three unfinished Stage A roadmap entries link to
one task-level status owner. Each subtask states its source requirement and
closure evidence; discovered prerequisites must identify their parent gate
before becoming work units. This export/replay subtask closes, while the
broader diagnostic gate retains the explicit subsystem inventory and coverage
requirements. No native or later-stage requirement was removed.

## 2026-09-14 - Map Stage A acceptance gates to visible roadmap entries

The owner pointed out that the eight-gate status report was not understandable
from Stage A's visible list. The roadmap now maps each scoped acceptance gate
to those entries and explains the broader scope of the milestone links. The
namespace-independence rule stays visible in an explicitly ongoing section,
excluded from finite completion. This changes presentation, not requirements
or completion verdicts.

## 2026-09-14 - Correlate deferred windows and qualify the real FSKit boundary

The optional flight recorder joins staged API calls, intent groups and commit
attempts with recorder-local window identities. Failed log writes identify the
attempted group without acknowledging it; closing a window does not assert
rollback or durability. Draining preserves context, and actual detachment or
late enablement records an explicit observation boundary. Legacy profiles 1–4
retain their original recording scope and wire bytes.

Four-cache-profile comparisons cover validation refusals, write/truncate and
barrier failures, snapshot publication and empty windows. The host layout probe
measured 72-byte events and a 176-byte recorder/Option, compared with 64/152
before this change. The 39 retained replay cases matched all six compared
artifacts exactly; the 20 Python replay tests and 18 documentation fixtures
passed. Formatting, Clippy and the seven-target 28,672-case codec gate passed;
the full Rust workspace completed 89 groups with 555 passed, zero failed and
10 ignored tests. Evidence is retained in `build/window-observation-2kbwudqc`;
the real FUSE gate below was executed separately from those ignored defaults.

With the owner's authorization, macFUSE 5.3.3 was installed from its signed,
notarized official package on macOS 26.6.2 (25G83). After the owner enabled both
FSKit modules, ExtensionKit still refused their identity during startup.
The official `macfuse install --force` command was then run, and the owner
entered the administrator password in the authorization dialog. After that
authorized registration, the startup failure disappeared. The explicit real-mount test passed in 7.62 seconds, including
host fsync, namespace operations, unmount and final image integrity. Its binary
hashes and environment are retained in `real-fuse-qualification.json`. No kernel
security settings were changed. This host result does not qualify native AROS
hardware or other host versions.

## 2026-09-14 - Preserve open transaction windows after preflight refusals

While reviewing deferred-operation correlation, reproduced a window ownership
bug: cancellation preflight took the open window out of Volume, then propagated
an error before restoring it. Deleting through a file as parent returned
NotDirectory and changed two pending operations to zero. The same path could
lose ownership of an acknowledged prefix alongside later unlogged work.

The preflight error path restores its read-only window. After an operation
mutates state, an internal failure to construct its log record requires remount
before another mutation. Log construction precedes unlogged-list edits. Neither
fix changes the wire format or public filesystem ABI.

Two focused regressions cover 16 cache/prefix/error-path combinations. Invalid
parents, invalid replacement names and injected read failure preserve pending
counts without writes or barriers; retry, fsync and remount recover exact bytes
with a clean checker. The original failing run is retained alongside the fixed
runs. Format, strict all-feature Clippy and seven fuzz targets at 4,096 cases
each pass. The full workspace reports 548 passed, zero failures, 10 explicitly
ignored tests and 89 result groups. Eighteen documentation fixtures and the
read-only checker pass. Thirty-nine retained replay cases match six artifacts
each, including full images, block traces and flight events. The sealed local
evidence is `build/window-preflight-arqm209s`.

## 2026-09-14 - Compute stage progress from scoped acceptance gates

Replaced Stage A's inherited whole-milestone completion calculation with eight
explicit finite gates. Five are complete: executable core, device harness,
checkpoint/cut verification, host accounting and reproducible replay. Core
flight coverage, mutation-family cache coverage and codec/property coverage
remain partial. The stage therefore stays partial, with the outstanding work
visible in its own acceptance inventory.

A shared milestone can keep its platform, hardware or format-freeze requirements
without reopening a completed earlier scope. Other stages retain conservative
milestone aggregation until their individual scope audit. The namespace
independence maintenance item is explicitly ongoing and stays unstruck.

The generator checks the roadmap's exact required-ID declaration against the
status owner's rows before writing anything. Tests cover both disagreements
between shared milestones and scoped gates, ongoing exclusion, missing/extra
rows, duplicate ownership/declarations, invalid states, absent evidence and
read-only/error atomicity. Eighteen temporary-fixture tests, documentation
validation, reproducible navigation and whitespace checks pass. No filesystem
implementation changed in this unit.

## 2026-09-14 - Correlate core API calls with checkpoint attempts

Added opt-in API spans around 66 mutable operational Volume entries. Nested
calls share their root operation ID and keep separate span/parent identities;
commit events carry the active context. Early refusals and Rust unwinding have
explicit outcomes. A guard restores parent context, and a source coverage test
checks the mutable entry registry against its wrappers.

Targeted comparisons cover four cache profiles, every barrier of a non-empty
file creation, duplicate names, snapshot preservation, protection changes,
active-handle deletion refusal and an injected provider unwind before writes.
The first barrier test expected checkpoint uncertainty one flush too early;
the corrected scenario includes the preceding data barrier and all three cuts.
The semantic outcomes, device traces and complete images match plain execution.

The measured macOS AArch64 event layout grew from 32 to 64 bytes; the recorder
and its Option grew from 112 to 152 bytes. Optional small rings retain bounded
storage with explicit diagnostic loss. Native and other-architecture resource
qualification is separate. Semantic profiles 1 through 4 keep API observation
disabled; 39 retained cases reproduce their flight bytes and semantic verdicts,
including five selected cuts and two expected negative cases.

Core call spans are one scope of the observability requirement. The queue names
extended export, deferred-window/intent correlation, object/block/view metadata,
mount/recovery and platform adapters as subsequent work. API success describes
a returned result; failure and unwinding do not promise rollback.

Local retained evidence: `build/api-spans-707ic93v`, including private source,
layout probes, copied runner and fresh legacy replay bundles. Qualification
passed format, strict all-feature Clippy, seven fuzz targets at 4,096 cases each,
546 workspace tests (zero failures, 10 explicitly ignored, 89 result groups),
20 replay tests and 13 documentation fixtures. All 39 legacy cases match six
artifacts exactly, including full images and block traces.

## 2026-09-14 — Replay selected diagnostics and bounded consumer delivery

Semantic profile 4 and AFSFLT03 bind category selection and a deterministic
bounded consumer to persistent replay. Sequence/attempt endpoints make fully
filtered batches explicit; separate loss and delivery counters obey independently
checked conservation rules. The queue is serviced between operations and can
be disconnected at a specified operation. Earlier protocol bytes are preserved.
This adds no on-disk filesystem fields and does not model arbitrary external
consumer timing.

The Rust scenario tests passed 640 profile/mask/ring/consumer combinations with
unchanged I/O and images, plus malformed-header admission. Twenty Python replay
tests passed, including fresh-process comparisons, inconsistent counters and
identities, preserved bundles, minimized failures and five selected-cut variants.
A refused duplicate create preserves the prior counters without inventing a
commit failure or delivery to a disconnected consumer. Seven scenario-admission
and ten rebuilt-comparison contract tests passed too.

Evidence is retained in `build/selected-diagnostics-9xqp6ug_`: 24 successful
version-4 bundles, a wrong-byte failure and its minimized reproducer, five cut
bundles and eight historical bundles. All were replayed; historical flight bytes
match the retained originals exactly. The first source companion binds revision
`24c36c68d7a17bf7019a9037a558dfecfa22aacd` and working-tree digest
`26356be94f20d70d6dbc1a48bfde749a601bfc64f93742b23fa27a3638574b70`.
The final source companion includes the additional runtime-refusal regression,
with working-tree digest
`9a5a2614a2de1da8be4f546e4630b29d8accac4bed7bd5cdf0c5c8120223ad3b`.
Final sequential validation passed 540 workspace tests, zero failures and ten
ignored tests across 88 result groups, plus formatting, Clippy and 28,672 codec
mutation cases. Documentation and all 13 checker fixtures passed; 256 code/build
files matched the final source companion.
API-wide identities, wider internal subsystems and real-consumer scheduling
qualification remain in the full audit queue.


## 2026-09-14 — Filter commit diagnostics and attach a bounded live consumer

The commit-tail recorder gained four runtime categories and a live-adapter
callback. Filtering preserves sequence and attempt assignment, including a
filtered Begin. Intentional filtering, local ring loss and unsuccessful live
delivery have separate saturating counters. A busy adapter permits later
attempts; a closed adapter is retained but not called until explicitly replaced.
The callback is trusted bounded host code, with consumer processing dispatched
outside filesystem operations through a preallocated queue.

Focused validation passed four recorder unit tests and six integration tests.
The selection test covers all 16 masks, all four cache profiles and three barrier
outcomes (192 combinations). The bounded one-event consumer covers all four
profiles and the same three outcomes, including saturation and disconnection;
operation results, device traces and every image block match unobserved execution.
The prior serialized replay profiles keep ALL selection and no live sink, so
selected-category export and wider subsystem identities remain explicit queue work.
Eight historical ALL-category cases (four cache profiles, capacities 1 and 32)
were rerun with the copied runner under `historical-replay`: their flight artifact
bytes match the retained originals exactly, and every semantic oracle passes.

Evidence is retained in `build/live-diagnostics-appscpxr`. The source companion
binds revision `e14afbcb7ce7246b56c08a01bad267e7a84a93f0` and working-tree digest
`dac6b47ceb25c207ae522693c77d17ba9b0d01e1675d2e72971bf4ea79330bbe`. A standalone macOS
AArch64 layout probe, compiled outside Cargo's active artifact directory, measured
recorder/optional-recorder size changing from 64 to 112 bytes; events remain
32 bytes. This is 48 bytes of fixed additional state, including when the optional
field is empty. Disabled diagnostics allocate no ring or adapter. Enabled ring
and transport capacity remain caller-bounded; these layout figures are not a
whole-process memory measurement or a 32-bit target qualification.

Final sequential validation passed 538 workspace tests with zero failures
and 10 ignored across 88 result groups, plus formatting, Clippy and
28,672 codec mutation cases. Documentation checks and all 13 checker fixture
tests passed. The source verification matched 187 build-source files to the
retained companion.


## 2026-09-14 — Exercise allocation codecs and reject undersized encoder output

The standalone Rust fuzz workspace had stopped compiling because its checkpoint
seed lacked `snapshot_roots`. Restoring the absent optional root preserved all
five historical seed fingerprints. The standard Rust quality gate now includes
this workspace, while ordinary constrained component builds exclude the tooling.

Bitmap-page and region-descriptor targets add partial final-page geometry,
independently counted free bits, endpoint mutations and geometry/generation
binding checks. Re-sealed corrupt fields test structural rejection beyond CRCs.
Seven targets passed 4,096 cases each (28,672 total), with saved-input replay.
Artifacts use exclusive durable creation and bounded reads; an existing retained
artifact is refused rather than overwritten.

Short-output tests reproduced arithmetic underflow in seven encoders: bitmap,
region, directory, object map, retired list, intent log and reclaim root. A wider
audit reproduced out-of-bounds slices in reclaim segments and tables too. All
nine paths reject insufficient output capacity before accessing their payload.
All 44 format round-trip regressions passed after correction, including exact
minimum bitmap, region, reclaim-segment and reclaim-table buffers. Valid disk
bytes and public signatures are unchanged.

Evidence is retained under `build/allocation-codec-fzyacrz9`: a copied executable,
six exact allocation artifacts replayed in fresh processes, negative reproduction
logs, format tests, Clippy and the standalone gate. The source companion binds
revision `42200acac170c7be2f98aa097f40300b48d538e2` and working-tree digest
`068ca0e358e69385ea0e5c1be7c106f8d8a9487117ce96f57a22bba054b8a409`.
The first workspace run completed executable tests but failed at doctest linking
with missing compiled dependency crates after overlapping rebuilds. Its full log
is retained; it is not a passing workspace verdict. An intermediate serial run was intentionally stopped when the two additional
reclaim encoder defects were found. The complete code is validated sequentially
without overlapping rebuilds; `final-source` retains the additional corrections
with working-tree digest
`4124fcb4e033f452c89e33d2d2b625073b961045b67167bc091f2f863952d7dc`.
Final sequential validation passed: 533 workspace tests, zero failures,
10 ignored tests across 88 result groups, plus Clippy, formatting and
the seven-target fuzz gate. Six artifacts in `final-replay` were replayed with
the copied final executable; 187 build-source files matched `final-source`.
This is hosted codec qualification; additional codec families, semantic fault
coverage and native platform evidence remain required by the audit queue.











## 2026-09-14 — Qualify seeded semantic properties and reconcile Stage A accounting

The Stage A audit separated a completed measurement harness from later resource
optimization and platform/application qualification. The harness has CPU, RSS,
requested-heap/origin, cache-policy, I/O, barrier and amplification-denominator
proofs, including independent repetitions and negative controls. Its individual
roadmap entry is therefore complete. Stage A itself remains partial; no global
milestone or hardware gate was closed. The stage-label generator also aggregates
whole milestones spanning later stages, so a future stage closure must reconcile
that mapping against its own finite requirements rather than simply completing
all cross-stage milestones.

A deterministic semantic generator now supplies a flat object-graph/byte-array
oracle, independent of filesystem codecs. It covers all nine operation families
of the initial scenario profile, nested setup, long names, block-boundary writes,
truncate growth/shrink, cancellation and cross-directory moves. The same semantic
prefix is run with 2/4/8/unlimited cache profiles. Each case retains its exact
scenario before execution and a complete verified replay bundle afterward, with
bounded aggregate bundle payload and explicit incomplete/error handling.

The campaign in `build/semantic-properties-c_l1936i` passed 24 cases: seeds 1, 7
and 42, prefixes of 48 and 96 operations, and all four profiles. Fresh processes
replayed all 24 exactly; all 266 original campaign files stayed unchanged. Bundle
role payload totaled 172,012,824 bytes. A deliberately changed expected byte was
rejected, and a fresh replay reproduced the semantic failure. The source companion
binds revision `325bbf1605e0d987aea252b844eefa65c56e5a83` and working-tree digest
`ec6e200cb8ae1e5e211d8ade382b6bc49b172a2ac3e0f10bb97c82186fa0f583`; the executed
runner is retained separately. A restored checkout also reproduced one passing
case and the intentional failure after the active documentation changed. This records source availability and exact host
replay, not an independent rebuild or native-device qualification.

Eleven temporary-fixture tests cover independent model examples, a golden seed,
generator bounds, refused source overlap, overwrite refusal, source/runner changes,
execution/publication failures, late completion failure and bundle budget refusal.
Ungenerated API families, allocation/other missing codec targets and the broader
fault matrix remain explicit work; this corpus does not stand for all fuzzing.
The eleven model/publication tests, four command-accounting tests, thirteen
documentation-checker tests and documentation/whitespace gates passed. Rust sources
were unchanged from the preceding 524-test workspace qualification.

## 2026-09-14 — Avoid copying caller payloads into atomic batch staging

Allocation-origin measurements identified an unnecessary overlap: plain batches
kept complete padded copies of caller data while encoding their metadata. The
synchronous API already borrows the caller's contents for the entire operation.
The batch now selects surviving creates by object identity after validation and
passes their payload references to the common commit tail. Full blocks are written
directly and one reusable block zero-pads partial tails. Cancelled creates are
excluded even when their name or allocation is reused. Intent-log windows retain
their existing write-through behavior. Public signatures, disk semantics and
publication barriers did not change, so no format/API decision was superseded.

Measurements retained in `build/borrowed-batch-8zf2vy7j` compare the new tagged
executable against the prior retained source/binary profile. Batch-origin peak
for the fixed 192-file creation fell from 1,665,784 to 877,304 bytes for the
2/4/8-page profiles and from 1,668,344 to 879,864 bytes unlimited. Two-page total
requested-heap peak fell from 18,561,936 to 17,770,672 bytes, including the fixed
16 MiB image. All five workload images and per-phase I/O/payload denominators
matched the earlier measurement. Three alternating-order process measurements per
profile also retained CPU/RSS results. With two pages, tagged-process RSS fell from
23,904,256–24,297,472 to 23,248,896–23,314,432 bytes. CPU ranges overlapped
(0.864–0.907 versus 0.859–0.910 seconds); concurrent workspace testing prevents
an isolated performance claim. These are fixture measurements, not a general
memory ceiling; encoded metadata and other transaction state remain in the
[resource queue](implementation/audit-work-queue.md).

The new regression checks direct use of caller full-block addresses, exact bytes,
zeroed tails, cancellation/name reuse, all four cache profiles and retry without
remount after each data-write or first-barrier failure. A mixed-payload cut matrix
requires exact old or complete new state and a clean raw checker at every modeled
cut. Invalid and fully cancelled small batches issue no writes or flushes.
The retained previous tagged executable fails the new 1 MiB fixture budget
(1,665,784 bytes), providing a negative control for the memory regression.
The full all-features workspace passed 524 tests with zero failures and ten
explicitly ignored tests across 88 suites. Formatting, Clippy, seven ordinary
workload tests, two origin integration tests and thirteen documentation-checker
tests passed.

## 2026-09-14 — Attribute requested allocations to their original context

The optional `allocation-domains` host meter records allocation origin through
resize and release rather than estimating opaque container overhead or confusing
allocation origin with current ownership. The production allocator is unchanged.
A private aligned header supplies the tag; its padding and size are accounted
separately from payload requests. The unsafe boundary stays in the existing host
meter module. Core hooks are safe, feature-controlled observation scopes.

Paired measurements retained in `build/allocation-origin-mxmddr3e` cover small
files and the 2/4/8/unlimited cache profiles. Images and block I/O agree with the
ordinary executable. In batch creation, tree-origin peaks were 40,288, 48,480,
64,864 and 204,128 bytes respectively; batch-origin peaks were 1,665,784 bytes
for the bounded profiles and 1,668,344 bytes unlimited. Allocator-origin peaks
were 36,952 bytes. These independent peaks are not additive. In particular,
limiting cached pages does not bound the prepared batch's whole memory footprint.

Allocator tests cover alignment, zeroing, preserved bytes, successful/failed
resize, header overflow, cross-thread freeing and scope unwinding. The full
all-features workspace gate passed 520 tests with zero failures and ten explicitly
ignored tests across 87 suites; Clippy, formatting, seven ordinary workload tests,
two origin integration tests and thirteen documentation checker tests passed. Integration
checks cover per-origin balance, separate instrumentation costs, unchanged image
and I/O results, and oracle retention/release with RSS observations. Broader
ownership, sustained mixed-workload and native resource gates remain open in the
[queue](implementation/audit-work-queue.md); the measurement contract defines
[the interpretation and limits](testing/benchmark-contract.md#allocation-origins-and-instrumentation-cost).

## 2026-09-14 - Measure resident memory around workload phases

Added opt-in `--resident-rounds 3..32` to the host workload. A safe-Rust, fixed
self-only ps query records RSS outside each phase's requested-heap/wall interval,
with separate probe timing. Default execution keeps its original report profile.
Repeated reads follow exact initial verification, with measured oracle preparation
and release, unchanged generation and zero write/flush assertions. Version-2
reports retain each sample and its observed range rather than asserting a plateau.

Four measurement unit tests, seven workload integration tests, formatting and
all-target/all-feature Clippy pass. The enabled/disabled image and original I/O
comparisons cover small files and cache profiles 2/4/8/unlimited. A separate
16-round observation retained five workload/process report pairs and source,
runner and ps hashes in ignored `build/resident-series-21ddwr0y`; each repeated
read matched exact bytes and caused no image writes. Full offline workspace
validation passes: 517 tests passed, 10 ignored, none failed. Documentation
validation and its 13 tests pass. RSS includes observer/runtime/fixture state; individual ownership,
sustained mixed workload behavior, Linux execution and native evidence remain open.

## 2026-09-14 - Bind internal commit diagnostics to semantic replay

Added optional bounded per-operation capture to the semantic runner, carrying the
same ring through explicit remounts and draining it between operations. A one-record
ring reports exact overwrites; already exported records never inflate loss counts.
Scenario version 3 binds cache policy and a 1..256 event capacity, exporting
AFSFLT02 internal records alongside semantic identities and block ranges. Older
profiles preserve their operation-only bytes. Admission checks capacity, bounded
records, identities, sequence/loss consistency and malformed/trailing input.
Minimization binds diagnostic capacity; selected-cut traces remain full recording
context rather than a claim about post-cut execution.

The 12 Rust scenario tests and 16 Python semantic tests pass, including four-profile
byte/I/O equality, failed commits, fresh-process replay, overwritten records,
resealed corruption, minimized failure and selected cuts. Seven scenario-admission
Python tests and ten rebuilt-comparison tests pass, including a distinct runner
identity and a changed internal event that leaves semantic results passing but
fails artifact comparison and returns CLI status 2. These wrapper-based tests do
not attest an independent compilation. Formatting and all-target/all-feature
Clippy pass; full offline workspace validation passes with 516 tests passed,
10 ignored and none failed. Documentation validation and its 13 tests pass. API-wide
identities, category masks, callbacks and other internal subsystem/recovery coverage
remain in the Stage A queue; no on-disk or hardware claim changes.

## 2026-09-14 - Observe the common checkpoint publication tail

Added an optional caller-sized flight ring to the common Volume commit tail,
including snapshot registry commits through that same function. Emission does
not allocate or read a clock; attempts distinguish retries at the same checkpoint
generation. Events distinguish data/metadata completion, publication beginning,
checkpoint barrier completion, root adoption and failure. An error after
publication reports the remount requirement without inventing a durability verdict.

Four integration tests pass: exact enabled/disabled I/O and image equality with
a three-record ring, prepublication retry versus final-barrier uncertainty,
checkpoint-write failure, and failed adoption after a successful checkpoint
barrier. A unit test covers fixed storage capacity and identifier exhaustion.
Full offline workspace validation passes: 514 tests passed, 10 ignored, none failed.
Formatting, all-target/all-feature Clippy with warnings denied, documentation
validation and its 13 checker tests also pass. API-wide identities, category masks,
callbacks, export and other subsystem coverage remain open; the semantic bundle
flight artifact is unchanged. No on-disk format or physical-provider claim changes.

## 2026-09-14 - Automate retained-host replay reconstruction

Added `rebuild-replay.py` with exclusive toolchain sealing, exact retained-tree
verification and complete reconstruction orchestration. The driver restores the
source package, verifies registry dependencies, uses copied Rust/Clang/linker/SDK
inputs with a fresh Cargo home and target per case, and requires positive build
plus three independently meaningful negative controls. It compares retained
bundles, rechecks original and input identities, and publishes a completion report
binding inputs, logs, rebuilt binary, comparison reports and all six retained
driver scripts. It also records the host Python/Git observations. The shared command
helper retains bounded combined logs and exposes nonzero command status without
weakening existing source/dependency behavior.

The 14 protocol regressions cover sealing changes, host/path/role/integrity
refusal, four-profile orchestration, preserved failures, changed retained inputs,
negative-control reasons, output collisions and early/late publication errors.
An additional focused negative-control case rejects diagnostic-looking output
with a non-Cargo exit code. Build environments explicitly record the bounded
whitelist, including inherited HOME/TMPDIR paths; unrelated flags and wrappers do
not enter the process environment.

A fresh CLI reconstruction passed against the original copied-toolchain probe in
`/private/tmp/afsplus-orchestrated-rebuild-3xc9tw7h`. A second experiment copied the
four retained tool roots into a new directory with spaces, sealed its 42,147-entry
inventory using the new command, then rebuilt from source and dependency packages
into another path with spaces. Positive build exited 0; absent linker, empty SDK
and empty registry-source controls each exited 101 for their intended reasons.
All four cache-profile comparisons were semantically successful and byte-equal.
The completion report's thirteen bound evidence files were independently checked
against their recorded sizes and digests.

The second experiment is retained in
`/private/tmp/afsplus-toolchain-sealed-m_1h848i`, with its new tool manifest,
`rebuilt job/qualification.json`, inputs, logs and paired bundles. These are
Darwin ARM64 host-profile results, not bit-identical executable, cross-host or
hardware claims. A standalone copy of all six driver scripts also completed the
entire reconstruction outside the development checkout in
`/private/tmp/afsplus-standalone-rebuild-uzhn3p4p`; its nineteen bound evidence files
include the retained driver sources and qualifier-host observations.

The final source, dependency, toolchain, original-bundle and driver inputs were
copied into the repository's Git-ignored
`build/replay-reconstruction-mr66m1gq` directory without moving or overwriting any
prior evidence. Reconstruction passed again using only those retained input paths.
`result/qualification.json` records the build, all three intended control failures
and four exact comparisons. Copied input files/directories were synchronized;
`retained-inputs.json` binds their manifests and the completed result. This local
retention set is excluded from Git publication, so the qualified inputs do not
exist only under temporary directories.

The host reconstruction prerequisite is implemented; Stage A's
transaction-internal flight diagnostics, broader publication/cache families and
full resource-accounting evidence remain open. No filesystem core or disk-format
change was made in this unit.

## 2026-09-14 - Reconstruct replay with copied tools and SDK

An isolated Darwin ARM64 experiment copied the Rust toolchain, selected Apple
Clang/linker and their four non-system linker libraries, Clang resources and the
MacOSX27.0 SDK. The copied inventory matched source bytes, modes and symlink targets
for 42,147 entries, containing 1,687,125,731 regular-file bytes. Files and directory
entries were synchronized before the copy manifest was recorded. The experiment
used macOS 26.6.2, build 25G83; its system runtime libraries remain an explicit host
prerequisite, separate from the copied tools and SDK.

A frozen offline build from restored `ae5edbd` sources and retained registry
sources passed with a fresh Cargo home and target. All 13 verbose Rust compilation
commands selected the copied Rust sysroot, Clang and linker; SDK/resource arguments
and deployment target 11.0 were explicit. A nonexistent selected linker failed
with the expected Clang linker-selection error. An empty SDK failed to find the
System library. Both controls failed while linking a host build script, confirming
that tool selection also reached that part of the build.

The new runner reproduced all eight non-metadata roles for the 2/4/8/unlimited
cache profiles, preserving every original bundle hash. The copied tools, copy
manifest, selected build inputs, positive/negative logs and four comparisons are
retained in `/private/tmp/afsplus-toolchain-probe-67a10vfm`; `qualification.json`
binds the source, dependency, copy and build-input manifest hashes.

This is a real same-host reconstruction experiment, not a reusable toolchain
package implementation, bit-identical executable claim or another-host result.
The next implementation is the verified package/build orchestration around these
measured inputs. Other-host qualification has an explicit M01/M12 Stage D queue
entry; it is not an added all-platform prerequisite for Stage A. Existing flight,
resource/cache, native-provider and complete audit-queue requirements are unchanged.

## 2026-09-14 - Rebuild replay with retained dependencies and an empty Cargo cache

Added source-bound offline dependency capture and verification. The package binds
all vendored file paths, sizes, modes and hashes to the observed source and lockfile,
with observed Cargo/rustc executable identities. Cargo's tree is synchronized
before manifest publication; missing or altered files, input changes and partial
output are refused. The shared source helper supplies bounded command output and
timeouts for the fixed Git and Cargo commands.

The prepared host cache initially lacked `redox_syscall 0.5.18`, which is locked
but not needed for this Mac's build. The exact missing public locked packages
were fetched through Cargo without changing the source lockfile or publishing
project content. The new capture command itself is always offline. The complete
registry set contains 5,696 files and 135,866,715 bytes, including other-platform
dependencies; their availability is not other-platform build qualification.

The real source-package checkout of `ae5edbd` compiled with `--frozen --offline`,
an initially empty Cargo home, a fresh external target and only the retained
registry directory. The resulting Cargo home contained bookkeeping and a registry
cache tag, no downloaded dependency source or archive. A second fresh build with
an empty replacement directory failed because `caseless` was absent, proving the
negative control reached dependency resolution. The new runner reproduced all
eight non-metadata roles for all four retained cache profiles; the originals and
dependency inventory were unchanged. Build inputs, logs, negative control,
comparisons and `qualification.json` are retained privately in
`/private/tmp/afsplus-deps-qualified-ns5gdy06`.

The dependency protocol has 13 focused regressions; the source helper's existing
12 regressions cover its shared process path. The cold build used the existing
host toolchain and SDK. Preserving their bytes and the explicit build environment
is the next reconstruction gate; neither their observed hashes nor an empty Cargo
cache closes that requirement. No Rust core or filesystem format changed.

## 2026-09-14 - Preserve working sources for independent replay reconstruction

Added private source capture/restore with a Git-history bundle, content-addressed
working files and independently retained staged objects. The manifest binds the
same observed revision and working-tree digest as semantic replay. Dirty contents,
missing files, executable modes, safe symlinks and conflicted index stages survive
restoration. Paths, file kinds, object digests and artifact sizes are admitted
before materialization; expanded file bytes have a separate bound so repeated
references to one deduplicated blob cannot exceed the working-tree budget.

The 12 focused regressions passed, including capture changes, unsafe paths,
corrupted/symlinked artifacts, output collisions and late publication errors.
An initial filename fixture used invalid UTF-8 bytes that this host filesystem
rejects; the materialized round-trip uses Unicode plus tab/newline names. Special-
file refusal targets tracked special files, because Git does not inventory an
untracked FIFO. Bounded Git-output cancellation also handles a host refusal to
signal a process group without replacing the original admission error.

The real `ae5edbd` source package preserved 563 paths in 7,225,177 bytes of unique
blobs. Restoring it produced the original source digest. A fresh external-target
`cargo build --locked --offline -p afsplus-check --bin afsplus-scenario` from those
restored sources passed; independent comparisons reproduced all eight non-metadata
roles in each of the four retained cache-profile bundles, with unchanged original
hashes. The package, restored sources, build log, comparisons and
`qualification.json` are retained in
`/private/tmp/afsplus-source-qualified-t1pq6p8k`.

This qualifies the source-restoration step on the host. The build used the
existing host toolchain and dependency cache; preserving those inputs and the
build environment is the next reconstruction requirement. No Rust core change,
native hardware result or overall Stage A completion is claimed.

## 2026-09-14 - Compare reconstructed runners without weakening strict replay

Added `afsptest.py compare-rebuilt` with an explicit caller-selected source root.
The command requires the original source identity, admits historical metadata and
expected-state bindings, and compares all eight non-metadata roles byte for byte.
It preserves two complete bundles and publishes a separate report containing both
runner identities, both outcomes and every artifact digest. Exact replay still
refuses a different binary. Matching failure artifacts remain a failure verdict;
matching final files cannot hide differences in diagnostics or block traces.

Nine focused regression tests passed, covering all four cache profiles, passing
and failing scenarios, a selected crash, altered diagnostics, source/metadata
refusal, publication errors and untouched originals. The existing 13 semantic,
seven scenario-admission and six bundle tests also passed. No Rust code changed.

The real isolated offline rebuild of `ae5edbd` was compared through fresh CLI
processes against all four retained cache bundles. Every comparison exited 0,
all eight compared roles matched exactly, each runner identity differed as
expected, and original file hashes were unchanged. New paired artifacts and
`qualification.json` are retained privately in
`/private/tmp/afsplus-rebuilt-cli-ac_wnyst`.
This is automated comparison evidence, not automated build provenance or a
portable source/dependency/toolchain package. Those reconstruction gates remain
open alongside the rest of Stage A.

## 2026-09-14 - Reconstruct the semantic runner in an isolated checkout

An offline, locked debug build of `afsplus-check --bin afsplus-scenario` in a
fresh local clone of `ae5edbd9a9d40b658b2638b5f36f21e69b38eb81`, with a fresh
external target directory and the existing host Cargo cache, reproduced all
eight non-metadata roles of each retained 2/4/8/unlimited cache-profile bundle
byte for byte. Both structural checker verdicts and the semantic verdict passed
in every regenerated bundle. Original bundle file hashes remained unchanged.
The checkout's observed source digest also matched the retained source digest.

The rebuilt executable digest differed: original
`7504565ff12e613807500cd6aa1bf15d65e52aaa855d8879bc47c7079d4e306f`, rebuilt
`2dfd95bc660783766dbc38fbb4a803dbc742d22f79261df373a8bf636c6cb8bd`.
Only `run.json` differed between each original and regenerated bundle. Strict
replay correctly refused all four rebuilt-runner attempts with an identity
mismatch. This is evidence of semantic reconstruction on this host, not a
bit-reproducible build or cross-host qualification. The cause of the executable
difference was not isolated; checkout paths and build environment remain inputs
to investigate rather than an assumed explanation.

The private experiment directory is
`/private/tmp/afsplus-rebuild-r49m5_or`: `build.log`, `identity.json`,
`comparison.json`, the detached source checkout, fresh target directory and four
new `rebuilt-*` bundles. Temporary artifacts are not durable distribution.
The build used the existing Cargo cache, so dependency/toolchain preservation,
dirty-source capture, a portable reconstruction package and automated comparison
remain required. The next implementation must keep exact replay strict and
report rebuilt semantic comparisons separately, preserving both identities and
all original artifacts.

## 2026-09-14 — Bind semantic replay and reduction to cache profiles

Added version-2 semantic scenarios with an explicit 2/4/8/unlimited tree-cache
profile. The compiler emits AFSPSC02, and the runner applies the setting at
initial mount, every remount and selected-result recovery/inspection. Version-1
scenarios preserve their original unlimited behavior and command encoding.
AFSOBS03 and version-3 actual JSON carry the selected profile; successful
inspection checks the effective mounted policy. Failed inspection retains the
attempted setting with its failure verdict.

Independent bundle validation rejects missing, mismatched, downgraded and wrongly
typed policy records even when an edited bundle has valid manifest hashes.
Reduction preserves the volume configuration and includes its cache setting in
the failure signature, so another resource configuration cannot replace the
original reproducer. The proposed replay ADR and the protocol documentation were
updated explicitly; the outer bundle, block trace and semantic flight formats
retain their version-1 framing.

The full directory/file operation ladder passes under all four profiles with
exact content and both checker views. A 120-file wide-name scenario proves that
two-page execution changes actual I/O ordering on both sides of remount.
Fresh-process bundle replay leaves original artifacts untouched. Reduction and
selected-crash observation preserve the four profiles, with negative controls for
missing or contradictory profile evidence. The documented JSON example also
passes the scenario compiler.

Validation: 509 Rust tests passed, 10 explicit qualification tests ignored,
workspace Clippy and formatting passed, 26 replay/admission/bundle Python tests
and thirteen documentation tests passed. The expanded minimizer case was checked
under all four settings. The full Rust log is
`/private/tmp/afsplus-profile-replay-workspace.log`.

Source/build reconstruction, transaction-internal diagnostics and additional
mutation-family cache/fault artifacts remain explicit work. This unit does not
establish physical-storage or classic-machine qualification, and Stage A stays
partial until its finite acceptance evidence is complete.

## 2026-09-14 — Integrate staged-tree cache profiles into transactions and recovery

Carried the optional nonzero mount cache policy into all nineteen volume
transaction constructors and the reserved allocation-root pool. The shared COW
engine consumes the transaction policy, including empty-tree initialization,
snapshot/lifetime edits, shared-reference edits and intent recovery. Existing
callers keep the unlimited default through explicit default mount fields. Changing
policy during an open window refuses, preserving that transaction's resource
contract. The disk format and C ABI did not change.

This refines the implicit unlimited behavior into an explicit runtime choice;
the earlier rationale of letting modern hosts retain their staged working set is
preserved. The policy covers each shared-engine mutation. Bulk builders, pending
publication buffers, batch overlays and total-volume admission remain distinct
resource work. The specification records both pre-eviction and post-eviction
residency because admitting an image temporarily uses one extra staged entry.

Fixed the prerequisite allocation-root cache assumption: its retained node set
must come from every live pool allocation, including spilled nodes. Using only
pending write buffers would omit reachable nodes and permit later premature
reuse. A three-checkpoint regression compares cached sets with fresh traversal
and full sweeps of both selectable checkpoints. Commit accounting also includes
all successful provisional writes; unique final-node counts stay separate.

The new integrated matrix covers 2/4/8/unlimited profiles through a 192-file batch,
deletes and remount; an uncheckpointed fsynced window replays under the same
profile. A real two-page directory split passes every modeled cut. Four early
spill failures preserve the old view and allow retry. Snapshot/shared-survivor
checks preserve historical bytes across deletion and remount in all four profiles.

Retained debug measurements in `/private/tmp/afsplus-cache-qualification-6xwvt3zh`
include phase heap/I/O, per-process CPU/RSS, observed source/binary hashes and a
separate toolchain observation. The 16 MiB fixture is outside the additional
phase heap figures below; batch inputs and publication buffers are included.

| Staged pages | Create peak above entry, bytes | Create writes | Delete peak above entry, bytes | Delete writes |
|---|---:|---:|---:|---:|
| 2 | 1,770,528 | 645 | 119,949 | 199 |
| 4 | 1,782,816 | 438 | 128,141 | 25 |
| 8 | 1,799,200 | 437 | 142,563 | 22 |
| Unlimited | 1,941,024 | 437 | 169,088 | 22 |

The two-page setting saved requested heap while increasing repeated I/O. Four and
eight pages avoided most of that cost on this workload. One debug sample cannot
select a universal default or establish physical-storage performance.

Validation: 507 Rust tests passed, 10 explicit qualification tests ignored,
workspace Clippy and formatting passed, five measurement tests and thirteen
documentation tests passed. The full log is
`/private/tmp/afsplus-integrated-cache-workspace.log`.

Stage A remains partial: the semantic operation ladder and its retained artifacts
need the profile matrix, and the harness requires every mutation suite to cover
its variants. Wider workload, bulk-builder, whole-heap and native requirements
stay in the full queue and their later milestone order. Native hardware evidence
was not produced by these host tests.

## 2026-09-14 — Measure requested heap across real filesystem phases

Added a host-only `afsplus-measure` executable with a narrowly confined System
allocator forwarding boundary. The filesystem crates retain their unsafe-code
prohibition. The meter records live requested bytes, phase peaks and successful
allocation/reallocation deltas; deterministic failure tests prove that refused
requests preserve both counters and the original allocation.

The fixed-memory workload measures format, mount, sixteen-file creation, boundary
writes, truncation, rename, deletion, sync, remount, exact content verification
and both full checker passes. Its 16 MiB fixture and result storage are allocated
before sampling. The block provider and counters allocate no per-I/O records.
The report exposes successful I/O, payload denominators and retained/temporary
heap separately. This closes a measurement blind spot in the earlier bitmap-only
RAM counter, while leaving cache ownership and steady RSS qualification open.

A retained debug run in `/private/tmp/afsplus-heap-qualification-fho1lfxy` contains
phase metrics, per-command CPU/RSS and observed source/binary identity with report
hashes. It used about 0.397 CPU seconds and 21,118,976 bytes peak process RSS.
These are an instrumentation qualification sample, not a performance baseline.
The requested-heap peaks above phase entry were 55,558 bytes for creation and
28,458 bytes for each checker pass; fixture storage stays in the entry baseline.
Allocator overhead, stack and OS memory require separate measurements.

Validation: 501 Rust tests passed, 10 explicit qualification tests ignored,
workspace Clippy and formatting passed; three workload report tests, four
per-command accounting tests and thirteen documentation tests passed. The full
Rust log is `/private/tmp/afsplus-heap-workspace.log`.

The cache audit also confirmed that volume publication uses the unbounded tree
entry point. Before enabling constrained profiles, allocation-root node tracking
must include spilled final nodes and commit metrics must include early spill
writes. Those requirements are recorded in the Stage A status table; the full
queue and all native qualification gates are preserved.

## 2026-09-14 — Explain each completion-table entry

Added readable Task and Stage / phase columns beside the stable item IDs.
Descriptions come from the actual list entries and link directly to their
containing stage or implementation phase. Navigation generation updates them
when source wording changes, preserves completion status and evidence, and
rebases source links for the milestone document. Added a regression covering
nested stage headings, implementation phases, renamed tasks and idempotence.

## 2026-09-14 — Require checker evidence in replay verdicts

Connected raw and recovered full checker reports to the scenario runner and
version-2 observations. Bundle success requires both clean structural views and
the exact expected namespace/content; warnings and full diagnostic reports are
retained. Recovery operates on a private copy of the selected image. The
minimizer preserves structural failures and replay compares complete reports.

The negative control keeps allocation counts consistent while marking a
reachable object block free. Namespace reads succeed, but both checker views
reject the defect. An initial corpus-based fixture had pending intent-log
recovery, which reused the misclassified block before observation; replacing it
with a synchronized fixture tests the intended distinction directly.
Validation passed: 498 Rust tests (10 ignored), workspace Clippy and formatting,
20 Python replay/bundle tests, twelve documentation-checker tests, documentation
validation and whitespace checks.

## 2026-09-14 — Show completion on individual stage and phase entries

Added generated strikethrough to completed individual roadmap and implementation
list entries, with status/evidence records in the milestone document. Partial and
ongoing items remain unstruck. Completion of a component is separate from the
containing phase's acceptance and platform gates. Phase 0 retains the actual
reader-format freeze requirement and points to Stage F; implemented codecs are
not falsely marked frozen.

Documented the rule and extended navigation validation for item identities,
evidence links, duplicate/orphan markers, idempotence and reopening. Validation
is read-only by default and checks all item bindings before writing files.
Also restored two shadowed progress tests by giving their test class a unique
name and using the authoritative milestone-table header in its fixture.
All twelve documentation/checker tests, generated navigation, documentation and
whitespace checks passed. Checker-verdict implementation work is separate and
continues after this documentation unit.

## 2026-09-14 — Bind selected crash states to replay bundles

Connected anchored crash selection to the private semantic runner and bundle
workflow. The direct selector matches every variant of the existing bounded
subset/tear oracle in the fixture, including repeated writes and 512-byte/4-KiB
devices. Python independently reconstructs the selected image from the retained
trace before replay. Tests check pre-publication loss/full/tear states, committed
content at the durable boundary, fresh-process replay and invalid selection.

Crash minimization retains the target operation and remaps its index after
earlier deletions. It preserves the local offset, variant, unflushed-tail size
and observable failure, preventing a reduction from silently changing the fault
kind. The successful baseline trace is retained separately in meaning from the
selected result image. Baseline recording must complete for this profile.

Stage A still needs publication-family coverage, transaction-internal diagnostics,
source/build reconstruction and resource/cache qualification. The selected-state
transport does not prove every publication path or physical storage behavior.
Validation passed: 496 Rust tests (10 ignored), workspace Clippy and formatting,
19 Python replay/bundle tests, seven documentation-checker fixtures, documentation
validation and whitespace checks.

## 2026-09-14 — Integrate semantic replay bundles and failure reduction

Added the memory-runner executable and private afsptest run/replay/minimize
workflow. Fresh runner processes emit exact images, a bound block trace,
namespace/content observations and semantic event ranges with resolved object
IDs. The Python tool independently validates trace/base/result binding and
replays every artifact byte under an explicit no-cut fault model. A wrong
expected state remains a failure after replay.

Bounded deletion-based minimization preserves the same observable failure,
rejects broken label dependencies and records budget exhaustion. Original
bundles remain untouched. Tests cover fresh processes, tampered/resealed bindings,
operation failures, export admission, irrelevant-operation reduction and limited
search. A Rust regression verifies log budgets and event ranges across remounts.

The source digest describes the observed working tree and the executable has its
own digest. This does not attest build provenance or reconstruct an unavailable
dirty source tree. Crash-cut integration, transaction-internal diagnostics and
source/build reconstruction remain explicit Stage A work; this commit does not
close the stage.
Validation passed: 494 Rust tests (10 ignored), workspace Clippy and formatting,
17 Python replay/bundle tests, seven documentation-checker fixtures, documentation
validation and whitespace checks.

## 2026-09-14 — Add bounded semantic runner and bundle admission components

Connected the admitted operation protocol to real in-memory filesystem operations:
create, write, truncate, rename, unlink, directories, sync and remount. Five Rust
regressions check exact namespace/content, reconstruction from every recorded
write, bounded recording refusal, retained failures and observation admission.
Recording limits apply across remounts and reject before mutation; a mount failure
no longer discards the captured fixture. Plan construction goes through admission.

Added Python scenario validation/compiler fixtures and exclusive bundle
publication/integrity fixtures. Publication tests inject each write/sync failure
and preserve existing paths. The proposed ADR-099 retains the full replay contract.
These are components of Stage A, not closure of its replay gate: integrated
fresh-process semantic replay, revision/fault binding, flight-record export and
failure-preserving minimization are the next work. Resource caps here describe
captured records and inspection output, not complete core RAM accounting.
Validation: workspace tests passed (493 passed, 10 ignored), workspace Clippy
and formatting passed, all eleven Python replay fixtures and seven documentation
checker fixtures passed, and documentation/whitespace validation passed.

## 2026-09-14 - Preserve bounded block-operation replay traces

[ADR-098](adr/ADR-098-bound-block-replay-traces.md) binds versioned recorded writes
and flushes to block geometry and the complete logical base-image identity.
The codec admits operation/wire/base limits, verifies the artifact digest before
returning decoded operations, and rejects malformed records and trailing bytes.
Base verification is exhaustive read-only harness work, separate from normal mount.

Tests cover every truncated prefix and single-byte mutation, resealed malformed
headers/records, bounded admission and modified-base refusal without writes.
A persisted file round-trip feeds decoded operations into the overlay cut oracle;
Python independently verifies the geometry, operations and both SHA-256 digests.
The all-feature offline workspace suite passed 488 tests with zero failures and
ten ignored qualification tests across 81 suites. Clippy, formatting,
documentation, seven checker fixtures and whitespace checks passed. Semantic
bundles, fresh-run reproduction and minimization remain open Stage A work.

## 2026-09-14 - Add bounded memory overlay branches and cut-state replay

[ADR-097](adr/ADR-097-bounded-memory-overlay-branches.md) defines shared immutable
bases and payloads with independent branch indexes. Admission limits live branches
and aggregate entries across forks; failure rolls back charges and branch drop
returns capacity. Exact filesystem tests preserve base images and diverging branch
names/bytes through remount, and budget exhaustion preserves acknowledged contents.

The overlay cut enumerator matches the existing memory-image oracle byte for byte
and description for description across completed barriers, lost-write subsets,
repeated block writes and sampled tears. Publication cuts recover exact old/new
namespace states. Budget/callback failures explicitly report incomplete enumeration.
A one-edit fork over both 512-KiB and 512-GiB logical bases copied a 32-byte index
entry, shared its 512-byte payload and copied zero payload bytes on this host.
These are index/payload measurements, not total process memory.

The all-feature workspace suite passed 483 tests with zero failures and ten
ignored qualification tests across 80 suites. Clippy with warnings denied,
formatting, documentation, seven checker fixtures and whitespace checks passed.
This closes the finite Stage A memory-overlay requirement. Persistent replay
packages, minimization, broader cache-profile coverage and full resource accounting
remain open. No persistent-overlay or physical-device durability is claimed.

## 2026-09-14 - Measure host commands with per-child CPU and RSS

Added a shell-free measurement wrapper with exclusive report creation, explicit
exit/signal/error outcomes and normalized macOS/Linux peak-RSS units. It records
per-child CPU and monotonic wall time without collecting the environment. Four
fixture tests cover literal arguments, accounting, failures/signals, spawn errors
and refusal to overwrite an existing report.

Ran the built fsync smoke workload through it and paired the JSON report with
the workload's existing operation, I/O, flush and metadata-amplification tables.
The debug run completed successfully while another suite was running, so this
is collection-path evidence, not a performance baseline. Process CPU/RSS totals
span all workload variants; they are not per-variant attribution or simultaneous
process-tree peak RAM. Full allocator/cache and steady-state accounting remain
open. Documentation checks, seven checker fixtures and whitespace checks pass.

## 2026-09-14 - Prioritize usable stage outcomes across the complete queue

The owner asked to align the active goal with the milestone discussion. Recorded
stage-first selection in the queue's resume instructions: evidence reconciliation,
finite prerequisites and usable outcomes precede further expansion of one feature
thread. Preserved the entire queue, snapshot/recovery context and physical-device
boundaries. The goal-management interface cannot rewrite the active objective;
repository instructions record this priority within its existing full scope.

## 2026-09-14 - Add bounded block-device partition views

[ADR-096](adr/ADR-096-bounded-block-slices.md) defines fixed nonempty slices
with overflow-safe admission, logical access checks, nested translation and
forwarded parent barriers. The wrapper needs no additional block allocation.
Tests verify neighboring sentinels, boundary buffers and parent failure behavior.
A real filesystem cycle formats and mutates two differently offset slices,
checks exact sparse/truncated contents through remount, runs the checker and
confirms every surrounding block is unchanged.

The all-feature workspace suite passed 474 tests with zero failures and ten
ignored qualification tests across 78 suites. Clippy, formatting, documentation,
seven checker fixtures and whitespace validation passed. The finite Stage A
slice requirement is implemented; overlay, replay, resource accounting and other
stage gates remain open. No physical storage was accessed by the qualification.

## 2026-09-14 - Audit finite executable-core stage requirements

Inspected Stage A's explicit deliverables against block-module exports, trace
accounting and crash/corruption tests. The core and host fault-simulation
components exist. Slice and overlay wrappers are missing from the block surface;
complete artifact replay, CPU/RAM accounting and mutation cache-profile coverage
need stronger evidence or implementation. Recorded these in the status home and
ordered their dependencies in the full audit queue. Stage completion remains
partial for finite work, independently of ongoing review and epoch-1 freeze.

## 2026-09-14 - Preserve scoped symlinks in bound archive groups

[ADR-095](adr/ADR-095-bound-symlink-archive-groups.md) binds captured targets,
exact metadata and full/recovery opaque inventories to symlink namespace groups.
Restoration creates through the destination grant, checks exact target readback
and reports preservation or explicit metadata loss. Real-volume tests cover live
unlink after capture, maximum inline targets, independent tar inspection and
remount. Binary security fixture tests cover full/recovery combinations.

Malformed re-enveloped archives exposed a missing-local-linkpath bug: restoration
could accept the raw tar placeholder as the target. The reader now requires the
explicit local target. Regression cases cover descriptors, profiles, names,
timestamps, kind, mode, ordinal and inventory mismatch, with poisoned replay and
released temporary handles on failure. Full opaque restoration needs a third
slot for the staged upload in addition to parent and created-object handles.

The full all-feature workspace run passed 470 tests with zero failures and ten
ignored qualification tests across 77 suites. Clippy with warnings denied,
formatting, seven documentation fixture tests, documentation and whitespace
checks passed. Whole-job enumeration, graph completeness, durable loss reporting,
source mutation/limit fault expansion and native qualification remain separate
work; this commit does not complete the backup/restore queue gate.

## 2026-09-14 - Separate ongoing review from finite stage completion

The owner asked that moving-target work remain visible without preventing stage
completion. Progress aggregation now excludes explicit Ongoing statuses and keeps
all-ongoing groups uncompleted. Stage 0 is ongoing review. M00 retains its finite
epoch-1 reader-format freeze and contributes to Stage F, with independent decoding
and resolution of experimental fields required before closure.

The early milestone audit records executable evidence and remaining integration
and qualification obligations without changing completion labels. The progress
parser reads only the authoritative milestone table; supplemental audit rows
cannot override it. Seven documentation/discovery/progress fixture tests and the
read-only documentation checker pass. The full filesystem queue remains intact.

## 2026-09-14 — Generate visible milestone and stage progress

Added the requested plain/bracketed/struck notation to planning navigation.
Milestone status cells drive generation; stages aggregate contributing
milestones, while the ongoing Stage 0 review is partial. Prototype completion
with qualification gaps stays partial. The documentation gate detects stale
markers, and `make toc` regenerates them while preserving links and anchors.

## 2026-09-14 - Add scoped symlink transport and VFS dispatch

ADR-094 defines explicit opaque-target operations through the VFS and separate
backup/restore grants. Added optional provider operations with NotSupported
fallbacks, held-grant kind checks, bounded target readback and handle admission
before restore creation. VFS unlink avoids regular-file orphan cleanup and
namespace creation preserves pending logged writes through the existing barrier.

Targeted tests cover live and captured bytes, short buffers, remount, grant
revocation, callback lifetime, unsupported providers, foreign handles and handle
exhaustion. Archive target/profile binding, replacement and native adapters are
separate work; C capability identities are unchanged.

## 2026-09-14 — Refresh implementation navigation and remaining work

Applied the approved docs-refresh review against commit f711cff and the local
working tree. Added phase/stage/milestone navigation; corrected the roadmap's
conditional hybrid-policy and intent-log experiments to ADR-062/063/064/065.
The replaced roadmap text asked to prototype full COW versus private in-place
and add an intent log only after measurement; the accepted ADRs own that
measurement and its decisions. Updated symlink codec and API summaries,
separated snapshot component integration from whole-job backup qualification,
and added snapshot/backup deliverables to the implementation plan.

Source and test inspection establish these corrections. Cargo was unavailable
in this session, so no fresh Rust test result is claimed. Hardware and complete
consumer qualification remain explicit work. Acceptance gates are unchanged.

## 2026-09-14 - Preserve symlink targets through core namespace transactions

Added core symlink creation, bounded live/captured target reads, final unlink,
and payload-preserving metadata and rename rewrites. Explicit metadata decoding
validates the complete inline payload. Directory validation and live/historical
ownership checking accept symlink metadata without treating targets as extents.
Short buffers return the required length and remain unchanged.

Targeted tests cover metadata edits, cross-directory rename, remount, final
unlink, retained snapshots and zero-write refusal. Modeled publication cuts for
create/rename/unlink/restore require complete old or new namespace and metadata,
exact captured bytes and exhaustive checking. Atomic replacement, VFS/backup
capabilities, OS adapters and native qualification retain separate gates.

## 2026-09-14 — Simplify ADR decision statuses

Normalized decision labels and removed approval attribution from ADR headers.
The index links implementation progress and the detailed work queue. Acceptance
is separate from implementation completion and format freeze. Proposed and
superseded decisions retain their lifecycle meaning; ADR-026 is partially
accepted because its public AtomicBatch API is proposed.

Technical qualifications removed from the status labels are retained below as
historical context; their linked decisions and milestone gates govern scope.

- [ADR-007](adr/ADR-007-utf8-nfc.md): Accepted for the executable prototype; epoch-1 interoperability validation pending.
- [ADR-008](adr/ADR-008-case-policy.md): Accepted; volume-default implementation complete, per-directory override pending.
- [ADR-009](adr/ADR-009-journal.md): Architecture resolved by ADR-063; final epoch-1 wire freeze remains M14.
- [ADR-018](adr/ADR-018-inline-data-optional.md): Reopened after PFS4 review.
- [ADR-020](adr/ADR-020-checkpoint-commit.md): Mechanism accepted by ADR-063; final epoch-1 wire and real-device gates remain M14.
- [ADR-022](adr/ADR-022-cache-pinning.md): Accepted as an implementation invariant.
- [ADR-026](adr/ADR-026-bounded-atomic-batches.md): Internal group-commit mechanism accepted by ADR-063; public AtomicBatch API remains proposed.
- [ADR-027](adr/ADR-027-reflink-clones.md): Accepted as an epoch-1 format requirement; implementation may be staged.
- [ADR-028](adr/ADR-028-rust-reference-core.md): Accepted implementation direction.
- [ADR-029](adr/ADR-029-dual-reference-implementations.md): Accepted project policy.
- [ADR-031](adr/ADR-031-portable-security-acls.md): Proposed, split into container-first and semantics-later phases.
- [ADR-034](adr/ADR-034-bounded-cow-tree.md): Accepted for the prototype; four authoritative adapters published.
- [ADR-035](adr/ADR-035-allocation-root-reserved-pool.md): Accepted and authoritative in the executable prototype.
- [ADR-036](adr/ADR-036-reclaim-queue.md): Accepted for the prototype (Reclaim Scale-2); wire format experimental.
- [ADR-037](adr/ADR-037-intent-log.md): Mechanism accepted by ADR-063; record wire remains experimental.
- [ADR-038](adr/ADR-038-mount-policy-and-feature-summary.md): Accepted for the Mountable Alpha-0 prototype.
- [ADR-039](adr/ADR-039-portable-vfs-slice.md): Accepted for Mountable Alpha-0.
- [ADR-040](adr/ADR-040-fuse-protocol-boundary.md): Accepted for Mountable Alpha-0.
- [ADR-041](adr/ADR-041-aros-dos-adapter.md): Accepted for Mountable Alpha-0.
- [ADR-042](adr/ADR-042-aros-c-boundary.md): Accepted for Mountable Alpha-0.
- [ADR-043](adr/ADR-043-native-aros-dospacket-translator.md): Accepted for Mountable Alpha-0.
- [ADR-044](adr/ADR-044-aros-trackdisk-viewport.md): Accepted for Mountable Alpha-0.
- [ADR-045](adr/ADR-045-native-aros-handler-shell.md): Accepted for Mountable Alpha-0 integration.
- [ADR-046](adr/ADR-046-hosted-aros-same-image.md): Accepted for Mountable Alpha-0 S0.
- [ADR-047](adr/ADR-047-hosted-aros-crash-replay.md): Accepted for Hosted and native-QEMU qualification.
- [ADR-048](adr/ADR-048-hosted-aros-system-pivot.md): Accepted; S1a and Hosted S1b qualified.
- [ADR-049](adr/ADR-049-hosted-aros-desktop-pivot.md): Accepted for Hosted S1b.
- [ADR-050](adr/ADR-050-external-aros-handler-lifecycle.md): Accepted for Hosted and native-QEMU lifecycle.
- [ADR-051](adr/ADR-051-explicit-aros-aarch64-platform-profiles.md): Accepted; apple-aarch64 pre-hardware runtime profile qualified.
- [ADR-052](adr/ADR-052-versioned-directory-comparison-keys.md): Accepted for the executable prototype.
- [ADR-053](adr/ADR-053-native-macaros-retained-image-transport.md): Accepted for pre-hardware QEMU qualification.
- [ADR-054](adr/ADR-054-native-macaros-crash-replay-extraction.md): Accepted for native MacAROS pre-hardware qualification.
- [ADR-055](adr/ADR-055-aros-m68k-emulator-gate.md): Accepted for pre-hardware qualification.
- [ADR-056](adr/ADR-056-native-aros-m68k-alpha0-and-replay.md): Accepted for the M68020-or-newer emulator reference profile.
- [ADR-057](adr/ADR-057-plain-m68000-emulator-gate.md): Accepted for A500-configured emulator functionality and recovery.
- [ADR-058](adr/ADR-058-m68000-memory-and-restart-lifecycle.md): Accepted for the A500-configured emulator profile.
- [ADR-059](adr/ADR-059-guest-failure-diagnostics-are-gate-verdicts.md): Accepted for all current AROS runtime gates.
- [ADR-061](adr/ADR-061-shared-extent-references.md): Accepted; wire format experimental until M14.
- [ADR-062](adr/ADR-062-explicit-hybrid-data-updates.md): Accepted as the epoch-1 data-update architecture; the persistent policy encoding is assigned by ADR-065.
- [ADR-063](adr/ADR-063-intent-log-epoch1.md): Accepted as the epoch-1 durability architecture; intent-log wire remains experimental.
- [ADR-064](adr/ADR-064-intent-log-data-update-compatibility.md): Accepted for the experimental version-3 record set; final wire freeze remains M14.
- [ADR-065](adr/ADR-065-persistent-data-update-policy.md): Accepted; wire format experimental until M14.
- [ADR-066](adr/ADR-066-bounded-orphan-directory.md): Accepted; wire format experimental until M14.
- [ADR-067](adr/ADR-067-epoch1-allocation-state.md): Accepted; exact wire format remains experimental until M14.
- [ADR-069](adr/ADR-069-consistent-snapshots-first.md): Accepted direction; retention policy and on-disk representation require further decisions.
- [ADR-070](adr/ADR-070-persistent-snapshot-priority.md): Accepted direction; persistent representation and retention policy require prototype evidence.
- [ADR-071](adr/ADR-071-snapshot-lifetime-prototype.md): Accepted for the integrated experiment; shipping wire and resource qualification open.
- [ADR-072](adr/ADR-072-snapshot-record-codecs.md): Accepted for the ADR-071 prototype; integration and wire freeze require qualification.
- [ADR-073](adr/ADR-073-snapshot-checkpoint-roots.md): Accepted for the ADR-071 prototype; writable snapshot support requires integration.
- [ADR-080](adr/ADR-080-pax-completion-envelope.md): Superseded by ADR-081 after independent Python tarfile recovery rejected the terminal global header.

## 2026-09-14 - Cross-read inline symlink records with Rust and C

Accepted ADR-068 for experimental implementation under delegated design authority,
with its full integration and platform qualification requirements retained. Added
an explicit borrowed-target Rust codec and an independent heap-free C record
validator. Fixed-record operations reject symlinks until mutation paths preserve
the variable payload; the standalone C function does not enable volume writes.

Strict and sanitized C probes cross-read relative, absolute, AROS-qualified,
Unicode and maximum-length Rust records and reject fourteen malformed variants
per target. Rust also checks every undersized encoding buffer, truncated input,
reserved fields and nonzero unused tails. Complete namespace mutations, snapshot
reads, adapters and platform qualification remain separate implementation work.

## 2026-09-14 - Reject undersized object encoder buffers

Reviewing the fixed record before the symlink extension found that an empty
file encoded into a caller-selected buffer shorter than 128 bytes could panic
while slicing the common header or fixed fields. The regression reproduced the
zero-length panic before the fix. The encoder checks the fixed header/payload
minimum before allocation or field writes and returns WrongBufferSize.
The test checks every size from 0 through 127 and round-trips the exact minimum,
one byte above it and the ordinary block size. No valid wire representation
changes. Complete symlink codecs and payload-preserving namespace/metadata
rewrites retain their separate implementation gates.

## 2026-09-14 - Bind directory and alias archive restoration

Added ADR-093 directory and hard-link groups to the archive consumer. Directory
restoration checks scoped emptiness, preserves full inventories or explicitly
reports recovery losses, and verifies exact metadata. Alias restoration checks
the primary descriptor and destination capacity before linking, then verifies
shared identity and incremented link count. Added the optional grant-held
one-entry emptiness query without creating another active handle.

Qualification covers a real snapshot directory/file/alias archive, live mutation,
Python tarfile inspection, bounded handle reopening, final directory metadata,
remount, shared writes, malformed bindings, revoked grants and resource refusals.
Full-directory opaque preservation uses a semantic provider; AFS+ opaque storage,
symlinks, complete namespace planning and durable job reports are separate gates.
The specification, API contract, qualification plan and queue preserve that scope.
The full workspace suite passed with 453 tests and 10 ignored qualification
workloads. Formatting, Clippy and documentation checks passed.

## 2026-09-14 - Reconcile the filesystem comparison with executable support

The owner requested an implementation-aware comparison and a personal “Built
by me” row checked only for AFS+. Replaced stale planned labels for implemented
core capabilities with explicit prototype support, linked code and qualification
evidence, and distinguished partial APIs and constrained profiles from complete
platform qualification. Unimplemented facilities retain planned/proposed labels.
Corrected the accelerator discussion: lost change history requires a reset and
rescan, not reconstruction. No filesystem behavior or format changed.

## 2026-09-14 - Reopen created restore entries with bounded active handles

ADR-092 used delegated recommended-option authority to add scoped
`lookup_created`. Namespace restoration needs to revisit files for aliases and
metadata finalization; requiring every handle to stay open would tie active
resource usage to the full object count. The new operation resolves one literal
component in the initially empty, exclusively owned restore destination, holds
the original grant through parent validation and lookup, and uses the existing
handle budget. It neither follows symlinks nor imports arbitrary object IDs.

The semantic fixture walked and reopened 128 levels with two active slots.
Invalid names, exhausted slots and foreign or revoked grants prevented provider
calls. Non-directory and symlink parents stopped before lookup; failed lookups
released their slot. A fresh authorized grant reopened an in-scope entry without
reactivating old revoked handles. The operation-permit probe covered both parent
stat and the actual provider lookup.

The real AFS+ fixture reopened eight nested directories and hard-linked names,
finalized exact file/directory metadata after namespace edits and verified bytes,
identity and metadata after remount. The outside file was untouched and could
not be reached from the restore root. Each measured lookup issued zero writes
and barriers and fewer than 128 reads in this fixture.

The full workspace all-features gate passed 445 tests with zero failures and
10 explicitly ignored qualification probes. Formatting, strict Clippy,
documentation, three checker fixtures and whitespace validation passed.

This is a live isolated-job operation, not pre-existing-destination merge,
overwrite or persistent resume. Native providers must independently qualify
exclusive ownership and no-follow behavior. Archive indexes/path storage and
complete namespace/job orchestration remain separate bounded-resource work.

## 2026-09-14 - Bind exact regular-file metadata to allocation and opaque inventories

ADR-091 used the delegated recommended-option authority to define explicit full
and recovery regular-file groups. The group binds exact metadata to allocation,
sparse contents and optional complete opaque inventories. Full restore refuses a
recovery group before writes. Recovery of a full group validates and drains its
opaque values without destination uploads and reports their counts/bytes and
captured knowledge. Uninspected inventories remain explicit uncertainty.

The real AFS+ snapshot fixture recovered captured bytes, sparse length,
protection and three distinct nanosecond timestamps through remount, despite
later live-source writes. It used 512-byte transfers and one-entry pages and
reported omitted reservations and uninspected inventories. A separate semantic
provider fixture preserved full-width protection, signed timestamp extremes and
unknown binary security values with one-byte transfers. These are different
evidence scopes: the latter does not establish AFS+ opaque storage support.

Rebuilt valid envelopes with conflicting file paths, kinds, mtime, inventory
knowledge/digests and value bindings were refused. Timestamp rounding at the
destination withheld success. Revoked authority, resource limits, mode conflicts,
unknown versions and source export failures poisoned further archive use. Core
metadata was applied after contents/inventories, then read back exactly. Existing
opaque parsing was shared with the recovery drain, with raw/effective ordinal
checks and fallible admitted record allocation.

The full workspace all-features gate passed 441 tests with zero failures and
10 explicitly ignored qualification probes. Formatting, strict Clippy,
documentation, three checker fixtures and whitespace validation passed.

The no-default-features archive gate passed 33 tests. Namespace orchestration
must still preserve directories, symlinks and hard-link aliases, finalize metadata
after namespace edits, validate whole-job completeness, synchronize and persist
loss reports. Native/older-system resource and durability gates remain separate.

## 2026-09-14 - Bind archive allocation records to verified sparse restoration

ADR-090 used the owner's delegated recommended-option authority to bind a
versioned allocation record to its sparse file. The exporter derives both from
one captured plan. Restore validates path, size, ordinal and clipped written
coverage before writes, then reserves in bounded calls and compares committed
destination coverage. Equivalent extent splitting is accepted; changed holes or
written/unwritten state is refused. Explicit recovery skips reservations and
returns their discarded range count and byte sum. The simpler content-only
export path avoids retaining the additional allocation list.

Targeted tests passed captured-source remounts and both restore modes at zero,
1 TiB-plus and maximum-u64 logical lengths, with reservations inside/beyond EOF
and in the final address block. A separate hand-constructed archive exercised
rounded written tails. Unsupported providers, changed reservation locations,
conflicting records/maps, revoked grants and budget exhaustion withheld success;
errors poisoned further archive use. The remount oracle compared independent
logical block/state maps and captured bytes. Codec tests rejected every
truncation, malformed fields/ranges and admitted-limit violations.

The full workspace all-features gate passed 437 tests with zero failures and
10 explicitly ignored qualification probes. Formatting, strict workspace Clippy,
documentation, three checker fixtures and whitespace validation passed.

Python and libarchive sparse extraction passed, including hole preservation and
independent wide-header decoding. Those generic-tool checks establish content
recovery, not reservation preservation. Whole-object metadata/identity binding,
job completion/loss-report persistence, native durability and actual older-system
peak-memory qualification remain separate work. Explicit quotas, small pages and
bounded transfers provide the constrained implementation path without silently
weakening preservation semantics.

## 2026-09-14 - Add scoped allocation readback for full restore verification

ADR-089 chose explicit destination allocation readback before accepting full
reservation preservation. A successful reservation call or matching total byte
count cannot identify changed holes or written/unwritten coverage. The new
restore query holds the original object's revocable grant, validates bounded
page responses and refuses missing provider support. The AFS+ query shares the
bounded extent reader with captured snapshots and adds no format or C ABI change.

Targeted tests passed one-entry enumeration, written data, beyond-EOF
reservations and a final range ending at 2^64, with exact layout after remount.
They rejected malformed pages, invalid budgets, foreign services and revoked
grants. The operation-permit probe covers the query itself. Core tests proved
zero query writes/barriers for empty, direct and tree layouts, and showed that
live allocation changes leave captured layouts unchanged. An open log window
caused explicit refusal without flushing; an explicit window commit made its
new layout available. Ordinary sync intentionally does not commit an open window.

The full workspace all-features gate passed 430 tests with zero failures and
10 explicitly ignored qualification probes. Formatting, strict Clippy,
documentation, three checker fixtures and whitespace validation passed.

This supplies verification evidence for the allocation-preserving archive
consumer. The archive allocation record, binding to sparse data, reservation
restore ordering and semantic coverage comparison remain the next integration
work. Full job and native/older-system qualification remain separate.

## 2026-09-14 - Transport captured sparse contents through the archive consumer

ADR-087 selected GNU sparse PAX 1.0 rather than expanding holes or replacing
ordinary recovery with a private content container. Research used the GNU tar
1.35 manual's sparse-format section, accessed on 2026-09-14. The exporter reads
semantic allocation pages through the retained snapshot grant; the importer
validates a bounded map before writing through the separate destination grant.
Written zeros stay data. Omitted unwritten reservations have explicit range and
byte totals for the enclosing content-recovery loss report.

An independent Python 3.9.6 test exposed conflicting PAX `size` behavior:
applying stored length after sparse metadata destroyed logical length and
following-member alignment. ADR-088 moved stored size into the raw header and
refused the conflicting override. Positive GNU binary size encoding handles the
33-bit octal boundary through unsigned 64-bit maximum without imposing a format
limit at 8 GiB. Python's independent numeric codec matched those headers. These
are representation tests, not multi-gigabyte payload qualification.

The sparse interop script passed Python 3.9.6 and bsdtar 3.5.3/libarchive 3.7.4
for mixed, all-hole and empty entries, including following-member alignment and
non-expanded all-hole extraction. The real AFS+ service test captured a file of
1 TiB plus 23 bytes, changed and shrank the live source after capture, and
restored the original content through verified replay into an isolated AFS+
destination. After synchronization and remount, exact written bytes, sampled
holes, logical size, written allocation and an outside file matched the oracle.
The archive was below 16 KiB, with 8192 written bytes and an 8704-byte sparse
payload. Export issued no source writes/barriers and fewer than 256 block reads.
The report identified two omitted unwritten ranges totaling 8192 bytes.

The workspace all-features gate passed 424 tests with zero failures and 10
explicitly ignored qualification probes. The default archive crate passed
33 tests. Formatting, strict Clippy, documentation, three checker fixtures,
whitespace validation and independent sparse extraction passed.

Targeted cases also passed source revocation during output for data and all-hole
files, wrong path/revoked/nonempty destination refusal, multi-block maps,
maximum logical length, malformed/count/overlap/overflow/padding/size cases,
resource admission and every truncated archive prefix. Ordinary constructors
explicitly refuse sparse fields; opt-in spool verification certifies integrity,
with semantic map validation at the consumer. Full reservation sidecars, object
metadata, namespace/link/inventory binding, complete-job outcomes, spooled large
maps and native resource qualification retain their queue gates.

## 2026-09-14 - Exercise repeated low-space reclamation with retained snapshots

The 24-cycle near-full fixture was extended to persistent snapshots in the
core snapshot harness. Both one-record and eight-record lifetime scan profiles
preserved exact original, rotating-view and live bytes through every remount.
Both selectable checkpoints and their snapshot registries passed exhaustive
verification each cycle. Oversized allocations issued zero writes/barriers;
active reader handles refused snapshot deletion. Every cycle recovered more
than 384 normally available blocks on a 512-block image within 512 calls.

The full workspace all-features gate passed 416 tests with no failures and
10 explicitly ignored qualification probes. Formatting, strict Clippy,
documentation, three checker fixtures and whitespace validation passed.

The targeted debug memory-backend run measured these maintenance-only totals:

| Scan budget | Calls | Records scanned | Blocks promoted | Reads / writes | Bytes read / written | Barriers |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 418 | 418 | 10,156 | 9,132 / 2,508 | 37,404,672 / 10,272,768 | 836 |
| 8 | 185 | 1,174 | 9,880 | 4,069 / 1,047 | 16,666,624 / 4,288,512 | 370 |

Each profile admitted all 24 cross-block writes and reached a minimum of 44
free blocks. Maintenance bitmap payload peak was 64 bytes, excluding all other
memory. Local maintenance elapsed times were about 0.92 and 0.42 seconds; these
are debug algorithm timings, not storage latency or a performance promise.
The smaller record budget preserved correctness while increasing calls and I/O.
No format change or hardware write was involved. Sustained aged workloads,
full process memory and native durability/resource gates stay explicit.

## 2026-09-14 - Bind full object inventories to counted descriptor manifests

Under the owner's delegated authority, ADR-086 chose a two-pass descriptor
manifest rather than retaining every key in memory. Counts, byte totals and
ordered SHA-512/256 descriptors bind each inspected object inventory. Export
refuses unknown inventory knowledge and poisons completion on source changes.
Verified scratch import enforces path, class, key and ordinal binding with one
upload slot, refusing success and further reader use after any discrepancy.
Earlier complete publications are explicitly partial restoration, not rollback.

The full all-features workspace gate passed 415 tests with zero failures and
10 explicitly ignored qualification probes. Formatting, strict Clippy,
documentation, three checker fixtures and whitespace validation passed.
The archive component's 45 tests passed. Inventories of 0, 1 and 70 entries
round-tripped every key and byte with pages of 1, 3 and 64 entries, one-byte
transfer buffers and 512-byte scratch chunks. Descriptor calls were exactly two
complete enumerations. Failure cases covered altered hashes/counts/byte sums,
duplicate keys, class/path mismatch, revoked grants, unverified replay,
uninspected sources, changing descriptors and admission/ordinal boundaries.

This is a full-preservation inventory component, not whole-job completion.
Cross-object namespace/data/reservation binding, AFS+ opaque storage mapping,
content-recovery loss reporting and native/older-system measurements retain
their queue gates. Small buffer fixtures establish hosted resource behavior;
they do not establish a working m68000 or bare-metal backup service.

## 2026-09-14 - Replay verified scratch archives with bounded upload slots

Accepted ADR-085 under delegated recommended-option authority. Added quota-bound
scratch capture, a privately retained Merkle root and checked replay. Capture
verifies the archive through chunk proofs before returning a replay capability.
Each replayed chunk is checked before exposing bytes; no caller-supplied root or
scratch mutation handle is exposed. Verified replay permits one staged value to
publish and release its upload slot before the next, while ordinary streaming
input retains its final-EOF gate. Reader identity and failure checks remain.

The integrated test restored 20 distinct keys with only root plus one upload
slot. Other tests compare an independently calculated Python SHA-256 root,
exercise odd trees and partial chunks, reject altered/locally resealed/reordered
bytes and proof hashes, enforce quotas and I/O failure handling, and replay an
actual temporary regular file. The complete workspace passed 410 tests with
zero failures and ten ignored tests; the archive consumer passed 40 tests.
Formatting, workspace Clippy, documentation checks, three checker fixtures and
whitespace validation passed.

The 3,584-byte fixture used 4,032 scratch bytes with 512-byte chunks and four
levels: capture plus verification/replay made 68 reads/8,896 read bytes and
21 writes/4,032 write bytes. With 4,096-byte chunks it used 4,128 scratch bytes,
one level, three reads/8,224 read bytes and two writes/4,128 write bytes. Maximum
read requests equaled one chunk in each case. These are small host resource
oracles, not sustained workload or whole-process RSS measurements. Full inventory
validation, segmented scratch beyond host seek limits, AFS+ metadata storage and
native lifecycle qualification remain owned queue work.

## 2026-09-14 - Carry opaque values through an authorized archive consumer

Accepted ADR-084 under delegated recommended-option authority. Added bounded
PAX descriptor/binary pairs with canonical ordinal and exact source-path, class,
key, encoding and size binding. Export reads through the captured backup reader,
checks exact length/EOF and poisons completion after any failure. Import opens a
staged destination upload only after pair validation. Its staged result can
publish only when that exact archive reader verifies the terminal digest and EOF;
a receipt from another reader cannot release it. This avoids activating a value
before later archive corruption is detected.

End-to-end tests cross independent source and destination providers using two-
and three-byte buffers, preserve unknown binary and empty values after live
source mutation, and check exact identities. Wrong bindings, source-length errors,
revocation, corrupted bytes, premature/foreign-reader publication and every
truncated archive prefix leave active metadata absent and release staging.
Descriptor tests cover required fields, versions, duplicates, limits and scalar
boundaries. The consumer dependency is optional; standalone framing/metadata
codecs do not require VFS. Updated the spec, queue and navigation.

Validation passed: 403 workspace tests, zero failures, ten ignored tests;
33 archive tests with the consumer and 28 standalone tests; formatting, workspace
Clippy, documentation checks, three checker fixtures and whitespace validation.
This qualifies an opaque-value transport component, not the whole backup job.
Complete inventory/object matching, large-inventory staging/spooling, AFS+ opaque
storage, generic metadata recovery and native resource/durability gates remain.

## 2026-09-14 - Stage opaque destination metadata before atomic publication

Accepted ADR-083 under delegated recommended-option authority. Added an optional
provider extension and non-cloneable upload handle bound to the original object
lease and grant. Host value admission defaults to disabled; each upload also
consumes a handle-budget unit. Sequential chunks remain in private staging until
exact-length finish. Staging errors permanently fail the upload; dropping,
revocation or premature finish releases provider staging without publishing.
The AFS+ mapping explicitly refuses the extension pending storage qualification.

Independent-provider tests cover exact unknown binary and empty values,
pre-finish invisibility, object-lease retention, budget recovery, invalid/excessive
requests, foreign/revoked grants, partial staging-write faults and old/complete-new
publication-error outcomes. In-backend probes verify held admission for every
phase. Remounted AFS+ refusal issues zero writes and flushes. Updated API,
qualification and queue ownership; accepted ADR bodies are unchanged except for
amendment relations.

Validation passed: 398 workspace tests, zero failures, ten ignored tests;
43 VFS tests including doctests; formatting, workspace Clippy, documentation
checks, three checker fixtures and whitespace validation. These are host
memory-provider tests. Durable AFS+ metadata storage, staging cleanup recovery,
archive binding and complete constrained/native qualification remain queue work.

## 2026-09-14 - Stream opaque captured attributes and security metadata

Added checked metadata descriptor pages and caller-buffer value reads to the
snapshot service and consumer facade. Attribute/security channels stay distinct;
keys and encoding identifiers remain exact and unknown binary values are not
interpreted. Requests and returned descriptors have explicit size, ordering and
progress checks. Missing implementations refuse transport. Operation permits
cover provider execution, and revoked calls preserve output buffers.

Independent-provider tests enumerate one entry at a time and read in two-byte
chunks, including empty, UTF-8 and unknown binary values. Captured data survives
live inventory changes. Malformed requests/results, cursor nonprogress, invalid
read counts, revocation and unsupported remounted AFS+ behavior are covered.
The API documentation records additive Rust compatibility and separate native
and destination requirements.

Review also found that docs/30 described canonical ACL evaluation as settled,
contradicting Proposed ADR-031 and unresolved Q5. Clarified its candidate scope,
retained the candidate design for evaluation, and linked the preservation and
qualification owners. This does not accept rich ACL evaluation semantics.

Validation passed: 394 workspace tests, zero failures, ten ignored tests;
39 VFS tests including doctests; formatting, workspace Clippy, documentation
checks, three checker fixtures and whitespace validation. Source transport is
not complete backup/restore: archive binding, lossless destination installation,
AFS+ metadata storage and actual native provider support remain queue work.

## 2026-09-14 - Inspect captured metadata knowledge through backup authority

Added a filesystem-neutral fixed-size inventory-knowledge operation to the
trusted provider, service and consumer facade. The conservative default validates
the captured object through stat and reports uninspected attribute/security
inventories. A complete provider may report empty or present from its captured
view. Admission holds the original reader grant throughout inspection, with
wrong-service and revocation rejection before backend access. Documented additive
Rust compatibility and explicit C/native qualification boundaries.

The independent provider test preserves captured inventory knowledge after live
changes. Default-provider tests verify held authority, missing-object errors,
revocation and fresh-grant isolation. The AFS+ remount fixture verifies
uninspected results with zero writes/flushes and a clean checker. Validation
passed: 392 workspace tests, zero failures, ten ignored tests; all 37 VFS tests
including doctests; formatting, workspace Clippy, documentation checks, three
checker fixtures and whitespace validation. Actual attribute/security enumeration
and transport remain queue work; this result does not certify full preservation.

## 2026-09-14 - Preserve exact object metadata and inventory knowledge

Accepted ADR-082 under the owner's delegated recommended-option authority.
Inspection found no attribute/security enumeration in the authorized snapshot
API; treating that limitation as an empty inventory would misstate preservation.
The versioned object payload therefore records empty, present or uninspected
inventories explicitly, alongside exact protection, three timestamps, path and
kind. Added a bounded borrowing codec with required-field, numeric, namespace,
duplicate, truncation and inventory-state regressions. Updated the specification,
Q11, queue dependency and documentation indexes. The accepted ADR-076 body is
unchanged; its amendment relation links the new decision.

Validation passed: 390 workspace tests, zero failures, ten ignored tests;
28 archive tests; formatting, workspace Clippy, documentation checks, three
checker fixtures and whitespace validation. No filesystem disk record or ABI
changed. Complete inventory transport, archive-wide matching and authorized
restoration remain implementation work; codec success cannot close those gates.

## 2026-09-14 - Bind local PAX records to streamed ordinary members

Integrated effective-field admission with envelope reading. Local records apply
exactly once; stacked or dangling blocks fail permanently. Metadata bytes are
bounded before buffer allocation and effective size selects framing before
payload access. Caller buffers stream contents, early advancement reports busy,
and a completion receipt cannot bypass local-record failure. Added exact
one-member override and payload oracles, admission/poisoning checks and every
truncated archive prefix. Full preservation and actual restoration remain
separate queue gates.

Validation passed: 387 workspace tests, zero failures, ten ignored tests;
25 archive tests; formatting and workspace Clippy; documentation checker,
three checker fixtures and whitespace validation. No filesystem record or
native ABI changed, and no hardware qualification is inferred.

## 2026-09-14 - Validate effective PAX member fields before restoration

Added borrowed ordinary-member resolution with explicit PAX record admission,
resolved namespace and hard-link checks, strict numeric overrides, unsupported
metadata refusal and exact signed nanosecond conversion. Reused the envelope's
canonical-path rule and extracted record preflight without serializing another
payload buffer. Added four regression tests covering override escapes, links,
resource admission and timestamp/numeric boundaries. The contract retains
numeric and textual identities separately; host account mapping and complete
preservation semantics belong to the consumer.

Validation: all 384 workspace tests passed, zero failed and ten were ignored.
The archive crate passed 22 tests. Formatting, workspace Clippy, documentation
checking, three checker fixtures and whitespace validation passed. This is host
admission evidence, not native or older-hardware qualification. Full sparse,
attribute/security transport and integrated archive restoration remain queue
work; no milestone was closed by these helper tests.

## 2026-09-14 - Bind archive integrity and termination to an independently checked envelope

Accepted [ADR-080](adr/ADR-080-pax-completion-envelope.md) under delegated
recommended-option authority, selecting SHA-512/256 and 128-bit byte accounting
after the NIST message-domain review exposed SHA-256's smaller bit-length limit.
RustCrypto sha2 0.11.0 supplies the implementation; seven packages were added to
the lockfile. The digest is unkeyed and makes no authentication claim.

Independent recovery rejected the terminal global-header prototype: Python
3.9.6 tarfile required a subsequent member. [ADR-081](adr/ADR-081-ordinary-pax-completion-member.md)
superseded it with version 2, a regular terminal control file and separated
`files`/auxiliary namespaces. The accepted earlier decision remains intact as
history. Source names resembling control names can live under the source subtree.
The bundled Python/LibreSSL lacked SHA-512/256; the gate uses existing OpenSSL
3.6.4 independently, without adding a runtime dependency on that executable.

Eighteen archive-crate tests passed on both default and compact software hash
backends. They cover delayed receipts, exact counts/digests, truncated prefixes,
changed body/control bytes, unsupported versions, duplicate/missing completion,
path/control collisions, limits and failed output/flush. Review extended the
raw-name guard to local PAX header names, and its regression cases pass on both
backends. Independent OpenSSL
hashes and Python/bsdtar ordinary recovery passed for both backends, including a
changed-payload negative oracle. Formatting, Clippy, documentation checks,
three checker fixtures and whitespace validation passed. The full workspace
suite passed 380 tests, with zero failures and ten ignored.

Profile interpretation, opaque metadata transport, actual authorized snapshot
and restore consumers, performance/peak RAM and native/older-system execution
remain queue requirements. The receipt establishes stream integrity and actual
termination; it does not establish full preservation or authenticated provenance.

## 2026-09-14 - Add streamed tar framing and independent recovery checks

Added strict ustar headers and a caller-buffer streaming reader/writer.
Member counts, effective payload sizes and trailing zero blocks have explicit
limits. The profile must validate PAX overrides before body selection. Invalid
headers/padding, truncated bodies/end markers and uncertain I/O poison the
reader/writer; successful finish establishes framing, with profile completion
and destination durability as separate requirements.

Eleven crate tests passed, including six framing tests: independent Python
fixture decoding, every truncated Rust-archive prefix, valid-checksum malformed
headers, long UTF-8 names, limit/sequencing checks, 64-bit override arithmetic,
and partial write/padding/finish/flush/read failures. The independent host gate
passed with Python tarfile and bsdtar 3.5.3 / libarchive 3.7.4, exact ordinary-file
payloads, Python header metadata, reproducible fixture bytes and a changed-byte
negative oracle. It extracted only to a private temporary buffer/file.

Formatting, Clippy, documentation checks, three checker fixtures and whitespace
validation passed. The full workspace suite passed 373 tests, with zero failures
and ten ignored. Sparse/full-metadata interoperability, the preservation profile,
integrity/completion, real authorized consumers and constrained/native runtime
remain integration requirements; ordinary tar success does not close them.

## 2026-09-14 - Add bounded PAX record parsing for the archive consumer

Added the dependency-free host archive crate and strict unique-key UTF-8 PAX
record codec. Explicit byte, record, keyword and value limits bound admission;
decoding borrows payload strings and encoding preflights total output bytes.
Duplicate keys, malformed lengths, invalid UTF-8 and NUL are rejected. Empty
values are retained for profile interpretation; general override semantics
and archive completion belong to the next archive layer.

Research checked the Oracle Solaris pax manual's extended-header section:
[primary source](https://docs.oracle.com/cd/E88353_01/html/E37839/pax-1.html).
The current Open Group endpoint returned HTTP 403, and GNU direct-page fetches
failed, so those fetches supply no new verification. Python 3.9.6 tarfile
independently generated the retained UTF-8/newline/equal-sign timestamp fixture;
exact encoder and decoder equality passed. No private project details entered
public queries.

Five tests passed, covering the golden record block, decimal-width boundaries,
every nonempty truncated single-record prefix, exact error classes, duplicate
keys, limits in both directions, and deterministic arbitrary input. Formatting,
Clippy, documentation checks, three checker fixtures and whitespace validation
passed. The full workspace suite passed 367 tests, with zero failures and ten
ignored.

The archive codec does not prove a complete backup. Framing, the versioned
preservation profile, integrity/completion, attribute/security transport,
independent archive recovery and real snapshot/restore consumers remain explicit
integration requirements in the queue and archive qualification plan.

## 2026-09-14 - Bound shrink working sets and retirements

The host Rust bounded resize entry point admits affected tail records and
retired allocated blocks. A replaced partial written tail counts once;
reservations beyond EOF count toward retirement, and holes do not. Local tree
edits preserve unrelated mappings and may retain an empty tree. Shared data
and captured views use the existing reference/lifetime retirement machinery.
The restore provider admits 64 records and 64 retired blocks per resize,
refusing excessive work before writes without silently splitting the operation.

Three targeted tests passed. A partial-tail edit among 301 fragmented records
used 99 device reads and preserved snapshots through shrink/regrowth/remount.
Record/block refusals issued zero writes. Empty-tree truncation preserved its
shared peer. The publication oracle passed 1171 states (1167 old, four new),
checking exact live records/bytes, shared-peer content and captured accounting.

Formatting, Clippy, documentation checks, three checker fixtures and whitespace
validation passed. The full workspace suite passed 362 tests, with zero
failures and ten ignored. Arbitrary large atomic truncation requires persistent
bounded cleanup and its own crash
oracles. Total RAM and native/constrained runtime need separate qualification.
Those requirements remain in the complete queue.

## 2026-09-14 - Grow sparse files without enumerating mappings

Extent-tree growth retains the exact root and allocation accounting while
publishing size, times and content generation through the existing COW tail.
The fixed-size direct path retains its promotion rules. A 300-record file
required 51 device reads to grow to the maximum logical size, preserving its
captured view and final-block zero reads through remount.

The maximum-size regression exposed an overflowing read block-end addition.
Saturating the boundary before EOF clipping fixes it; restoring the former
expression reproduced the overflow as a negative control. Growth publication
passed 346 modeled crash states (342 old, four new), checking exact object
records, complete bytes, reservation accounting and snapshot contents.

The targeted tests, formatting, Clippy, documentation checks, three checker
fixtures and whitespace validation passed. Full workspace regression passed
359 tests with zero failures and ten ignored. Bounded shrinking, total RAM and native/constrained runtime
gates remain separate requirements in the queue.

## 2026-09-14 - Bound atomic writes for restore consumers

Added the host Rust bounded-write entry point, using local extent windows and
the generalized changed-key publication helper. It admits touched blocks and
local records, preserves existing tree representation, and shares the data,
reference and checkpoint publication machinery. The restore provider uses
64-block and 64-record limits; larger transfers require smaller independently
durable requests. Ordinary unrestricted writes retain their admission behavior.
No disk encoding, C ABI or native capability advertisement changed.

Four targeted tests passed: a 130-record fragmented byte oracle with snapshot
and remount checks; block/record refusal before writes; shared-peer isolation;
empty/direct promotion; eligible private tree overwrites with unchanged physical
mappings; and 642 publication crash states (638 old, four new) preserving
captured zeros. The full workspace suite passed 357 tests with zero failures
and ten ignored. Formatting, Clippy, documentation checks, three checker
fixtures and whitespace validation passed.

Total peak RAM, allocator/retention bounds, truncation memory, sustained
near-full operation and native/constrained runtime qualification need further
work. The local-vector and data-buffer limits do not close those queue gates.

## 2026-09-14 - Bound reservation edits in fragmented files

The host Rust reservation entry point accepts explicit touched-block and
extent-record budgets. Local key pages and changed-key COW edits avoid loading
and rebuilding unrelated extent mappings. The checked restore provider uses
64 records plus its existing host-granted byte bound. Refusal occurs before
device writes; callers can retry smaller separately durable requests. The
convenience core API preserves unrestricted request admission. No disk, C ABI,
or native advertisement changed.

The full workspace suite passed 353 tests, with zero failures and ten ignored.
Formatting, warning-denying Clippy, documentation checks, three checker fixtures
and whitespace validation passed.

Three targeted tests passed. A one-block reservation among 600 fragmented
records used 98 device reads, 12 metadata writes, 61440 written bytes and two
flushes. Boundary/no-op and input/result-budget tests preserved exact records
and issued zero writes on refusal. The multi-node publication oracle passed
4300 crash states: 4296 old and four new, with exact live mappings and captured
allocation ranges. Full graph checking covered selectable checkpoints.

The design bounds local extent vectors; total peak RAM and before/after process
memory measurements are unqualified. Ordinary writes/truncation and allocator
working sets need their own bounds. Native and older-system execution remains
UNVERIFIED. These gates stay in the complete queue.

## 2026-09-14 — Cross-read initialized reservations through the portable C reader

Added deterministic baseline-format images before and after private reservation
initialization, plus a fallback image whose newer checkpoint is invalidated.
The fallback keeps the changed physical payload while the older mapping must
return logical zeros. A separate C probe verifies all 16,384 logical bytes in
257-byte chunks. Strict C99 and ASan/UBSan runs passed the three images, with
75/108/75 callback reads; negative controls rejected initialized data when zeros
were expected.

The probe uses 6 KiB caller scratch for directory traversal and 4 KiB for object
lookup/file reads; a smaller lookup workspace is refused before I/O. These
bounds exclude stack, stdio and process memory. The configured/default m68k
compiler path was unavailable, so the optional compile gate reported SKIP;
m68000 runtime and total-resource qualification remain unverified. The fixture
has no snapshot-registry feature, keeping that portable consumer gate separate.

Validation: 350 workspace/all-features tests passed, zero failed and 10 were
ignored. Strict and sanitized C reads passed six image cases plus two negative
controls. Formatting, Clippy, shell syntax, documentation, three checker
fixtures and whitespace checks passed. The m68000 compile gate explicitly skipped.


## 2026-09-14 — Initialize private unwritten reservations without replacement data allocation

Under the owner's delegated design authority, ADR-079 selects initialization
of proven-private unwritten blocks followed by COW publication of their written
mappings. The baseline regression reproduced replacement allocation (physical
block 25 became block 33). The implementation preserves eligible physical
addresses and lifetime birth, checks the live reference tree, and keeps shared
or stale-marked reservations on fresh storage. A mixed write copies written
portions while initializing eligible reservations. Commit counters distinguish
initialization from the opted-in written-data in-place policy.

Six targeted tests passed: physical reuse in mixed writes, shared/stale-marker
fallback, low-space progress, 362 crash states (358 old and 4 complete new),
false-private shared-reference refusal, and completed-write/three-barrier I/O
failures. The low-space fixture initialized 32 data blocks with 24 blocks
available for new allocation, writing 5 metadata blocks and 163,840 bytes with
3 flushes while the snapshot retained zeros. Both checkpoints passed ownership
checking. These are host fixtures; portable C cross-reading, bounded fragmented
layout traversal, sustained pressure and older-target resource evidence remain
explicit work.
Also corrected a stale runtime-only qualification sentence in files/extents;
ADR-065 and the implemented persistent policy remain the authority.

Validation: the workspace/all-features suite passed 350 tests, zero failures
and 10 ignored tests. Formatting, Clippy with warnings denied, documentation,
three checker fixtures and whitespace checks passed.


## 2026-09-14 — Add checked destination reservation restoration

Added exact reservation operations to the restore provider/service/client. The
host configures a per-operation byte cap; its default is disabled. Checked
operations reject invalid/excessive requests before provider calls and retain
the original handle grant through admission. Providers explicitly refuse missing
support. AFS+ requires block alignment, preserves written data and logical size,
and supports the final rounded block ending at 2^64.

Extended the independent and AFS+ restore job with reservations in a hole and
beyond EOF. Added refusal, budget, revocation, final-address and remount checks;
the existing zero-write fixture covers alignment refusal and the in-backend
probe covers reservation admission. Added exact old/new object-record crash
oracles for preallocation while retaining an earlier snapshot, checking both
checkpoint slots and captured bytes/allocation.
The targeted crash run enumerated 626 states: 622 old and 4 complete new states.

Review identified two remaining qualification requirements: the existing core
loads full extent layouts for preallocation, and later COW writes allocate new
data blocks. The API cap alone cannot establish constrained-memory behavior or
near-full consumption of reservations. Recorded these explicitly in the queue
and test plan; they are implementation work before a complete reservation
capacity promise, not a reason to downgrade the full-preservation contract.

Validation: full workspace/all-features suite passed 344 tests, with zero
failures and 10 ignored tests. Formatting, Clippy with warnings denied,
documentation checks, three checker fixtures and whitespace checks passed.


## 2026-09-14 — Delegate decisions while the owner is away

The owner authorized choosing recommended design options without further
questions until the full queue is handled. Decisions must retain their rationale
and alternatives; public communication and hardware-write restrictions continue
to apply. Completion must include correctness and platform-interest review and
older-system degraded operation under resource constraints. Added those gates
to the queue so host success cannot substitute for constrained/native evidence.


## 2026-09-14 — Enumerate captured allocation and decide preservation modes

The owner chose separate full-preservation and content-recovery modes after
reviewing preallocation's capacity guarantee and restoration costs. ADR-078
records preservation/refusal for the full mode and explicit reservation-loss
reporting for content recovery. The precise PAX profile and destination
reservation qualification remain work.

Added bounded snapshot allocation pages, exposing semantic byte ranges and
unwritten state, including rounded tails and reservations beyond EOF. The VFS
backup facade checks authority and page bounds; unsupported providers explicitly
refuse. The extent reader checks a predecessor at page boundaries. No physical
addresses or sharing layout enter the consumer interface, and no disk or C ABI
changes were made.

The core fixture enumerates 140 fragmented written/unwritten records plus a
reservation at a 1 TiB offset after live truncation and remount. Page sizes 1,
7 and 64 preserve captured coverage with at most 32 device reads per page and
zero writes/flushes. Direct, empty, rounded-tail, bad-limit, directory,
out-of-range and stale-handle cases are covered. A valid-CRC overlapping extent
fixture exercises the cross-page check. The same VFS consumer reconstructs bytes
from allocation coverage and ordinary reads on independent and AFS+ providers;
revocation and in-backend authority exclusion have regression coverage.

Review found that the final rounded allocation can end at 2^64. Changed byte
conversion to multiply offset and length separately, then added an actual
preallocation/snapshot/remount regression at that boundary. Stopped the first
full-suite run to fix this before validation; its interrupted result is not a pass.

For the next archive unit, an exploratory GNU sparse 1.0 fixture recovered
12,288 exact logical bytes, including gaps and a trailing hole, through Python
3.9.6 tarfile and bsdtar 3.5.3/libarchive 3.7.4. The layout followed the
[GNU tar sparse 1.0 specification](https://www.gnu.org/software/tar/manual/html_node/PAX-1.html),
checked on 2026-09-14. This temporary-fixture experiment is an encoding input,
not archive, reservation or metadata-preservation qualification.

Validation: the final workspace/all-features run passed 341 tests with zero
failures and 10 ignored tests. Formatting, Clippy with warnings denied,
documentation checks, all three checker fixtures and whitespace checks passed.


## 2026-09-14 — Enforce separate destination restore grants

Implemented the owner's ADR-077 choice in a filesystem-neutral checked restore
service. Backup and restore use separate opaque public grant types and share
private admission machinery. Every operation checks the original handle grant;
revocation drains admitted calls. Linking checks both grants and avoids taking
the same read lock twice. Handle limits are reserved before creation, shared by
clones, and released after the provider handle drops even following revocation.

The AFS+ provider owns a writable volume and a selected empty directory. It
creates fresh entries without importing existing outside objects. One consumer
runs on an independent provider and AFS+, restoring nested names, file bytes,
sparse gaps, hard links, protection and three timestamps. The AFS+ test remounts,
checks exact values and sparse allocation, preserves an outside sentinel and
an older snapshot, and runs the exhaustive checker. Denial, malformed names,
metadata refusal, capacity cleanup and concurrent revocation have regression
coverage. In-backend probes check held write/link permits; compile-fail cases
reject cross-role grants and consumer backend access.

The API and qualification docs define the source-only compatibility boundary,
create-only destination and partial-work semantics. Q11 explicitly owns merge,
overwrite, resume and completion publication. This library test does not qualify
native host authentication, IPC, security-container/attribute transport or a
complete PAX archive. Those remain in the queue.

Validation: full workspace/all-features suite passed with 337 tests, zero
failures and 10 ignored tests. Formatting, Clippy with warnings denied,
documentation checks, all three checker fixtures and whitespace checks passed.


## 2026-09-14 — Restore existing metadata through the common COW tail

Added protection mutation and exact restoration of protection plus creation,
modification and change timestamps. Other object fields remain destination-owned.
The existing file data-policy setter shares the same metadata transaction helper;
it also rejects invalid timestamps before publication. Invalid caller metadata
has a distinct core error mapped to the existing VFS invalid-argument category.
No disk encoding, C ABI, feature identity or protection interpretation changed.

The timestamp audit found object and intent-log encoders accepting nanoseconds
that their readers reject. Shared validation closes that mismatch. Mutation
entry points validate supplied times before staging work, with a zero-write
namespace/log-window regression and encoder tests across all log operation kinds.
Valid encoded records retain their existing representation.

Tests cover files, directories and root, signed timestamp extremes, nanosecond
bounds, sparse shared storage, hard links, policy flags, no-ops, hidden targets,
read-only modes, open windows and uncertain publication. The crash oracle checked
346 states (342 old tuples, four new), preserving file bytes and snapshot metadata
with both-slot full checks. The sparse/shared fixture issued 28 KiB without
snapshots and 32 KiB with snapshots, four versus five metadata blocks and two
flushes in each case; neither wrote file data. These are fixture costs, not a
shipping resource profile. The AFS+ backup fixture changes live protection bits
before verifying the unchanged captured metadata and bytes.

The owner selected separate destination-scoped restore grants; ADR-077 records
that decision. The checked restore facade, native host authorization and full
PAX archive/restore preservation remain separate follow-up gates.

Validation: the final workspace all-features suite passed 326 tests, zero failed
and ten were ignored. Formatting, workspace Clippy with warnings denied,
documentation, all three checker fixtures and whitespace validation passed.
Portable C source already enforces the timestamp bound; no new native or
portable-C restore qualification is claimed.

## 2026-09-14 — Enforce revocable authority on the backup interface

Added the filesystem-neutral backup service and consumer facade under ADR-075.
The host owns grant issuance and privileged backend access. Every consumer call
checks its service-bound grant and retains a shared permit throughout backend
execution; revocation takes the exclusive lock and returns after admitted work.
Readers bind to their original grant. Final close releases the provider lease
before its explicit reader budget, even when authority has been revoked.

The same paged/streaming consumer runs against an independent provider and AFS+.
AFS+ captured names, metadata and bytes survive live write, deletion and remount;
old-service authority cannot access the new service. The independent provider
also changes live protection metadata. Tests distinguish denied calls from
backend access, preserve denied-read buffers, exercise duplication, busy deletion,
reader exhaustion, new grants and concurrent revocation. A compile-fail fixture
checks that the consumer facade has no unchecked backend hook.

Review corrected a missing-object error mapping: a captured directory entry
whose object is absent reports corruption. A valid-CRC damaged-image fixture
distinguishes that broken reference from an ordinary absent-object lookup,
checks zero source writes/flushes and requires independent checker rejection.

The Rust interface is additive and does not advertise C/IPC or OS support.
AFS+ protection mutation, exact restoration, sparse/attribute/security transport
and real host authentication remain owned follow-up work. A test collector is
not a complete archive/restore consumer.

Archive research on 2026-09-14 compared PAX with preservation metadata against
a dedicated container; the owner selected PAX and ADR-076 records the
direction, complete-versus-partial distinction and preservation gates. The
[GNU tar manual](https://www.gnu.org/software/tar/manual/tar.html) documents
PAX-based xattr/ACL storage and GNU sparse extensions. The
[tar 0.4.46 builder](https://docs.rs/tar/0.4.46/tar/struct.Builder.html) exposes
streamed entry and PAX-extension writing; host-path convenience methods do not
replace snapshot enumeration. Format choice still requires exact preservation,
unsupported-metadata refusal and independent extraction tests.

Validation: the final workspace all-features suite passed 319 tests with zero
failures and ten ignored tests. Formatting, workspace Clippy with warnings
denied, documentation checks, three checker fixtures and whitespace validation
passed. These are host-side results, not native authentication or full restore
qualification.

## 2026-09-14 — Configure snapshot limits before writable mount recovery

Added an explicit `mount_with_snapshot_limits` core entry point. It validates
runtime budgets and existing view admission before recovery can write, while
baseline mount APIs retain their feature mask. Snapshot orchestration fixtures
now use the public mount path, including their both-slot crash and churn checks.
No disk encoding or filesystem API v2 ABI changed.

The owner selected revocable host backup authority after comparing live-file
permission checks. ADR-075 records per-operation checks, historical access
independent of later live permissions, cleanup after revocation and the host
admission/conformance gates. Ordinary-user historical browsing remains open.

New mount fixtures cover all four modes, rejected budgets with pending durable
replay, unknown feature bits and corrupt selected roots with write/flush traps.
The one-to-90-view fixture measured nine versus ten mount reads before readers
or exhaustive checking, then verified every captured view. Shipping resource
profiles and host authorization remain independent requirements.
Snapshot-aware recovery preserved acknowledged live bytes and captured bytes
across 346 modeled interrupted states, with both-slot ownership checks after
recovery. The model covers full-write subsets and representative tears.

Validation: 311 workspace all-features tests passed, zero failed and ten were
ignored. Formatting, workspace Clippy with warnings denied, documentation,
three checker fixtures and whitespace validation passed. These are host-side
results; adapter authorization and native qualification remain separate gates.

## 2026-09-14 — Protect both checkpoint generations during reclamation

Implemented ADR-074's R <= P promotion boundary in all reclaim tiers, where P
is the oldest structurally valid checkpoint generation. Protected heads stop
consumption without moving their cursor. COW eligibility also consults the
previous snapshot registry; missing roots or unreadable older registry state
disable the optional optimization. Full-COW requests avoid that optional lookup. Runtime statistics report checkpoint blocking.

The post-deletion data-write oracle covered 352 modeled states: 347 retained
a registered view in a valid slot and preserved its exact bytes; five replaced
that registration. Both-slot full ownership checks accompany snapshot remounts,
create/delete cuts and churn. On the 512-block, 160-cycle fixture, minimum free
space was 472 blocks versus 475 under the former boundary; maximum ledger-retired
count was ten and final release transferred five protected blocks. TxAllocator
inline size on aarch64 rose from 936 to 960 bytes; heap state is additional.

Reclaim qualification exposed older tests' one-generation assumptions. The
fixtures require a protected maintenance generation before reuse and a two-root
steady state. Existing low-space recovery passed. The tiny-volume admission
oracle preserves its actual committed prefix rather than promising the same
capacity after adding retention. The original 64-block accounting workload
completes with two explicit maintenance publications: four additional flushes
and 64 KiB issued by those two steps, with final allocator bitmap residency of
four bytes. These are fixture costs, not a shipping resource profile. Previous-checkpoint preservation does not
change the ordinary non-snapshot opt-in in-place byte-failure contract.

Validation also exposed a documentation-checker race with Cargo deleting target
subdirectories. Discovery prunes excluded trees before traversal and propagates
errors in included source directories; temporary fixtures cover both behaviors
and deterministic output. The command-line interface and read-only default are
unchanged.

Validation: the full workspace all-features suite passed 306 tests with zero
failures and ten ignored tests. Workspace formatting, Clippy with warnings
denied, documentation checks, all three documentation-checker fixtures and
whitespace validation passed. These are host-side results; native hardware
and portable-C snapshot qualification remain separate gates.

## 2026-09-14 — Verify snapshot ownership and select stronger recovery retention

Added exhaustive registry/ledger and captured-namespace validation to the core
checker and enabled read-only snapshot inspection in afsplus-check. New-image
formatting has an explicit snapshot option; writable mount negotiation stays
closed. The checker reconciles lifetimes, namespace/housekeeping/quarantine,
metadata birth headers and bitmap accounting. Historical sharing does not use
the live reference tree. Physical blocks shared between views count once.

The full workspace all-features gate passed: 302 tests, zero failures and ten
explicitly ignored qualification tests. Formatting, all-target/all-feature
Clippy with warnings denied, documentation and whitespace checks passed.

The targeted gate passed ten core tests and one checker-facing test, including
resealed corruption cases and ownership sweeps across the existing create/delete
cuts and 160 churn cycles. A real checker gap was fixed: a disconnected directory
cycle could satisfy all incoming-link counts. Both live and historical graph
validation require reachability from namespace roots.

A post-deletion write experiment also clarified ADR-036: older shadow registry
storage could be reclaimed before its checkpoint slot was replaced. The newest
selected state stayed protected, so an additional COW check alone was not a
complete remedy. The owner explicitly selected stronger previous-checkpoint
protection. ADR-074 records the generation boundary, snapshot COW obligation,
pre-release compatibility limits and required low-space/crash qualification.
The handoff prioritizes that implementation before writable snapshot exposure.


## 2026-09-14 — Integrate persistent snapshot lifecycle into Volume

Implemented registry creation/deletion, mount-scoped reader leases, captured
namespace reads and resumable directory cursors under ADR-071/072/073. Creation
closes the intent window; registered snapshots override in-place writes. The
common transaction tail seals lifetimes, publishes both roots and transfers
eligible storage into ordinary quarantine. Deletion uses emergency headroom and
refuses active handles. Reclaim reports scan and promotion separately.

The full workspace all-features gate passed: 299 tests, zero failures and ten
explicitly ignored qualification tests. Formatting, all-target/all-feature
Clippy with warnings denied, documentation and whitespace checks passed.

Eight targeted memory-backend tests passed. Create cuts covered 197 modeled
states (193 absent, 4 present); delete cuts covered 115 (4 absent, 111 present),
checking membership and exact bytes. A 512-block fixture held an old view through
160 churn cycles with minimum free space 475 blocks and maximum retired count
12; releasing the view released its five protected blocks. Additional tests
covered private/shared COW, log-window capture and replay, cursor/handle remount
identity, near-full admission/deletion, uncertain final flush and exhausted IDs
without publishing a pending window.

Formatting and mounting snapshot images remain test-only. Ordinary mounts reject
the feature until full ownership checking and supported configuration are
qualified. These tests do not prove host authorization, backup restoration,
portable-C parity, shipping budgets or native durability. Q5 explicitly owns
snapshot management and historical-read access/revocation policy; the handoff
moves to checker integration and the real consumer.


## 2026-09-14 - Bind lifetime accounting to allocator transactions

Added namespace capture, housekeeping and lifetime-seal phases to the transaction
allocator. Births exclude released and log-sacrificed allocations. Last-live
retirements stay in the ledger; eligible transfers enter the ordinary queue only
with a successfully prepared lifetime mutation. Failed sealing poisons the
transaction, unsealed finalization fails, and caller mutations after sealing fail.
The finalized result carries replacement roots and writes into the common commit
publication path. Volume orchestration and feature activation remain later work.

The aarch64 inline allocator footprint measured 1,216 bytes with embedded
snapshot state and 936 bytes after making that state a lazy allocation.
Feature-absent transactions carry only the optional pointer. Enabled transactions
still allocate the accounting object; its heap and allocator overhead are
additional, so this is not a claim of lower total snapshot RAM or faster I/O.

Four tests use real bitmap, descriptor, allocation-root and reclaim state. They
verify housekeeping exclusion, exact replay births, guards and the delay between
ledger release and reallocation. The transfer crash matrix covers 115 modeled
states with exact ledger/quarantine ownership alternatives; deleting its metadata
barrier is a required failing control. The specification now states the sealing
and finalization obligations explicitly under ADR-071.

## 2026-09-14 - Prepare transactional lifetime edits

Added the lifetime mutation builder on the shared COW tree engine. It fetches
affected committed records and neighbors before editing, preserves birth across
splits, coalesces final equal lifetimes and adjusts retained totals with checked
arithmetic. Requested transfers are matched against their exact committed records
and the complete paged registry. Record and registry work budgets fail closed.
The result includes staged tree writes and runs requiring atomic quarantine.

Six tests compare disk-tree state with a per-block oracle, preserve old COW nodes,
exercise second-page snapshot protection and require invalid preparation to stop
before allocator calls. A 600-record sparse tree loads three lifetime records
for one retirement. This unit leaves the Volume commit hooks, real bitmap
quarantine, snapshot handles and integrated crash tests for the following work.

## 2026-09-14 - Read snapshot trees with bounded contextual checks

Added typed registry and lifetime readers on the shared key-page path. Registry
access validates control state and ID relationships. Lifetime pages check their
predecessor and lookahead so overlaps or missing coalescing cannot hide across
batch edges. Tests preserve sparse IDs, simulate cursor persistence/deletion
and wrap, inject semantic corruption and verify read counts with zero writes.

Added a constant-memory allocator-pool envelope for reserved-range exclusion.
Its output matches the existing enumerated placement across small geometries
and computes at the maximum region count without allocating a pool vector.
These are reader prerequisites; bitmap/log ownership and atomic lifetime edits
still belong to integrated snapshot transactions. No mount feature was enabled.

## 2026-09-14 - Bind snapshot roots in the checkpoint codec

Added ADR-073 before extending checkpoint encoding. Feature-absent checkpoints
retain their 96-byte payload; the snapshot extension appends two roots in a
112-byte payload. Selection rejects feature/payload mismatches without falling
back to older namespace state. Snapshot mounts stay unsupported through this
codec unit.

Independent full-block fixtures fix both encodings, with a separate bitwise
CRC32C construction. Tests cover lengths, root bounds, valid-CRC mismatches,
legacy byte stability and short encoder buffers. The short-buffer test also
removed an existing size-subtraction panic in checkpoint encoding. Updated the
C refusal fixture to carry extended checkpoint headers as well as the feature.

Reconciled the disk-layout draft's stale open allocation-architecture language
with the accepted ADR-067; exact wire freeze remains a separate gate.

Validation passed: 276 workspace tests, 10 ignored; formatting, Clippy with
warnings denied, documentation, whitespace and independent fixture reproduction.
The portable-C reader/writer, sanitizer and static-analysis gate passed; its
optional m68k and CMake checks were skipped for unavailable configured tools.


## 2026-09-14 - Add bounded key pages for persistent maintenance cursors

Added a shared-tree key-page reader for the snapshot ledger's durable cursor.
It seeks an inclusive key, stops at the caller's record budget and validates
visited subtree counts, generations, ranges and cycles. A physical key cursor
survives deletion of earlier records; wrapping explicitly revisits insertions
behind the cursor.

Three tests use a three-level, 1,024-record fixture and independent sorted-key
oracles. Traced reads equal reported reads and obey the fixture's bounded-page
budget. A mutation between reads deletes earlier records and inserts a new
key behind the cursor; successor enumeration and wrap preserve the expected
sets. This is a shared-core prerequisite; persistent cursor commit/recovery
belongs to the upcoming snapshot integration.

Validation passed: 274 workspace tests, 10 ignored; formatting, Clippy with
warnings denied, documentation and whitespace checks.


## 2026-09-13 - Add persistent snapshot record codecs

Added ADR-072 and the snapshot-record specification before codec implementation.
Registry and lifetime values use fixed-size independent byte fields, separate
AFST kinds and a negotiated incompatible identity. Validation covers reserved
bytes, contextual generation/range bounds, ID exhaustion and lifetime endpoints.
Conformance tests construct full checksummed leaf images and test corruption.

The core supported-feature mask deliberately excludes snapshot ownership until
integration. A named Rust mount/checker regression and an independent portable-C
probe require feature rejection. Checkpoint root binding, lifetime maintenance,
reader handles and crash-safe publication are the next integration work.

Validation passed: 271 workspace tests, 10 ignored; formatting, Clippy with
warnings denied, documentation and whitespace checks. The portable-C gate
passed its reader/writer, sanitizer and static-analysis checks. Its optional
m68k compiler and CMake checks were skipped because the configured tools were
unavailable; those runs provide no m68k compilation or CMake-package evidence.


## 2026-09-13 - Accept the integrated lifetime experiment and handle rule

John approved the lifetime-ledger prototype and selected busy-on-active-reader
snapshot deletion. ADR-071 records both decisions and amends ADR-070. The
proposal, Q4 and handoff link that decision. The follow-up wire record and
integrated resource/crash evidence precede acceptance of persistent support.
Documentation and whitespace checks passed.


## 2026-09-13 - Add checkpoint extraction with explicit loss reporting

Implemented `afsplus-extract` for Q11. It opens the source with an OS read-only
descriptor, mounts with NO_CHANGES, and exports readable checkpoint files into
a new destination. Numeric filenames prevent source names from becoming host
paths. A streaming JSON Lines manifest preserves original name bytes, object
and link identities, decoded core metadata and explicit loss findings.

The corruption fixtures recover healthy siblings beside damaged object records
and directory trees, refuse unreadable mount roots, and compare source bytes
before and after. Tests cover exact multichunk/sparse logical bytes, hard links,
Unicode names, timestamp precision, budgets and unknown-feature refusal. A
read-only spy panics on any source write or flush attempt; injected data-read
failure preserves the exact 64 KiB prefix, and pending durable log work is
reported as excluded from checkpoint extraction.

The tool requires a stable offline image. It reports unchecked payload integrity
and preserves partial artifacts on failure. Raw discovery through damaged roots,
full metadata-preserving restoration and transactional repair retain separate
Q11 gates. Corrected the milestone's stale assignment of repair to the grow-
resize milestone; recovery work follows Q11 under M05/M14.

Validation passed: 266 workspace tests, 10 ignored; formatting, Clippy with
warnings denied, documentation checks and whitespace checks.


## 2026-09-13 - Prepare the integrated snapshot accounting decision

Made the Q4 integrated candidate reviewable: physical-run lifetimes retain
birth across reflinks, a durable bounded scan transfers eligible runs to
ordinary checkpoint quarantine, and housekeeping metadata is excluded from
recursive snapshot ownership. The proposal spells out reader roots, ID
exhaustion, incompatible-feature negotiation, metadata admission and busy-on-
active-handle deletion. These are proposed experiment choices awaiting owner
discussion; no disk fields, feature bits or accepted ADRs were changed.

The next independent queue work is read-only extraction with an explicit loss
report and a corruption corpus. Documentation and whitespace checks passed.


## 2026-09-13 - Measure snapshot retention and reclaim traversal

Added five isolated Rust model tests for Q4. With one live block and one
snapshot-owned old block, oldest-generation FIFO retention exhausted 64-block
capacity after 62 unrelated temporary-file cycles (63 retired blocks, zero
free). At 256 blocks it exhausted after 254 cycles (255 retired, zero free).
Lifetime intersection with a rotating scan completed 256 and 1,024 cycles
respectively, retaining one block and leaving 62 and 254 free. It examined
two entries per cycle in this workload; each reclaim call is capped at eight
entries. After releasing all views, both policies reclaimed every queued
entry within the asserted budget.

The tests also cover birth/retirement equality, three release orders, progress
past 32 protected entries, failed allocation preserving live state, and a
negative control that detects deliberately reused snapshot media. These
results reject conservative FIFO retention for the model's bounded unrelated
churn target. They justify integrating more precise accounting for measurement.
The rotating in-memory scan is an experimental mechanism: sealed production
queue traversal, lifetime storage, reflinks, snapshot metadata, selectable
checkpoint protection, persistence and crash recovery need their own design
and evidence. No production format or filesystem API was changed.

Updated Q4, the proposal, qualification plan and handoff to include the
protected-head traversal requirement. Shipping budgets and the accounting
format remain explicit decisions. The complete audit queue is preserved.

Validation: workspace tests passed (261 passed, 10 ignored); formatting,
Clippy with warnings denied, documentation checks and whitespace checks passed.
The five model tests are included in that total.


## 2026-09-13 — Prepare the complete implementation handoff

Added the [audit work queue](implementation/audit-work-queue.md) so snapshot
work cannot displace salvage, repair, backup/restore, adapter/cache/device,
catalog/query/stream, security, tiny-file and sustained application findings.
Each row names its owner, existing evidence and completion gate.

John authorized the recommended next step: measure the conservative
oldest-snapshot retention barrier first. Q4 and the proposal record the
experiment order; the shipping accounting mechanism, registry encoding and
retention limits are not selected by that authorization. The handoff identifies
the relevant source paths, invariant boundaries and format-review requirements.
No snapshot implementation or additional runtime qualification is claimed.


## 2026-09-13 — Exercise repeated pressure and define the snapshot experiment

After the first push, added a 24-cycle near-full test with shared survivors,
forced growth failure, cross-block COW writes, bounded reclamation and remount.
Each cycle preserves exact bytes, checks allocation ownership and restores
capacity for the next cycle. This is live-sharing evidence, not snapshot
implementation evidence.

The [snapshot proposal](proposals/persistent-snapshot-prototype.md) separates
registry publication, retention accounting, in-place isolation and admission.
Source inspection identifies the relevant gaps: reclaim promotion has no
snapshot predicate, retirement records have no birth information, in-place
eligibility ignores snapshot ownership, and the fixed allocation pool cannot
pin arbitrary old checkpoint roots. Q4 links the proposal and keeps the
conservative-barrier versus precise-accounting experiment explicit.

Clarified the failure invariant against accepted ADR-062 and its existing
regression: opted-in in-place I/O can change user bytes before metadata
publication fails. Full-COW byte preservation and rejection before I/O are
separate stronger cases.

Validation: 256 Rust workspace tests passed with 10 explicitly ignored;
formatting, Clippy with warnings denied, documentation and whitespace checks
passed. The targeted repeated-pressure test passed. Persistent snapshot wire
semantics and implementation remain future work under the accepted direction.


## 2026-09-13 — Reconcile invariants and block writes after uncertain publication

Consolidated retention/reclamation, failure boundaries, catalog completeness,
change discovery and adapter/cache obligations in [spec/invariants.md](spec/invariants.md).
Corrected [recovery](docs/19-recovery-and-maintenance.md): catalog rebuild is
possible; discarded stream history requires reset and rescan. Clarified that
reserved and quarantined blocks are legitimate allocated ownership classes,
and that an acknowledged fsync prefix cannot be discarded during recovery.
These consolidate existing contracts without new wire fields or API signatures.

The [coverage map](testing/book-review-qualification.md#normative-coverage-map)
links every family to executable tests or an owner experiment. New failure
coverage reproduced a correctness defect: after a complete checkpoint write
and failed final flush, the mounted writer could mutate again from stale roots
and allocation state. The common commit tail now sets its remount-required
state before attempting checkpoint I/O and clears it only after successful
adoption. Completed writes reporting errors and failed post-publication reads
receive the same protection. Existing WindowPoisoned adapter mappings are
reused; pre-publication transient failures keep their safe retry behavior.

Added three tests: uncertain final-barrier mutation rejection; completed-write
and adoption-read error handling; and a dynamically enumerated replacement
write/flush fault matrix with exact namespace, content and checker oracles.
The targeted six-test fault suite passed, including existing retry controls.

John selected consistent filesystem snapshots first. [ADR-069](adr/ADR-069-consistent-snapshots-first.md)
records the direction, the required snapshot protection against in-place
updates and a backup/scanner prototype. Q4 retains the bounded-retention and
persistence questions. New Q11 and Q12 own salvage/restore and storage-stack
qualification; the roadmap places their experiments before safe daily storage
claims. Snapshot storage and API encoding were not implemented or frozen.



John subsequently selected persistent snapshots first; [ADR-070](adr/ADR-070-persistent-snapshot-priority.md)
records that choice. The allocation-root pool has capacity for two selectable
checkpoints plus one commit, so arbitrary snapshot pins cannot reuse that
capacity proof. The persistent registry and retained-ownership design need
explicit prototype evidence before wire/API decisions.

Full validation: 255 Rust workspace tests passed, 10 explicitly ignored;
formatting, Clippy with warnings denied, documentation and whitespace checks
passed. The parent roadmap checker and all 13 checker fixtures also passed.
No native or long-duration qualification is implied.

## 2026-09-13 — Complete BFS book review and fragmented allocation regression

Read the twelve chapters and construction-kit appendix of Giampaolo's
*Practical File System Design* against source revision
`44ea5ebb7ba1efb5fd37588956700500f1047e1e`. The
[review](docs/33-practical-filesystem-design-review.md) records source identity,
printed pages, chapter dispositions and corrections. Earlier research had
misattributed substring-search costs to OR and overgeneralized alignment,
old duplicate performance and the absence of COW antecedents.

The allocator reproduced the book's repeated large-request fallback problem.
For 64 isolated free blocks in the 512-block allocator fixture, the original
loop made 385 allocation searches and examined 169,536 bitmap positions.
Keeping a reduced request cap for one invocation cuts those counts to 70 and
8,256 respectively (81.8% and 95.1% reductions). This is a work-count
measurement, not a throughput claim. The cap uses the existing stack variable;
two u64 counters add 16 bytes to AllocStats. No on-disk encoding or barrier
changes. The regression checks exact extent positions and verifies a later
request can use a newly freed contiguous run. Residual within-region rescans
remain measurable follow-up work.

Added a second independent byte-vector oracle covering three seeds and 576
mixed write/truncate/preallocate operations, 36 exhaustive-check/remount
boundaries, short-read buffer preservation and reflink isolation. It passed.
Existing tests already cover interrupted replay, batch failure rollback,
near-full progress and open-unlinked cleanup, so those engines were retained.

Refined catalog completeness and stream rescan requirements with explicit
backfill, enumeration/cursor handoff, reset identity and notification overflow
gates. Cache coherence, VM reentrancy, query predicates and metadata-preserving
transport have owner experiments in the
[qualification plan](testing/book-review-qualification.md). These are
requirements for their respective future facilities, not claims that those
facilities were implemented. Accepted ADRs and unresolved format choices were
preserved; allocation tuning is allowed by ADR-067's runtime-policy decision.

Validation on this macOS arm64 host with an isolated temporary Rust 1.98.1
installation: `cargo test --workspace --all-features` passed 252 tests with
10 explicitly ignored tests; `cargo fmt --all -- --check` and
`cargo clippy --workspace --all-targets --all-features -- -D warnings` passed.
`make toc`, `make check-docs` and whitespace validation passed. The ignored
qualification workloads, long soak and native hardware gates were not run.
All changes remain local; no mount, hardware write or publication was made.


## 2026-09-03 — Portable C allocates and logs its first COW data block

The independent ABI-1 writer crossed the allocation boundary with
`afspw_write_file_block_cow`. The API accepts one complete caller-assembled
logical block, rejects shrinking ranges and nonzero data beyond a partial EOF,
and keeps the 8 KiB no-heap contract. Its allocator decodes the fixed-key
allocation root, selected region descriptor and bitmap pages independently of
Rust, recross-checks all free counters, proves every existing log extent is
base-checkpoint-free and skips those reservations before choosing a block.
The same 8-to-64-block runtime emergency floor as the Rust writer protects
recovery capacity.

Durability is two-stage: write/flush the new data, then write/flush the
version-3 record. Distinct statuses and stages identify data-write,
data-flush, record-write and record-flush uncertainty. Torn data and torn
record retries both recover through a fresh probe; Rust replay observes the
exact 123-byte final-block replacement and the checker accepts the resulting
allocation. The cached seven-record path needs 25 reads, the 8 KiB path 223,
and either torn retry 50. A 512-block fixture reserves every ordinary-growth
block in a prior record while retaining the 16-block emergency floor, and
proves ENOSPC with zero writes; injected descriptor and bitmap failures report
exact LBAs. Strict C, sanitizers, static analysis, CMake and m68k compilation
retain a 768-byte maximum writer frame.

## 2026-09-03 — Portable C emits its first version-3 data mutation

The ABI-1 C writer now appends data-free truncates of existing regular files.
It supports sparse growth, zeroing and block-aligned shrink without allocating
media or publishing a checkpoint. A same-size request succeeds with an
explicit zero-I/O result. Unaligned shrink instead returns
`AFSPW_ERR_TAIL_REWRITE_REQUIRED`: exposing stale bytes from a materialized
partial tail would be worse than delaying that case until the COW data
allocator exists.

The C reader observes the new size immediately, and Rust replay verifies both
zero shrink and growth to 12,295 bytes with an exact zero-filled suffix before
running the checker. Version-3 feature absence and unsupported tail rewrite
both write nothing. Torn-record retry and flush uncertainty retain the same
diagnostic contract as namespace records. At a seven-record prefix the cached
path uses 21 reads, the 8 KiB path uses 178, and a cached retry uses 42; the
shared writer setup refactor lowers the largest measured m68k writer-function
frame to 740 bytes.

## 2026-09-03 — Portable C creates empty files without an allocator

The ABI-1 portable writer now emits an empty regular-file create into one
preallocated intent slot. Its semantic preflight validates the complete
namespace prefix, proves the parent directory and name absence, advances over
every prior logged create and returns the exact monotone object ID. The
operation allocates no data block and publishes no checkpoint; Rust replay
materializes its metadata later.

The C reader sees object 19 and zero durable bytes immediately after the
write, then Rust replay preserves the same identity and the checker accepts
the result. A case-insensitive collision and missing parent return the exact
create-lookup failure before any write. A separate valid checkpoint with
`next_object_id = UINT64_MAX` proves exhaustion is distinct from corruption
and also writes nothing. A second valid-checksum checkpoint regresses the
watermark below the highest committed object; a bounded right-edge object-map
lookup catches it before the writer can reuse an identity. The successful
paths retain 144-read 8 KiB and 21-read cached ceilings, one block write, one
flush and the bounded m68k stack contract.

## 2026-09-03 — Portable C trades caller RAM for 7x fewer writer reads

The bounded C writer used only 8 KiB, but its deliberately stateless semantic
preflight reread the same log and metadata blocks many times. On the fixed
seven-record qualification image that cost 142–152 logical reads before one
log write. A call-local LRU now uses only complete extra blocks supplied after
the mandatory workspace. It owns no memory, retains no media pointer and is
discarded on every return; the exclusive-writer and uncertain-I/O contracts
are unchanged.

The 8 KiB path remains a first-class, cross-replayed low-memory configuration.
The recommended size is 56 KiB: the original two-block workspace plus twelve
cache entries. That profile needs 21 reads for rename, delete or replacement,
and 42 across a torn-write retry's two full preflights, while preserving the
single write and flush. The executable gate fixes all three numbers as upper
bounds, compiles the same implementation for m68k and validates both memory
profiles through Rust recovery and the checker. Its main m68k writer frame is
760 bytes in the current toolchain and carries a 1 KiB regression ceiling.
This is a memory/performance choice for adapters, not a second disk format or
ABI. A transient first-log
read failure additionally proves exact stage/LBA reporting, zero media writes
and a clean fresh retry; the probe can print every preflight LBA under an
environment switch for adapter diagnostics.

## 2026-09-03 — Intent replay makes final namespace removal bounded

Durable-log replay no longer retires the complete layout of a final delete or
replacement victim. It preclaims every logged data run before allocating
metadata, then moves committed final victims into ADR-066 orphan state in the
same replay checkpoint. If object 2 was never used, its record, empty tree root
and first entries are created inside that transaction-local COW overlay; there
is no preparatory checkpoint that could stale an unapplied log. A logged write
or truncate followed by final removal publishes the final data layout under the
orphan identity, while non-final hard-link removal still decrements normally.

The focused gate replays a 33-extent victim with ordinary free space nearly
exhausted and proves the replay retires fewer blocks than the victim has
extents, writes no data and preserves every byte. Four durable records create
64 orphans in one checkpoint, and every modeled write/flush/torn cut of a
replacing replay converges to the new visible source plus the byte-exact old
target in orphan state. Shared data remains referenced until bounded cleanup.

With that invariant established, the ABI-1 C writer now exposes regular-file
delete and replacing rename alongside non-replacing rename. Each operation
performs a fresh semantic preflight, writes one preallocated log block and
flushes once. The cross-language gate observes the namespace in C, replays it
in Rust, verifies one or two expected persistent orphans and runs the exhaustive
checker. The seven-record fixture measured 142 reads for delete, 148 for
replacement and 152 for non-replacing rename, all with 8 KiB caller scratch;
the call-local cache described above subsequently addressed that
classic-hardware optimization target without removing the 8 KiB path.

## 2026-09-03 — Portable C performs its first durable mutation

The independent C99 path now appends a non-replacing regular-file rename to
the preallocated intent log. It freshly probes and validates the complete
checkpoint-plus-log namespace, proves the source type and absent destination,
writes one record and issues one flush. Rust replays the C-produced sequence-8
record and the exhaustive checker validates the materialized image. A torn
record leaves the prior seven-record namespace, can be overwritten in the
same slot after a fresh scan, and never publishes a hybrid rename. Write and
flush callback failures have separate explicitly uncertain results; full-log
and existing-target cases perform zero media writes.

The deliberately narrow initial surface exposed a correctness trap during its
audit: replay could retire a delete or replacement victim directly. The next
lot above closed that gap and expanded the same ABI. The original
maximum-prefix measurement remains 152 uncached logical-block reads, one write
and one flush with 8 KiB caller scratch.

## 2026-09-03 — Q3 allocation architecture accepted

The owner accepted ADR-067 after the post-orphan low-space requalification.
Epoch 1 therefore keeps triple-version bitmap pages and region descriptors,
the deterministic fixed-topology `3N` allocation-root pool, authoritative
bitmaps and the segmented reclaim queue. Runtime emergency headroom, rover,
locality hints and maintenance batch sizes remain tunable policy rather than
new wire fields.

This closes the allocation architecture question, not the global format
freeze. M14 still owns exact byte-layout review, overflow bounds, feature
negotiation and independent cross-reading before the wire epoch can be called
stable.

## 2026-09-03 — Emergency headroom makes ENOSPC recoverable

The post-ADR-066 Q3 pass added a soft transaction-allocation floor rather
than another reserved disk area. On volumes of at least 64 blocks the runtime
keeps `clamp(ceil(total blocks / 32), 8, 64)` raw blocks for destructive and
recovery work. Normal user/data and metadata allocations are checked against
the same floor, including allocations performed only while sealing the
reclaim queue, so a rejected growth transaction still issues no writes.
`statfs`, `afsplus-info` and `afsplus-dump` now distinguish raw free,
emergency headroom and normally available blocks.

The filesystem-facing VFS uses the hidden orphan transition for every final
file unlink or replacement, not only while a handle is open. Visible
namespace removal is therefore independent of the target's extent count;
cleanup is idle, bounded and restartable. A 160-region test grows the user
namespace past a single tree leaf, fills a highly fragmented file down to the
emergency boundary, removes its name in two bounded checkpoints, then drains
it eight extents at a time. It deliberately configures the general reclaim
budget to one block, which exposed a negative-progress loop; orphan cleanup
now promotes at least 16 blocks per step and the same test converges.

The optimized maximum-region run kept 64 emergency blocks and finished with
85 raw/21 normally available blocks after reserving 262,011 data blocks. It
issued 31 reads, 16 writes, 65,536 written bytes and two barriers, dirtied all
nine bitmap pages, one descriptor and one allocation-root node, and retained
the 32 KiB allocator-RAM bound. The 512-block and multi-node sweeps refused
fills that would cross their 16- and 64-block floors; every accepted fill
still completed destructive progress. These are memory-backend qualification
figures, not device-latency claims.

## 2026-09-03 — Open-unlinked files gain a bounded crash-restartable lifetime

The owner accepted ADR-066 after the first Q3 low-space measurements showed
that final unlink could not safely retire an arbitrarily fragmented file in
one reserve-sized transaction. The implementation reserves object ID 2 as a
lazily-created hidden directory. A final visible link with live handles moves
there atomically, including an open atomic-replace target, while the file's
ordinary link count stays one. Existing handles retain read, write, truncate,
data-policy and fsync identity; public lookup, stat, new open, hard-link and
rename cannot rediscover internal state.

Cleanup uses extent-tree subtree counts to read only a bounded tail. Each
maintenance transaction removes at most the configured logical-extent budget
and publishes a smaller orphan before a final small transaction deletes the
entry and record. Read-write mount and filesystem sync advance one orphan;
adapters also have an idle hook and a pending count. The checker treats object
2 as an explicit feature-gated root and rejects malformed names/types,
visible aliases and a missing feature bit. The dump labels the internal role;
the independent C reader negotiates the `RO_COMPAT` bit but returns
`NOT_FOUND` through its ordinary API.

The qualification exercises multiple handles, hard links, name reuse, legacy
fallback, open-target replacement, resumable multi-extent cleanup and every
modeled write/flush cut of insertion, post-unlink update, cleanup and
replacement. Every recovered intermediate is required to be checker-clean and
semantically old or new; no mount-only oracle is used.

The recorded optimized memory-backend run removed exactly eight of 33 sparse
extents in 537 microseconds and left 25 for restart. It issued 24 reads, eight
writes, 98,304 read bytes, 32,768 written bytes and two barriers; the committed
transaction contained five metadata blocks, one bitmap page and 512 bytes of
allocator bitmap payload. These are qualification evidence for this build,
not a hardware-performance claim.

## 2026-09-03 — Q3 qualification exposes the missing emergency reserve

The first combined near-full allocation qualification found a liveness gap
before the prototype encoding could be proposed for freeze. A successful
preallocation could consume the last raw free blocks and leave the volume
checker-clean but unable to allocate the metadata needed by unlink. On the
512-block single-region fixture, unlink allocated four blocks after one
quarantine promotion and failed with two raw free blocks; on the 145-region
fixture it allocated five blocks, wrote two reclaim structures and failed
with three raw free blocks. The observed thresholds are measurements, not a
safe reserve definition.

The same suite pinned the good side of the boundary: a failed ENOSPC attempt
issued zero writes and zero barriers, a near-full unlink survived every
modeled power cut as exactly the old or new generation, and bounded reclaim
restored space. One maximum-size 1-GiB region reserved 262,075 blocks in 4.7
ms in the recorded optimized memory-backend run, with 31 reads, 16 writes,
two barriers, nine bitmap pages, one descriptor, one allocation-root
record/node and 32 KiB of
allocator bitmap payload. A 145-region transaction crossed the allocation
root leaf boundary while staying below 64 KiB.

This made the dependency on open-unlinked/orphan cleanup explicit. Unlinking
a highly fragmented file cannot reserve metadata proportional to the whole
file on a classic volume; namespace removal needs a bounded orphan step, then
restartable reclamation. Q3 therefore stays open while that M03 machinery and
the already-specified free-versus-available headroom contract are implemented
and requalified.

## 2026-09-03 — Checkpoint selection regains Rust/C parity

The checker corruption corpus made Rust reject three reserved checkpoint
fields, but the independent C reader still accepted them when the block carried
a matching CRC. That divergence let Rust and C select different retained
generations from the same forged or future-format image.

The portable decoder adopted the same zero requirements for common-header
flags, common-header owner and payload flags. Its Rust-produced fixture mutates
each field independently, reseals the block, and tests both safe fallback and
the two-invalid-slot failure. A focused Rust codec regression pins the other
side of the contract so either implementation changing alone becomes visible
at its normal developer gate.

## 2026-09-03 — Prototype commands become integration-grade tools

The formatter moved from a convenience binary to the official `mkafsplus`
contract. It gained named compatibility profiles, reproducible UUID/timestamp
inputs, stable JSON and diagnostics, and same-directory staging so a failed
format never exposes a partial destination. This also uncovered an old
geometry wart: arbitrary MiB sizes inherited the total block count as their
region size and therefore failed whenever that count was not a power of two.
The official formatter uses the fixed maximum region geometry and accepts
ordinary image sizes.

`afsplus-info` and `afsplus-dump` deliberately split cheap recognition from
forensic depth. Info reads only identification and both checkpoint slots;
dump walks the full committed object/directory/extent/allocation/reclaim/shared
state and the durable log prefix. Both use a real OS read-only descriptor plus
fail-closed mutation methods. Black-box tests run all three installed command
names, pin the schema and exit contracts, distinguish corrupt media from host
I/O, and repeat both inspectors against a populated mode-0444 image while
proving its bytes and permissions unchanged.

## 2026-09-03 — Rust codec failures gain stable case identities

Five dependency-free fuzz targets began exercising the identification,
checkpoint, typed-tree, object-record and intent-log codecs. Every seed must
round-trip to one canonical block; the deterministic engine mixes raw CRC
failures with resealed semantic mutations, short inputs, multi-byte changes
and bounded extensions. The default gate runs 20,480 cases and records the
target and case before each call.

Before publication, the same engine also completed an extended run of 100,000
cases per target (500,000 total) without a panic or canonical round-trip
failure.

The separate `.afrf` artifact format keeps the target, stable case number,
seed-schema version and exact bytes needed for replay. This made failures
portable across machines without requiring `cargo-fuzz` or a particular LLVM
runtime, while leaving room for those engines to call the same pure decoder
entry points later. Confirmed artifacts placed in `fuzz/regressions/` become
permanent repository-gate inputs.

## 2026-09-03 — Checker corruption becomes a replayable corpus

The Rust checker gained a twelve-image corpus spanning identification,
checkpoints, typed trees, object records, allocation bitmaps and the intent
log. Each surface has a raw integrity failure and a valid-checksum semantic
failure. Fixed UUIDs, timestamps and geometry make the sparse images and
their exact JSON reports byte-reproducible; a versioned manifest names every
changed LBA and byte range so another implementation can replay a failure
without reading the Rust tests.

Building the cases exposed two gaps before publication. Checkpoint decoding
accepted nonzero reserved header and payload fields, so both retained records
could carry unknown state without failing selection. Intent scanning also
treated a nonzero undecodable record exactly like a zero unused slot. The
decoder now rejects the checkpoint fields, while the log keeps its legal
crash-boundary semantics and adds a forensic warning that distinguishes torn
or damaged bytes from an empty tail.

## 2026-09-03 — Portable C overlays the durable namespace

The independent C99 reader gained the namespace half of the intent-log view.
It now resolves and enumerates the final checkpoint-plus-log directory state,
chases rename chains without recursion, honors replacement, and keeps object
identity correct across hard-link deletion and rename. The same semantic pass
checks source/target preconditions, monotone create IDs and write/truncate
expected sizes before file bytes are exposed. Calls retain the 8-KiB
caller-owned scratch contract; an explicit cursor makes ordered iteration and
small-buffer retries possible without hidden allocation or global state.

The Rust-produced fixture grew from three to seven durable records and now
combines delete, write, truncate, create, rename chains and replacement.
Portable C matches both surviving oracle files and the final
case-insensitive namespace, while four valid-checksum mutations demonstrate
that semantic inconsistencies fail at the responsible record. A seventh AFZF
seed carries a successful namespace lookup through the deterministic
ASan/UBSan mutation and replay gate. The reader also learned
[ADR-065](adr/ADR-065-persistent-data-update-policy.md)'s per-file policy flag
and rejects the flag without its volume feature, keeping the Rust and C format
boundaries congruent.

## 2026-09-03 — The per-file data policy becomes persistent

ADR-062 had fixed the hybrid data-update semantics but deliberately left the
per-file opt-in unencoded, so the choice evaporated at every mount. ADR-065
(approved on board decision #63) assigns the encoding: object flag bit 1,
`OBJECT_FLAG_DATA_IN_PLACE`, protected against silent loss by the validated
object-flags namespace every implementation already enforces, plus `COMPAT`
bit 0 `org.aros.afsplus:data-policy` activated by mkfs profile with
shared-extent-style congruence — a flagged record on a volume without the
feature is corruption in the read path and the checker. The core gained
`set_file_data_policy`/`file_data_policy` (a metadata-COW record update) and
the VFS a `DATA_POLICY` capability with handle-based set/get; the write path
takes the in-place route when the record carries the flag, under unchanged
ADR-062 eligibility. One real bug surfaced while wiring it: layout staging
rebuilt object flags from scratch, so any write dropped the freshly set
policy bit — the persistence regression caught it immediately, and the bit
now travels through every layout rewrite. Seven persistence regressions plus
two VFS API tests cover remount survival, in-place stats, clearing, feature
refusal, planted-flag fail-closed on both sides, shared-block COW fallback
and the power-cut contract through the flag.

## 2026-09-03 — Portable C reads the durable log view

The independent C99 implementation no longer stops at the selected
checkpoint when a writer has acknowledged later fsync groups. With an 8-KiB
caller-owned scratch buffer it scans the bounded intent-log prefix, validates
record shape and sequence, content-verifies every referenced replacement block
and reports the first excluded tail slot and LBA. No replay is written to the
device.

The first view layer resolves create, write and truncate records into final
file sizes and bounded reads. Its cross-language fixture leaves three durable
records over a Rust checkpoint, then C reconstructs an existing file after
write-plus-shrink and a file that exists only in the log, byte-for-byte against
Rust-produced oracle files. Damaged data, sequence gaps and missing feature
contracts have exact regression expectations. Rename and delete explicitly
disable the file-data view until namespace identity is overlaid rather than
returning plausible but incomplete bytes.

The same decoder is now the sixth compact mutation seed. Parent and child
tree item counts are also compared during descent, closing an independent
review observation where an internally consistent parent over-claim could
otherwise hide reachable entries. Strict C99, sanitizer, analyzer, CMake and
m68k compilation remain one gate so future integrators get the same failure
coordinates as the in-tree build.

## 2026-09-03 — Portable C failures become replayable artifacts

The first portable-reader expansion substantially increased the amount of
untrusted C parsing, so its next lot hardened that surface before adding more
features. A host recorder now executes successful operations against a real
Rust-built image and writes only the blocks that were actually observed. Five
12–48-KiB packets retain checkpoint probing, object lookup, both extremes of a
303-entry directory and a complete directory-to-file read without copying or
carrying the complete disk image through every mutation.

Each packet receives 4,096 stable mutations under AddressSanitizer and
UndefinedBehaviorSanitizer in the ordinary repository gate. Every operation
header byte and the first 128 bytes of each observed block are tested both
with a broken checksum and with a recomputed CRC, so structural checks are not
hidden behind the checksum gate. Block-wide deterministic flips, overwrites,
multi-byte changes and truncations follow. The progress record is updated
before execution, so a crash names a seed and case number; the same case can
export an exact `.afzf` artifact and the replay tool reports symbolic operation
results plus the terminal stage and LBA. This turns parser failures into small
regression inputs an integrator can keep and exchange.

The harness also exports the standard libFuzzer callback. The installed Apple
command-line tools do not currently contain its link runtime, so that engine
is opportunistic rather than a hidden local prerequisite; the deterministic
sanitizer runner is the portable mandatory gate. The remaining M01 corpus is
listed by wire surface instead of being represented as a generic “fuzzing
open” checkbox.

## 2026-09-03 — The independent portable C reader starts executing

The first ADR-029 implementation slice added a C99 reader that does not link or
generate code from the Rust codecs. Caller-owned block I/O and one 4-KiB
scratch block are sufficient to validate the identification record, negotiate
the implemented feature summary, validate bounded geometry and structurally
select between both retained checkpoints. The public reader surface is
versioned before expansion so future profiles do not depend on native C struct
serialization.

Its cross-implementation gate formats and advances an image with Rust, compiles
the reader under strict C99 warnings, and asks C to select it. The same binary
injects a bad newest checkpoint, valid-checksum structural corruption, two bad
checkpoints, an impossible duplicate generation, a selected-state feature/root
mismatch, a bad identification checksum and a short scratch buffer, with
structured stage/LBA/per-slot diagnostics. The feature/root case proves that
selection does not hide corruption beneath a newer structurally valid
checkpoint by falling back. Sanitizer, CMake-example and available AROS-m68k
compiler gates catch host-language and integration faults; Rust's checker
validates the untouched source image. Object-tree traversal and file reads form
the next reader slice rather than being simulated through the Rust
implementation.

The next slice made that bootstrap useful without turning it into a second
host runtime. A 303-file Rust fixture forces object-map and directory descent
through internal nodes; C enumerates the root by ordinal, resolves a file
object, and reproduces its bytes through bounded 777-byte reads. A wire-valid
extent leaf over the same Rust-written data adds sparse-hole and extent-floor
coverage. Tree CRC, valid-checksum identity, generation/owner/level/range/count
invariants, typed object/directory/extent values and exact failing LBAs are now
checked along every accessed path. The API remains additive: the original
entry shape stays intact, new operations advertise capability bits, and all
storage remains caller-owned.

Independent review also caught that the checkpoint payload's reserved `flags`
word had no explicit reject-or-ignore rule in either implementation. Rather
than make the C reader silently stricter than Rust, the freeze choice is now
[Q10](implementation/open-questions.md) and must be closed during M14.

## 2026-09-03 — FSKit durability moves to write replies

The owner chose a scoped workaround instead of debugging the macFUSE/Apple
FSKit boundary. The macOS FSKit mount treats each delivered data write or
truncate as an implicit durability point and replies only after the AFS+ log or
checkpoint is durable. This preserves host `fsync` correctness even when the
transport omits `FUSE_FSYNC`, without changing the core, on-disk format,
host-neutral FUSE behavior or AROS adapters and without requiring the legacy
kernel extension.

The tradeoff is intentionally visible: small host writes lose cross-request
batching and can each pay the data-and-record barriers. A protocol regression
crashes immediately after successful write and truncate replies without an
`fsync` request. The real macFUSE mount then passed its syscall-boundary oracle:
host `fsync`, immediate second-descriptor inspection, clean unmount and checker
all succeeded without administrator authorization. The host benchmark contract
must label the fallback.

## 2026-09-03 — Qualification order follows executable targets

The owner reordered the four platform-validation stages so an unavailable
bare-metal target cannot block executable qualification. The sequence is
Hosted MacAROS, Amiga 500/m68k emulation, the physical A500, then native
MacAROS on Apple Silicon after that system exists. These are three target
platforms: the emulator and physical A500 are two stages for the same classic
target.

This ordering change does not weaken the reopened host `fsync` verdict. The
real FSKit mount proves that the syscall returns without delivering a FUSE
`FSYNC` request to AFS+, while direct VFS and FUSE-protocol tests exercise the
expected durability path. The defect is therefore below the AFS+ engine, in
the macFUSE-FSKit/Apple-FSKit path. The trace does not by itself assign the bug
between macFUSE's shim and Apple's FSKit plumbing; a stock macFUSE loopback
reproducer is the attribution gate.

## 2026-09-03 — macFUSE FSKit reopens the host fsync gate

Re-running the real mount after wiring the VFS log exposed a false-positive in
the Mountable Alpha-0 host gate. On macOS 26.6.2 (25G83) with macFUSE 5.3.3's
FSKit backend, both Rust `File::sync_all` (`F_FULLFSYNC`) and a direct
`fsync(2)` returned after dirty data reached the FUSE `WRITE` callback but no
FUSE `FSYNC` callback was delivered. Instrumentation distinguished `WRITE`,
`RELEASE` and `FSYNC`; only `RELEASE` arrived when the descriptor closed. For
two seconds after the syscall, a second descriptor on the image saw the same
checkpoint generation and zero pending intent records. The later rename or
unmount checkpoint had allowed the old round-trip test to pass.

The ignored real-mount test now inspects the live backing image immediately
after host fsync and requires either a replayable record or a newer checkpoint.
It intentionally fails on that FSKit combination, while the direct FUSE
protocol, VFS, AROS and C-bridge fsync tests pass. M08 is therefore partial
again. A safe FSKit-specific write-through workaround would add barriers to
every delivered write, so it requires measurement and an explicit policy
choice rather than being enabled silently; the preferable fix is for the host
stack to deliver FUSE `FSYNC` before returning from the syscall.

## 2026-09-03 — Logged data fsync reaches the portable Rust adapters

The portable VFS now routes existing-file writes and truncates through the
version-3 data-update window when the volume advertises that incompatible
feature. Reads and stats see the pending logical layout before publication;
`fsync` writes the bounded data/record barriers without advancing a checkpoint,
while oversized/full groups, namespace operations, reflinks and filesystem
sync publish the window through the ordinary checkpoint engine. Log-free and
older volumes retain their immediate checkpoint behavior.

The FUSE protocol, packet-neutral AROS Rust adapter and current C ABI bridge
inherit this path from the VFS. A trace test proves a successful VFS fsync
writes no checkpoint slot and that remount recovers its bytes; adapter tests
cover pre-fsync visibility, remount and checker cleanliness. The next
portability boundary is the independent portable-C filesystem implementation,
followed by real-device barrier measurements.

## 2026-09-03 — Intent-log writes and truncates survive nested recovery crashes

The accepted ADR-063 architecture now covers committed-file data changes in
the Rust core. Experimental record version 3 names freshly allocated COW
replacement blocks for writes and, when needed, one zero-tailed block for a
partial shrinking truncate. Data is flushed before the record; replay checks
the complete-block CRC, claims those exact blocks while they are FREE in the
base checkpoint and publishes the final layout through the ordinary COW
transaction engine. Multiple fsync groups on one file collapse into one final
metadata mutation without weakening their monotone-prefix recovery contract.

The new matrix covers old-or-new cuts around the live write, every write and
flush of recovery itself followed by another remount, write-plus-rename,
sparse growth, aligned and partial shrink, successive writes, and a reflink
split whose other owner retains the old bytes. Release workloads measured
2.200 writes/2.032 flushes per 200-byte logged append and 2.134/2.032 per 4 KiB
DB hot-set rewrite, versus 9.037/3.000 for checkpointed append. These are
memory-backend structural counts, not Apple or Amiga hardware results.

Review exposed a compatibility trap: an older version-2 scanner treats an
unknown record version as a torn tail. ADR-064 therefore assigns
`org.aros.afsplus:intent-log-data-updates` as a dependent INCOMPAT identity so
such readers reject the volume before silently losing an acknowledged fsync.
The numeric wire remains experimental; VFS/C parity, real-device flush testing
and the M14 format review remain open.

## 2026-09-02 — Q1 and Q2 architecture blockers closed

The shared-extent implementation made the private/shared write fork executable,
so Q1 was settled by measurement rather than preference. A runtime-only
private-in-place prototype was compared against full data COW on random 4 KiB,
database-hotset, append and reflink-first-write workloads. The large random
case fell from 26,972 to 8,000 device writes, 24,870 to 3,000 allocations,
21,936 to 2,000 retirements and 1,767 to one final extent. The database hot
set retained a smaller but material advantage; append and shared first-write
rows were identical because they correctly remained COW. Power-cut and
injected-error tests demonstrated the price directly: an old metadata
generation may expose old, new or torn overwritten bytes, and a returned error
does not imply byte rollback. ADR-062 therefore selected full COW by default
plus a persistent, explicit per-file private-in-place opt-in, never automatic
detection.

Q2's optimized re-run held the earlier result: a logged Git-style ref update
cost 2.152 writes and 1.032 flushes per operation, versus 10/3 for one
group-commit checkpoint and 26.992/6.998 for the original sequence. Review of
the executable record format caught an overclaim in the old report: version 2
can replay create/delete/rename and created-file content, but not writes or
truncates of existing files. ADR-063 accepted checkpoint COW, bounded group
commit and the intent log as the layered epoch-1 architecture while refusing
to freeze that namespace-only wire. Existing-file replay, the persistent Q1
policy representation, portable-C parity and real-device qualification remain
M14 gates rather than unresolved architecture choices.

## 2026-09-02 — Shared extents and reflinks qualified

The ADR-061 implementation completed the volume-wide reference tree, shared
copy-on-write writes, `CloneFile`, byte-range `CloneRange`, checker
reconstruction and corruption cases, and every-write/every-flush crash
matrices. Review caught and fixed the dangerous rc=2→1 case before closure:
removing one of two mappings deletes the reference record but never retires
the surviving file's blocks. A second review found a left-boundary prefetch
case that could leave adjacent equal-count records non-canonical; its
regression now exercises rc=2/rc=3→rc=2 merging.

When the original implementation session reached its limit, the remaining
range-clone lot was taken over in the shared checkout. The completed operation
shares every representable full block, privately copies partial boundaries,
keeps holes sparse and publishes both file maps with the reference tree in one
checkpoint. The same semantics are exposed through the portable VFS with
per-volume capabilities. The retained commands, revisions and limitations are
in [the completion report](implementation/shared-extents-completion.md).

## 2026-08-31 — Team board adopted

Coordination between agents and the owner moves to the shared
[agent-board](https://github.com/jonx/agent-board) (project `afsplus`).
`board init . --agents claude` wrote [.mcp.json](.mcp.json), the Claude Code
hooks under `.claude/` and the protocol block in [CLAUDE.md](CLAUDE.md);
[AGENTS.md § Team board](AGENTS.md#team-board) points to it. The board is the
live conversation; this file stays the repository's record.

## 2026-08-31 — Shared extents chosen as the next epoch-1 lot

With Mountable Alpha-0 closed, the next objective is the largest unimplemented
epoch-1 *requirement* rather than an open question: [ADR-027](adr/ADR-027-reflink-clones.md)
has been "Accepted as an epoch-1 format requirement" since the design phase, a
freeze gate depends on it, and nothing implemented it — the extent record
carried one flag and no reference state existed. ADR-027 itself records why
that cannot be deferred: sharing changes the allocator, checker, reclamation
and reverse-map invariants at once. It is also coupled to open question Q1,
because reflink-shared ranges always copy on write, so the write path must fork
on shared versus private — exactly where the data-policy decision lives.

[ADR-061](adr/ADR-061-shared-extent-references.md) selects the mechanism: a
volume-wide typed reference tree on the ADR-034 engine, keyed by physical run.
A first draft was revised after review found six contract gaps, two of them
genuine correctness holes: an operation on a flagged extent must partition by
overlap rather than look up its start, because peers split the underlying runs
independently; and although clone is not journalled, the existing journalled
delete and rename-replacement can remove a file holding shared extents, so
their replay has to maintain the counts. The review also refused the
checkpoint offset until the transitional inline region records — dead since
[ADR-035](adr/ADR-035-allocation-root-reserved-pool.md) but still decoded when
the allocation root is zero — were removed outright rather than left to
collide with a new field.

That review exposed a defect in these very rules: they declared ADRs immutable
from the moment they were written, which leaves no way to correct a draft that
review has rejected. The rule now says what it should have said — immutable
from acceptance, revisable while `Proposed`.

## 2026-08-31 — Documentation restructured around one home per fact

The repository's documentation is reorganised so that state, design,
procedure and history each have exactly one home
([README.md § Documentation map](README.md#documentation-map)), every document
is reachable from an index, and [`tools/check-docs.py`](tools/check-docs.py) enforces the mechanical
part of the contract (`make check-docs`).

Measured before the restructuring: 132 Markdown files (13,785 lines) outside
the code trees; 82 files carried a `Status:` line (60 ADRs and 22 others); 365
journey words ("now", "still", "remains", ...); 2 Markdown links between
documents against 118 backticked path mentions (9 of them not resolving from
the repository root); 59 documents referenced from nowhere; 31 files longer
than 150 lines, none with a table of contents; `FILE_INDEX.md` and
`SHA256SUMS.json` generated once in the initial commit and never again.

What moved where:

- [`implementation/milestones.md`](implementation/milestones.md) is the only status authority. [`README.md`](README.md)
  keeps a five-row summary; [`ROADMAP.md`](ROADMAP.md) carries the stage plan only and links
  each stage to its milestones.
- `Status:` lines outside `adr/` were removed. Where a line carried a
  substantive sentence (docs 23–26, the fsync baseline) the sentence stays as
  plain text; the classification words are recorded here:

| File | Removed line |
|---|---|
| [`ROADMAP.md`](ROADMAP.md) | Status: initial review complete. Continue subsystem-by-subsystem only when implementation reaches that subsystem. |
| `CODEX_HANDOVER.md` | Status: active handover document for continuing architecture supervision and implementation review. |
| [`docs/23-pfs3-stage0-review.md`](docs/23-pfs3-stage0-review.md) | Status: architecture review v1, completed before the first implementation milestone. Source-level study should continue when the corresponding AFS+ subsystem is implemented. |
| [`docs/24-filesystem-comparison.md`](docs/24-filesystem-comparison.md) | Status: design benchmark. This table compares architectural capabilities, not marketing claims. AFS+ entries marked Planned or Proposed are not implemented yet. |
| [`docs/25-filesystem-wishlist.md`](docs/25-filesystem-wishlist.md) | Status: product/design exploration. Nothing in this document is automatically a 1.0 requirement unless promoted by an ADR. |
| [`docs/26-debug-observability.md`](docs/26-debug-observability.md) | Status: required development architecture. Most facilities are runtime-optional and must have near-zero cost when disabled. |
| [`docs/27-rust-implementation-strategy.md`](docs/27-rust-implementation-strategy.md) | Status: implementation direction |
| [`docs/28-virtual-images-and-viewports.md`](docs/28-virtual-images-and-viewports.md) | Status: developer-platform design |
| [`docs/29-first-class-content-inspection.md`](docs/29-first-class-content-inspection.md) | Status: developer/security API design |
| [`docs/30-portable-security-model.md`](docs/30-portable-security-model.md) | Status: security architecture proposal |
| [`docs/31-extreme-workloads.md`](docs/31-extreme-workloads.md) | Status: workload architecture and qualification plan |
| [`docs/32-reflink-clone-semantics.md`](docs/32-reflink-clone-semantics.md) | Status: epoch-1 design requirement for shared data extents |
| [`testing/extreme-workload-benchmarks.md`](testing/extreme-workload-benchmarks.md) | Status: required qualification design |
| [`testing/security-model-conformance.md`](testing/security-model-conformance.md) | Status: required test plan before security-format freeze |
| [`testing/aros-system-volume-qualification.md`](testing/aros-system-volume-qualification.md) | Status: required after Mountable Alpha-0 secondary-volume qualification |
| [`testing/benchmark-contract.md`](testing/benchmark-contract.md) | Status: required benchmark policy |
| [`testing/security-scanning-benchmarks.md`](testing/security-scanning-benchmarks.md) | Status: qualification requirement |
| [`implementation/peer-review-prototype-plan.md`](implementation/peer-review-prototype-plan.md) | Status: implementation guidance after external review |
| [`implementation/fsync-intent-log-baseline.md`](implementation/fsync-intent-log-baseline.md) | Status: measurement report for architecture blocker 2 (prefix removed, sentence kept) |
| `proposals/*.md` | Status: proposal — for team review … (replaced by a "Target on acceptance" line; the review request is stated once in [`proposals/README.md`](proposals/README.md)) |

- The M06 detail that lived in the milestone cell (which platforms are
  qualified by which gate) is now
  [testing/aros-system-volume-qualification.md § 7](testing/aros-system-volume-qualification.md#7-qualified-configurations).
- `FILE_INDEX.md` and `SHA256SUMS.json` are deleted: no generator, no
  consumer, stale since the first commit; Git and the per-directory indexes
  replace them.
- `CODEX_HANDOVER.md` (955 lines, written 2026-08-29 at commit `c2c2ed3`) is
  split and deleted. Where each section went: §1 role, §2 context precedence,
  §19 things not to do, §20 files to read first, §21 use of the conversation,
  §22 expected interaction and §24 resume sentence → [`AGENTS.md`](AGENTS.md); §3 what
  AFS+ is and §4 language strategy → the review lens in [`AGENTS.md`](AGENTS.md), the
  facts themselves being [README.md](README.md), [ADR-028](adr/ADR-028-rust-reference-core.md)
  and [ADR-029](adr/ADR-029-dual-reference-implementations.md); §5 current
  implementation, §6 lesson from the first crash matrix, §7 hardening list and
  §23 next action → the dated entries below; §8 architecture blockers and §13
  developer-idea maturity → [implementation/open-questions.md](implementation/open-questions.md);
  §9 allocator experiment → the design is [docs/07](docs/07-allocation.md) and
  [ADR-035](adr/ADR-035-allocation-root-reserved-pool.md), the G1/G2/G3 reuse
  workload with its allowed and forbidden states is
  [testing/crash-testing.md](testing/crash-testing.md#block-reuse-across-generations),
  its metrics are in [testing/benchmark-contract.md § 1](testing/benchmark-contract.md#1-benchmark-dimensions);
  §10 crash-consistency rules → the reviewer checklist in [`AGENTS.md`](AGENTS.md), the
  design being [docs/08](docs/08-transactions-and-journal.md) and
  [ADR-021](adr/ADR-021-deferred-reclamation.md); §11 feature evolution →
  [docs/09](docs/09-feature-framework.md); §12 derived versus authoritative →
  [ADR-012](adr/ADR-012-catalog-derived.md), [ADR-013](adr/ADR-013-change-stream-bounded.md);
  §14 workload philosophy and §15 benchmark contract →
  [docs/31](docs/31-extreme-workloads.md), [testing/benchmark-contract.md](testing/benchmark-contract.md);
  §16 debuggability → [docs/26](docs/26-debug-observability.md) and the
  regression-scenario rule in [`AGENTS.md`](AGENTS.md); §17 portability rules →
  [docs/14](docs/14-paths-and-namespaces.md), [ADR-017](adr/ADR-017-namespace-outside-format.md);
  §18 security philosophy → [docs/30](docs/30-portable-security-model.md),
  [ADR-031](adr/ADR-031-portable-security-acls.md); the superseded ideas →
  [adr/README.md § Superseded directions](adr/README.md#superseded-directions).
  Two ADRs gained relation lines the text already implied: ADR-009 is amended
  by ADR-020, ADR-042 by ADR-056 and ADR-057.

The [`README.md`](README.md) status section read as follows before it was reduced to the
five-row table (moved here verbatim from `README.md § Status`, commit
`624b6a8`):

> This repository now includes an executable Rust prototype, checker, portable
> VFS API, FUSE protocol adapter, packet-neutral AROS DOS adapter, a versioned
> AROS C/staticlib boundary, a cross-qualified native `DosPacket` translator,
> bounded trackdisk partition adapter and fully linked off-tree handler module.
> The host adapter and mount CLI can be built with
> `cargo build -p afsplus-fuse --features fuser-adapter --bin afsplus-mount`.
> Linux uses fuser's native mount path. On macOS, install macFUSE and build with
> `--features macfuse-mount`; the mount CLI selects macFUSE's user-space FSKit
> backend and the mountpoint must be an existing directory or a new direct child
> of `/Volumes`. The library is loaded at runtime, so ordinary workspace builds
> do not require a system FUSE installation. If macOS's File System Extensions
> switches are inert, use the diagnostic and reversible workaround in
> [`docs/macos-fskit-activation.md`](docs/macos-fskit-activation.md). Fuse-T's NFS transport is not a raw
> substitute for macFUSE's message channel; see ADR-040. The native AROS bridge
> and its cross-build qualification are documented in
> [`docs/aros-native-bridge.md`](docs/aros-native-bridge.md), ADR-042 through ADR-060.
> [`tools/check-hosted-aros-alpha0.sh`](tools/check-hosted-aros-alpha0.sh) now qualifies a bidirectional Hosted
> MacAROS → macFUSE → Hosted MacAROS round trip on one checked image. Hosted
> intent-log replay and the cumulative post-bootstrap S1 system-volume pivot are
> also qualified. The handler now supports standard AROS `Mount SHUTDOWN`
> termination, restart through the retained device node and a final
> `Assign DISMOUNT`. AFS+ is distributed as an external `L:` handler plus
> DOSDriver and does not require acceptance of AFS+ into the upstream AROS
> source tree; an upstream build recipe remains optional. Genuine defects found
> in generic AROS interfaces are still fixed with focused regression-tested
> patches and proposed upstream. AROS qualification images use the versioned
> Unicode 16 case-insensitive, spelling-preserving namespace; general-purpose
> images remain configurable and default to case-sensitive lookup. The native
> Apple-AArch64 pre-hardware QEMU gate now loads the external handler from a
> versioned retained image, executes the same operation matrix and proves a
> clean dismount/unload. File-backed guest RAM lets the host extract and
> strictly check the mutated payload; all six modeled intent-log cuts also
> replay to their exact old/new state with zero pending records. The RAM block
> transport still makes no reset-durability or Apple-hardware claim; those
> remain explicit gates. Autonomous boot selection and the classic ports also
> remain later gates. Qualification is reported as three target platforms over
> four ordered stages: Hosted MacAROS, native MacAROS/Apple Silicon, Amiga
> 500/m68k emulation, then the physical A500. The emulator is the pre-hardware
> validation stage of the A500 target. [`tools/check-aros-m68k-boot-fsuae.sh`](tools/check-aros-m68k-boot-fsuae.sh)
> now makes the first native m68k boot prerequisite machine-readable with
> matching official ROM, floppy and system-media hashes.
> [`tools/check-aros-m68k-alpha0-fsuae.sh`](tools/check-aros-m68k-alpha0-fsuae.sh) now also qualifies the real external
> handler's Alpha-0 operation matrix and all six recovery cuts on both the
> M68020-or-newer reference profile and an A500-configured plain M68000
> profile. The latter rejects M68020 long-multiply instructions before boot and
> binds the patched LLVM library in its evidence. This deliberately makes no
> physical-A500, final constrained-memory-budget or hardware-performance claim;
> ADR-057 records the boundary. ADR-058 adds a measured 8-MiB emulator profile
> and proves that sequential shutdown/restart cycles reuse one DOS-loaded
> segment without material memory growth; 4 MiB does not boot the control AROS
> profile far enough to test AFS+. ADR-059 makes AROS fatal diagnostics an
> explicit verdict across Hosted, native-QEMU and FS-UAE gates, so a modal
> requester cannot be mistaken for a hang or a successful outer process.
> ADR-060 closes the Mountable Alpha-0 objective with
> [`tools/check-mountable-alpha0.sh`](tools/check-mountable-alpha0.sh): one checksummed composite result binds the
> portable APIs, real macFUSE round trip, Hosted/native operation matrices and
> twelve recovery cases. This completion makes no physical-hardware or format
> freeze claim.

Two facts from that paragraph found finished-state homes rather than this
journal: the mount-CLI build instructions are in
[README.md § Build, test and mount](README.md#build-test-and-mount), and the
external-handler distribution model is stated in
[README.md § What AFS+ is and is not](README.md#what-afs-is-and-is-not) with
[ADR-050](adr/ADR-050-external-aros-handler-lifecycle.md). The rule that
genuine defects in generic AROS interfaces are fixed with focused
regression-tested patches and proposed upstream is retained in [AGENTS.md](AGENTS.md).

## 2026-08-31 — Mountable Alpha-0 closed

[`tools/check-mountable-alpha0.sh`](tools/check-mountable-alpha0.sh) produced `result=PASS` with
`hardware_claim=none`: the portable VFS, FUSE protocol, intent-log and AROS
adapter suites, one real macFUSE/Hosted same-image round trip with clean
checkers at every boundary, six Hosted and six native replay cases (four exact
old states, two exact new states), no guest failure requester, and a
156-file checksum manifest that validated in full. The evidence set was
re-verified independently the same day in the gate's reuse mode. ADR-060
records the decision; M08 is complete; M06 stays partial because no physical
hardware was involved.

## 2026-08-29 — Region allocator, bounded mount and shared COW trees

(Moved from `CODEX_HANDOVER.md` § 5 and § 23; the commits are `7a61c06`
through `35ec483`.)

Commit `7a61c06` completed the immediate hardening and the first Stage 6
allocator experiment: checkpoint selection is structural and never falls back
over corrupt state; same-generation slots are ambiguous; the negative
bad-ordering crash test is present; triple-buffered region descriptors and
three reserved generational slots per logical bitmap page break allocator
self-reference; retired blocks spend one generation in quarantine before
reuse; create-with-content, delete, the G1/G2/G3 reuse crash workload and
resource measurements are implemented. The first major proof after the
bootstrap prototype was not "allocation works" but "AFS+ can reuse storage
after deletes without any selectable checkpoint ever observing stale metadata
that points at newly reused content" — that invariant passed the modeled crash
matrix.

The continuation replaced the eager mount walk with bounded root loading.
Ordinary mount reads identification/checkpoints, the object-map root, root
object/directory-tree root and the retired-list root; descendant records are
decoded on access and allocation bitmaps are split across independently
checksummed pages, loaded as a mutation touches them through the selected
region descriptor. Clean pages that fail an allocation scan are evicted
immediately; the checker retains the exhaustive whole-volume view.

The Stage B1 continuation supports the proposed 262,144-block (1 GiB at 4 KiB)
region. The verifiable wire representation needs nine bitmap pages, not the
earlier rough estimate of eight. Three region-descriptor slots plus three slots
per bitmap page reserve 30 blocks (120 KiB, about 0.0114 %) per full region;
region 0 additionally contains ident and the two checkpoints. A small
transaction writes one dirty bitmap page and one descriptor before the
checkpoint. Cross-page allocation, corruption deferral/detection, quarantine
and power-cut behaviour are covered by executable tests. The multi-page design
deliberately does not put one record per bitmap page in the checkpoint: a
triple-buffered reserved region descriptor, itself selected by the
checkpoint's one record per region, binds the bitmap pages
([docs/07 § 3.1](docs/07-allocation.md)).

Core Scale-1 began with ADR-034 and the shared `AFST` node format: tree kind,
owner, level, strict binary keys, exact subtree item count and a counted
reference for every internal child, so an internal split can be planned from
the current page without reading every child. Transactional multi-upsert
copies a committed path once, keeps later changes in a write overlay, performs
balanced leaf/internal splits and grows the root; deletion merges or
redistributes underfull siblings and collapses one-child roots. A permuted
300-key test grows three levels, then deletes 299 keys in another permutation
and returns to one root leaf; a bad-level corruption test is executable. The
object-map adapter became authoritative (mkfs/checkpoints, bounded mount/stat,
mixed create/delete, checker ownership, crash matrices, a typed 1,001-entry
multi-page test), then the directory adapter (typed leaf at mkfs, bounded
lookup, exhaustive enumeration, 1,000-entry and end-to-end 300-entry
checked/remounted tests). Legacy object-map and directory codecs stayed as
transitional tests.

The namespace continuation removed the root-only API restriction: directory
lookup/enumeration, file creation/deletion, mkdir/rmdir, hard links and
same/cross-directory rename address parents by stable object ID. Rename
publishes both parent trees, parent records, the moved record and object-map
updates in one checkpoint, preserves object identity and rejects moves into
self or descendants; its every-write/every-flush matrix accepts only the
complete pre- or post-rename namespace. Link counts are transactional; the
first unlink preserves shared data and the final unlink retires it.

The typed extent-map adapter followed: small contiguous files keep the
zero-extra-tree direct representation; fragmented, sparse or preallocated
files set an experimental object flag and use an owner-bound `AFST` tree keyed
by logical block. `write_file_at` uses full data COW and supports writes beyond
EOF, `truncate_file` preserves zero-tail semantics, `preallocate_file` creates
zero-reading unwritten mappings, and final unlink retires both extent-tree and
mapped data blocks. A dedicated matrix proves sparse range writes recover to
exactly the pre- or post-transaction content.

The multi-upsert overlay retains every dirty node in RAM by default but has a
constrained mode with a 2/4/8-page final-image LRU that spills to
allocated-but-unreachable blocks; all three budgets pass a multi-level mutation
test, and a 100,000-key eight-page structural qualification passes (about 8.1 s
optimised, 53 s debug, so it is an explicit ignored scale test). This did not
close the classic-memory gate: decoded recursive ancestors and the split peer
stayed outside the measured staged-image budget.

ADR-035 fixed the allocation-root self-reference direction: `AFST` nodes in a
permanently allocated `3N` pool for an `N`-node fixed-topology region tree,
leaving a complete writable generation while two checkpoints stay selectable.
`TreeAllocator` decoupled the COW engine from ordinary free-space allocation;
inline checkpoint records became empty. A 1 TiB geometry bulk-build creates
1,024 typed records; a sparse 1 TiB image formats, bounded-mounts, performs two
small commits and runs the exhaustive checker in about 3.51 s on the
Apple-Silicon/APFS development host (commits about 18.6/12.0 ms, checker about
2.34 s), with host physical allocation between about 112 MiB and 2.13 GiB, so
the test uses a relative < 1 % sparse bound. Allocation-root publication
upserts only dirty region records (at most three tree nodes per measured
commit); the 145-region boundary forces a two-level allocation root and has an
exhaustive crash matrix; checker bitmap equality compares a sparse accounted
set against set bits byte-wise.

The next actions recorded at that point were: prototype multi-page region
bitmaps and larger regions against the one-page baseline, and measure the
fsync checkpoint path before deciding whether an auxiliary durability log is
justified — both done later the same week
([fsync-intent-log-baseline.md](implementation/fsync-intent-log-baseline.md),
[ADR-036](adr/ADR-036-reclaim-queue.md), [ADR-037](adr/ADR-037-intent-log.md)).

## 2026-08-29 — Lesson from the first crash matrix, and the hardening list

(Moved from `CODEX_HANDOVER.md` § 6 and § 7.)

Because the bootstrap allocator never reused blocks, an incorrectly ordered
commit could sometimes be hidden by doing a full reachable-state validation at
mount and falling back to the old checkpoint. That is dangerous: the crash
harness appears to prove ordering correctness while mount is repairing around
an invalid commit protocol. `validate_checkpoint_reachable()` also walked
every object reachable from the checkpoint; used as the ordinary selection
path it would turn mounting a volume with millions of files into a full-volume
scan. The design direction that followed — bounded structural checkpoint
selection at mount, full reachable validation only in `afsplus-check`, shadow
verification, the test harness and explicit recovery modes; torn or
CRC-failing checkpoints as the only legitimate fallback — is the rule the
reviewer checklist in [`AGENTS.md`](AGENTS.md) carries.

The hardening list executed before the allocator work: separate bounded
checkpoint selection from full reachable validation and add the negative
mis-ordered-commit crash test; make `Timespec::read()` and every public decode
helper reject short buffers instead of panicking; checked arithmetic for
generation, next free block, next object ID and range arithmetic; two distinct
checkpoint slots claiming the same generation are ambiguous, not tie-broken;
the power-cut model is described exactly as what it enumerates (all
full-write subsets plus representative torn writes of the modeled tail);
directory entries enforce `entry.key == comparison_key(entry.name)`; an object
in the authoritative object map with zero namespace references fails
validation until an explicit orphan model exists.

## 2026-08-29 — First executable prototype

(Moved from `CODEX_HANDOVER.md` § 5; commit `74b1410`.)

Steps 1–5 of
[implementation/peer-review-prototype-plan.md](implementation/peer-review-prototype-plan.md)
became executable and green: 29 tests, zero clippy warnings, about 3,592
lines, `afsplus-format` building as `no_std + alloc` on
`aarch64-unknown-none`. `afsplus-format` held the prototype wire codecs
(explicit little-endian, no native-struct serialisation, CRC32C, a 32-byte
common metadata header with type, version, owner, generation, payload length
and checksum, bounds-first decoding, a normalisation-preserving directory
entry shape with an identity comparison-key encoder). `afsplus-block` held
the block provider plus memory, sparse file, trace/accounting, deterministic
fault and power-cut recording backends. `afsplus-core` held mkfs, the
identification record, alternating checkpoints A/B, bootstrap bump allocation
from a checkpoint high-water mark (scaffolding that never reused blocks), the
root object, root directory and object map, the first COW metadata
transaction (create empty file: four fresh metadata blocks, flush, alternate
checkpoint, flush) and the shared reachable-state validator. `afsplus-check`
was a verify-only checker reporting both checkpoint slots as text and
versioned JSON.
