# SPDX-License-Identifier: BSD-2-Clause
#
# Convenience entry points. The build itself is the Cargo workspace under
# crates/; this file only names the repository-level gates.

PYTHON ?= python3

.PHONY: check check-docs toc adr-index rust-gate rust-codec-fuzz-gate \
	portable-c-gate portable-c-fuzz-gate portable-c-fuzz-long

## check: every repository gate (Rust quality gate + documentation)
check: rust-gate rust-codec-fuzz-gate portable-c-gate portable-c-fuzz-gate check-docs

## check-docs: documentation contract (links, TOCs, indexes, references)
check-docs:
	$(PYTHON) tools/check-docs.py

## toc: regenerate every table of contents and the ADR index
toc adr-index:
	$(PYTHON) tools/check-docs.py --write

## rust-gate: the gate from CONTRIBUTING.md
rust-gate:
	cargo fmt --all -- --check
	cargo test --workspace --all-features
	cargo clippy --workspace --all-targets --all-features -- -D warnings

## rust-codec-fuzz-gate: deterministic format-codec mutations and artifact replay
rust-codec-fuzz-gate:
	tools/check-rust-codec-fuzz.sh

## portable-c-gate: compile the independent C99 reader and cross-read a Rust image
portable-c-gate:
	tools/check-portable-c-reader.sh

## portable-c-fuzz-gate: deterministic sanitizer mutations of compact C-reader seeds
portable-c-fuzz-gate:
	tools/check-portable-c-fuzz.sh

## portable-c-fuzz-long: longer local mutation run (override AFSPLUS_FUZZ_RUNS)
portable-c-fuzz-long:
	AFSPLUS_FUZZ_RUNS=$${AFSPLUS_FUZZ_RUNS:-100000} \
		tools/check-portable-c-fuzz.sh
