# SPDX-License-Identifier: BSD-2-Clause
#
# Convenience entry points. The build itself is the Cargo workspace under
# crates/; this file only names the repository-level gates.

PYTHON ?= python3

.PHONY: check check-docs toc adr-index rust-gate

## check: every repository gate (Rust quality gate + documentation)
check: rust-gate check-docs

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
