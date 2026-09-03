# Rust Codec Fuzz Regressions

Copy any failing `.afrf` artifact here after minimizing or confirming it.
Committed artifacts are replayed by `tools/check-rust-codec-fuzz.sh` on every
repository gate. The artifact stores its schema version, seed version, codec
target, stable case number and exact input bytes; never replace an existing
artifact with different bytes under the same name.
