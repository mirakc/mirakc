// Took from https://raw.githubusercontent.com/tokio-rs/axum/refs/tags/axum-extra-v0.12.3/axum-extra/src/extract/host.rs
//
// The `Host` extractor has been removed in v0.12.5.
// See https://github.com/tokio-rs/axum/issues/3442.
//
// Do not format host.rs in order to reduce differences from the original file.
#[rustfmt::skip]
mod host;

// Took from https://github.com/tokio-rs/axum/blob/axum-extra-v0.12.5/axum-extra/src/extract/host.rs
//
// Do not format rejection.rs in order to reduce differences from the original file.
#[rustfmt::skip]
mod rejection;

pub use host::Host;

#[cfg(test)]
use axum_macros::__private_axum_test as test;

#[cfg(test)]
use axum::test_helpers;
