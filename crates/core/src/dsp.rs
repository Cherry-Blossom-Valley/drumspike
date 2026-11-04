//! Filter and statistics primitives shared by the detection rules and the exports (refactor R7).
//!
//! Every function here keeps the exact float operations of the copy it replaces: the `to_bits`
//! tests in `dsp/tests.rs` compare each one with the former copies bit for bit.

#[cfg(test)]
mod tests;
