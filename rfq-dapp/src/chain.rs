//! Connecting to a chain.

use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use alloy::transports::http::reqwest::Url;
use eyre::{eyre, Result};

use crate::config::{env_var, Network};

/// A read-only connection.
pub async fn connect_read(network: Network) -> Result<(DynProvider, u64)> {
    let provider = ProviderBuilder::new().connect_http(rpc_url(network)?).erased();
    let chain_id = provider.get_chain_id().await?;
    check_chain_id(network, chain_id)?;
    Ok((provider, chain_id))
}

/// A connection that can send transactions.
pub async fn connect_write(
    network: Network,
    signer: PrivateKeySigner,
) -> Result<(DynProvider, u64)> {
    let provider = ProviderBuilder::new()
        .wallet(signer)
        .connect_http(rpc_url(network)?)
        .erased();
    let chain_id = provider.get_chain_id().await?;
    check_chain_id(network, chain_id)?;
    Ok((provider, chain_id))
}

fn rpc_url(network: Network) -> Result<Url> {
    let raw = env_var(network.rpc_var())?;
    raw.parse()
        .map_err(|e| eyre!("{} is not a valid URL: {e}", network.rpc_var()))
}

/// Catches an RPC URL pointed at the wrong chain, which otherwise shows up as
/// a signature that will not verify or a contract that is not there.
fn check_chain_id(network: Network, actual: u64) -> Result<()> {
    match network.expected_chain_id() {
        Some(expected) if expected != actual => Err(eyre!(
            "{} is configured for {network}, which is chain id {expected}, \
             but the node reports {actual}",
            network.rpc_var()
        )),
        _ => Ok(()),
    }
}
