//! FX forward pricing and quote construction.
//!
//! [`fx`] prices outright forwards from covered interest parity, converts
//! them into the forward points dealers quote, and builds broken dates by
//! interpolating a curve.
//!
//! The binary runs one client request end to end: a four month hedge
//! priced off a 3M/6M curve.

pub mod fx;
