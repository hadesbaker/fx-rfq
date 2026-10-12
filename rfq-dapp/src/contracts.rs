//! Rust bindings generated from the compiled contracts.
//!
//! The artifacts in `abi/` are produced by `forge build`, so the `Quote`
//! struct and every selector here come from the contract that was actually
//! compiled rather than from a hand-written interface that can drift out of
//! step with it.
//!
//! Regenerate after changing a contract:
//!
//! ```sh
//! cd contracts && forge build && cd ..
//! ./scripts/sync-abi.sh
//! ```

use alloy::sol;

// The generated bindings expand the Quote struct into a wide constructor,
// which trips the argument-count lint on code this crate does not write.
sol!(
    #[allow(missing_docs, clippy::too_many_arguments)]
    #[sol(rpc)]
    RfqSettlement,
    "abi/RfqSettlement.json"
);

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    MockUSDC,
    "abi/MockUSDC.json"
);

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    MockFixingOracle,
    "abi/MockFixingOracle.json"
);
