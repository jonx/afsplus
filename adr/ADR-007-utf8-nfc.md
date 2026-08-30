# ADR-007: UTF-8 names with NFC normalization

Status: Accepted for the executable prototype; epoch-1 interoperability validation pending

## Decision
AFS+ stores the original valid UTF-8 spelling. Its comparison key, not the
stored name, is normalized. Identification version 3 pins Unicode 16.0.0:
case-sensitive keys use NFC, while case-insensitive keys perform canonical
decomposition, full default case folding and NFC recomposition.

## Open validation
Test behavior with Rust, Git, macOS, Linux, Samba, and classic AROS before freezing epoch 1.
