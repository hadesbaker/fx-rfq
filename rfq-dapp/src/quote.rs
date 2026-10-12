//! Building and signing the EIP-712 quote the contract verifies.
//!
//! The dealer signs off chain, so nothing about the auction costs gas and the
//! price is given at network speed rather than block speed. What the signature
//! buys is that the dealer cannot later deny a price it gave, and cannot be
//! bound to one it did not.

use alloy::primitives::{utils::parse_units, Address, FixedBytes, B256, U256};
use alloy::signers::Signer;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol_types::{eip712_domain, Eip712Domain, SolStruct};
use eyre::{eyre, Result};

use crate::contracts::RfqSettlement::Quote;

/// Must match the domain the contract builds in its constructor.
pub fn domain(chain_id: u64, settlement: Address) -> Eip712Domain {
    eip712_domain! {
        name: "CRX RFQ Settlement",
        version: "1",
        chain_id: chain_id,
        verifying_contract: settlement,
    }
}

/// A corridor identifier as the contract stores it: ASCII, right-padded.
pub fn pair_to_bytes32(pair: &str) -> Result<FixedBytes<32>> {
    let bytes = pair.as_bytes();
    if bytes.len() > 32 {
        return Err(eyre!("pair {pair:?} does not fit in 32 bytes"));
    }
    let mut out = [0u8; 32];
    out[..bytes.len()].copy_from_slice(bytes);
    Ok(FixedBytes::from(out))
}

/// Scales a human figure to fixed-point integer units.
///
/// Via a decimal string rather than multiplying: `5.544602 * 1e18` exceeds the
/// range in which an `f64` holds integers exactly, so the product would be
/// wrong in its low digits.
pub fn scale(value: f64, decimals: u8) -> Result<U256> {
    if !value.is_finite() || value < 0.0 {
        return Err(eyre!("{value} is not a scalable amount"));
    }

    let text = format!("{value:.12}");
    let scaled: U256 = parse_units(&text, decimals)?.into();

    // Rejected rather than returned, because a collateral figure below the
    // token's smallest unit truncates to zero, and the contract skips the
    // transfer entirely when collateral is zero. Silently opening an
    // unmargined position is worse than failing.
    if scaled.is_zero() && value > 0.0 {
        return Err(eyre!(
            "{value} rounds to zero at {decimals} decimals: below the token's smallest unit"
        ));
    }

    Ok(scaled)
}

pub struct QuoteParams {
    pub dealer: Address,
    pub client: Address,
    pub pair: String,
    /// 0 = client buys the base currency, 1 = client sells it.
    pub side: u8,
    /// Base currency units, human readable.
    pub notional: f64,
    /// Quote units per base unit, human readable.
    pub rate: f64,
    pub tenor_days: u32,
    /// Collateral required from each side, in whole tokens.
    pub collateral: f64,
    pub collateral_decimals: u8,
    /// Seconds from now until the price dies.
    pub ttl_secs: u64,
    pub nonce: u64,
}

pub fn build(p: &QuoteParams) -> Result<Quote> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();

    Ok(Quote {
        dealer: p.dealer,
        client: p.client,
        pair: pair_to_bytes32(&p.pair)?,
        side: p.side,
        notional: scale(p.notional, 18)?,
        rate: scale(p.rate, 18)?,
        tenorDays: p.tenor_days,
        collateral: scale(p.collateral, p.collateral_decimals)?,
        expiry: now + p.ttl_secs,
        nonce: U256::from(p.nonce),
    })
}

/// The digest the dealer signs, computed locally.
pub fn digest(quote: &Quote, chain_id: u64, settlement: Address) -> B256 {
    quote.eip712_signing_hash(&domain(chain_id, settlement))
}

/// Signs as the dealer. Returns the 65 byte `r || s || v` the contract expects.
pub async fn sign(
    quote: &Quote,
    signer: &PrivateKeySigner,
    chain_id: u64,
    settlement: Address,
) -> Result<Vec<u8>> {
    let d = digest(quote, chain_id, settlement);
    let signature = signer.sign_hash(&d).await?;
    Ok(signature.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_handles_the_configured_values() {
        assert_eq!(scale(5.544602, 18).unwrap().to_string(), "5544602000000000000");
        assert_eq!(scale(10_000_000.0, 18).unwrap().to_string(), "10000000000000000000000000");
        assert_eq!(scale(500_000.0, 6).unwrap().to_string(), "500000000000");
    }

    #[test]
    fn scale_refuses_to_truncate_to_zero() {
        // A 6-decimal token cannot hold this, and returning zero would open an
        // unmargined position without saying so.
        assert!(scale(0.0000001234, 6).is_err());
        assert!(scale(0.000001, 6).is_ok());
    }

    #[test]
    fn scale_rejects_nonsense() {
        assert!(scale(f64::NAN, 18).is_err());
        assert!(scale(-1.0, 18).is_err());
        assert!(scale(0.0, 18).is_ok(), "zero is a legitimate amount");
    }
}
