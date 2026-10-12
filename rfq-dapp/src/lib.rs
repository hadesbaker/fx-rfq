//! An off-chain RFQ auction settled on chain.
//!
//! The auction itself comes from [`rfq_engine`] and the prices from
//! [`fx_forward_pricer`]. This crate adds the chain: the winning dealer signs
//! its quote under EIP-712, and the client submits that signature to a
//! contract that verifies it, checks the expiry and a nonce, and records the
//! position with collateral from both sides.

pub mod chain;
pub mod config;
pub mod contracts;
pub mod quote;
