// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice Source of the published reference rate a non-deliverable forward
///         settles against: PTAX for BRL, the RBI reference rate for INR, the
///         KFTC rate for KRW.
///
/// @dev A central bank publishes one number per day. It is not a counterparty
///      and it is not in the payment path; the contract only points at what it
///      publishes. Bringing that number on chain is the hard part, and every
///      failure mode of an oracle applies: staleness, a single source, a local
///      holiday on which nothing prints, and manipulation of the observation
///      window.
///
///      `publishedAt` exists so a caller can reject a stale fixing rather than
///      settle against yesterday's number.
interface IFixingOracle {
    /// @param pair       corridor identifier, e.g. bytes32("USD/BRL")
    /// @return rate      quote units per base unit, scaled by 1e18
    /// @return publishedAt unix seconds at which this fixing was published
    function latestFixing(bytes32 pair) external view returns (uint256 rate, uint64 publishedAt);
}
