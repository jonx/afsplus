# SPDX-License-Identifier: BSD-2-Clause
#
# Convenience entry points. The build itself is the Cargo workspace under
# crates/; this file only names the repository-level gates.

PYTHON ?= python3

.PHONY: check check-docs toc adr-index rust-gate portable-c-gate

## check: every repository gate (Rust quality gate + documentation)
check: rust-gate portable-c-gate check-docs

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

## portable-c-gate: compile the independent C99 reader and cross-read a Rust image
portable-c-gate:
	tools/check-portable-c-reader.sh
