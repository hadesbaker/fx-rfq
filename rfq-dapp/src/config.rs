//! Configuration: non-secret settings from `config.toml`, secrets from `.env`.
//!
//! The split is deliberate. Deployed contract addresses are public and belong
//! in a committed file. RPC URLs carry provider API keys and private keys are
//! private keys, so both live in `.env`, which is gitignored.

use std::fmt;
use std::path::Path;

use alloy::primitives::Address;
use eyre::{eyre, Context, Result};
use serde::Deserialize;

/// Which chain to talk to.
///
/// There is deliberately no mainnet. These contracts are unaudited and the
/// collateral token lets anyone mint, so nothing here belongs on a chain where
/// a mistake costs money.
///
/// Working against real chain state does not require one. A fork inherits the
/// forked chain's id and is reached as [`Network::Local`]:
///
/// ```sh
/// anvil --fork-url https://...
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    /// Anvil, whether standalone or forking another chain.
    Local,
    Sepolia,
}

impl Network {
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "local" => Ok(Network::Local),
            "sepolia" => Ok(Network::Sepolia),
            other => Err(eyre!("unknown network {other:?}, expected local or sepolia")),
        }
    }

    /// The env var holding this network's RPC URL.
    pub fn rpc_var(&self) -> &'static str {
        match self {
            Network::Local => "LOCAL_RPC_URL",
            Network::Sepolia => "SEPOLIA_RPC_URL",
        }
    }

    /// The chain id this network must report, or `None` where any is accepted.
    ///
    /// Local accepts anything because a fork inherits the forked chain's id.
    /// Sepolia is pinned, which catches a `SEPOLIA_RPC_URL` pointed somewhere
    /// else. That otherwise surfaces as a signature that will not verify, or a
    /// contract that is not there.
    pub fn expected_chain_id(&self) -> Option<u64> {
        match self {
            Network::Local => None,
            Network::Sepolia => Some(11_155_111),
        }
    }
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Network::Local => "local",
            Network::Sepolia => "sepolia",
        };
        f.write_str(s)
    }
}

/// Deployed addresses for one network. Zero means not deployed yet.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Deployment {
    #[serde(default)]
    pub settlement: Address,
    #[serde(default)]
    pub collateral_token: Address,
    #[serde(default)]
    pub oracle: Address,
}

impl Deployment {
    pub fn require_settlement(&self, network: Network) -> Result<Address> {
        if self.settlement.is_zero() {
            return Err(eyre!(
                "no settlement address for {network}. Deploy first, then set \
                 networks.{network}.settlement in config.toml"
            ));
        }
        Ok(self.settlement)
    }
}

/// The auction to run off chain before anything touches a chain.
#[derive(Debug, Clone, Deserialize)]
pub struct AuctionConfig {
    pub dealers: usize,
    pub window_ms: u64,
    pub ttl_secs: u64,
    /// Fixed so a run reproduces. Change it and a different dealer wins.
    pub seed: u64,
}

/// The request and the market the mock dealers price it against.
#[derive(Debug, Clone, Deserialize)]
pub struct MarketConfig {
    pub pair: String,
    pub spot: f64,
    pub r_base: f64,
    pub r_quote: f64,
    pub tenor_days: u32,
    /// Base currency units, human readable. Scaled by 1e18 on chain.
    pub notional: f64,
    /// Collateral required from each side, in whole collateral tokens.
    pub collateral: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub default_network: String,
    pub auction: AuctionConfig,
    pub market: MarketConfig,
    pub networks: Networks,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Networks {
    #[serde(default)]
    pub local: Deployment,
    #[serde(default)]
    pub sepolia: Deployment,
}

impl Settings {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("cannot read {}", path.display()))?;
        toml::from_str(&raw).wrap_err_with(|| format!("cannot parse {}", path.display()))
    }

    pub fn deployment(&self, network: Network) -> Deployment {
        match network {
            Network::Local => self.networks.local,
            Network::Sepolia => self.networks.sepolia,
        }
    }
}

/// Reads a required environment variable, naming it in the error.
pub fn env_var(key: &str) -> Result<String> {
    std::env::var(key).map_err(|_| eyre!("{key} is not set. Copy .env.example to .env and fill it in"))
}
