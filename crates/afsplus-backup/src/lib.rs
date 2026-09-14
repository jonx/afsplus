//! Filesystem-neutral archive primitives for the ADR-076 backup consumer.
//! Framing alone does not establish archive integrity. Envelope receipts verify
//! stream integrity/termination; preservation semantics require profile validation.
pub mod envelope;
pub mod pax;
pub mod tar;

pub mod member;

pub mod stream;

pub mod metadata;

#[cfg(feature = "consumer")]
pub mod attachment;
