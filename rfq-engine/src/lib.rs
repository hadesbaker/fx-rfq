//! A request-for-quote engine.
//!
//! A client announces what it wants, every dealer is asked at the same time,
//! the responses that arrive inside the window compete, and the best price is
//! returned with an expiry.
//!
//! Fair value comes from [`fx_forward_pricer`]; this crate handles
//! distribution.

pub mod dealer;
pub mod engine;
pub mod types;
