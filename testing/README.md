# Test Plans Index

Each test plan states what a gate checks and which script or `cargo test`
target runs it. Test plans do not record results; the milestone a plan feeds
is tracked in [implementation/milestones.md](../implementation/milestones.md).
The qualification scripts themselves are listed in
[tools/README.md](../tools/README.md).

| Test plan | Qualifies | Gate | Feeds |
|---|---|---|---|
| [test-strategy.md](test-strategy.md) | The test pyramid: unit, property, portable-core, image, crash, fuzz, black-box handler, interoperability, application | `cargo test --workspace --all-features` | every milestone |
| [conformance.md](conformance.md) | Versioned reference images decode to their expected output | `cargo test -p afsplus-check --test basic --test hardening --test mount_modes` plus the dedicated corruption corpus | M00, M02, M05, M12 |
| [corruption-corpus.md](corruption-corpus.md) | Integrity and valid-CRC semantic damage on every implemented wire surface is rejected or bounded at an intentional log-tail warning | `cargo test -p afsplus-check --test corruption_corpus`; replay artifacts from `afsplus-corruption-corpus` | M05 |
| [crash-testing.md](crash-testing.md) | Every transaction boundary recovers to an allowed state under the modeled power-cut and fault model | `cargo test -p afsplus-check --test crash_matrix --test alloc_crash --test faults --test reclaim --test batch --test intent_log` | M03, M04 |
| [intent-log-write-truncate-qualification.md](intent-log-write-truncate-qualification.md) | Existing-file records preserve old/new atomicity, monotone prefixes, shared owners and restartable recovery | `cargo test -p afsplus-check --test intent_log --test shared_crash`; optimized workloads in `fsync_workloads` | M04, M14 |
| [shared-extents-qualification.md](shared-extents-qualification.md) | Reflink reference accounting, write-COW isolation, corruption rejection and every-write/every-flush crash recovery | `cargo test -p afsplus-check --test shared_extents` once ADR-061 is implemented | M03, M05, M14 |
| [fuzzing.md](fuzzing.md) | Every independent metadata parser survives mutated input | no fuzz harness in the workspace; parsers are bounds-first and covered by `cargo test -p afsplus-format` | M01 |
| [developer-harness.md](developer-harness.md) | A failure reproduces from a small artifact set (trace, image, seed, fault schedule) | `cargo test -p afsplus-block` (trace, fault and power-cut backends) | M05 |
| [benchmark-contract.md](benchmark-contract.md) | The metrics every benchmark reports: latency, CPU, RAM, I/O, flushes, amplification | `cargo test -p afsplus-check --test measurements --test fsync_workloads --release -- --ignored --nocapture` | M13, M14 |
| [data-policy-qualification.md](data-policy-qualification.md) | Full-COW versus private-in-place workloads and distinct crash oracles | `cargo test -p afsplus-check --test data_policy --release -- --ignored --nocapture` | M03, M14 |
| [performance-benchmarks.md](performance-benchmarks.md) | Metadata- and data-path benchmarks with correctness verification | `cargo test -p afsplus-check --test measurements --release -- --ignored --nocapture` | M09, M13 |
| [extreme-workload-benchmarks.md](extreme-workload-benchmarks.md) | Streaming, Git-scale tree and AI/LLM workloads | no dedicated harness; the Git ref-update rows of `fsync_workloads` are the first executable subset | M13 |
| [application-qualification.md](application-qualification.md) | Cargo, Git, Zed-style watcher, Ferail and Moonstone operate on AFS+ | `cargo test -p afsplus-check --test fsync_workloads --release -- --ignored --nocapture` (Git ref-update pattern); application runs are open | M13 |
| [aros-system-volume-qualification.md](aros-system-volume-qualification.md) | The S0–S3 AROS ladder: same-image mount, recovery replay, `SYS:` pivot, boot selection, repeated boot; AFS/FFS comparison | `tools/check-hosted-aros-*.sh`, `tools/check-macaros-native-*.sh`, `tools/check-aros-m68k-*.sh`, composite [`tools/check-mountable-alpha0.sh`](../tools/check-mountable-alpha0.sh) | M06, M08 |
| [security-model-conformance.md](security-model-conformance.md) | Identity round-trip, ACL evaluation and simple-host preservation before the security format freezes | no harness; the security container is a design proposal | M14 |
| [security-scanning-benchmarks.md](security-scanning-benchmarks.md) | Scan-once-per-content-generation cost against rescan baselines | no harness; depends on the change stream | M10 |
