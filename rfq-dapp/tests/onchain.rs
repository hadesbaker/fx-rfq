//! End to end against a spawned anvil node.
//!
//! The test that matters most is `digest_matches_the_contract`. The Rust
//! bindings and the Solidity both compute an EIP-712 digest over the same
//! struct, and if the two definitions ever diverge the digests differ and no
//! signature verifies. Comparing them directly turns a silent integration
//! failure into a failing test.

use alloy::node_bindings::Anvil;
use alloy::primitives::{Address, U256};
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy::transports::http::reqwest::Url;

use rfq_dapp::contracts::{MockFixingOracle, MockUSDC, RfqSettlement};
use rfq_dapp::quote::{self, QuoteParams};

const PAIR: &str = "USD/BRL";
const COLLATERAL_DECIMALS: u8 = 6;

struct Fixture {
    _anvil: alloy::node_bindings::AnvilInstance,
    url: Url,
    chain_id: u64,
    dealer: PrivateKeySigner,
    client: PrivateKeySigner,
    settlement: Address,
    usdc: Address,
}

async fn deploy() -> eyre::Result<Fixture> {
    let anvil = Anvil::new().spawn();
    let url = anvil.endpoint_url();
    let chain_id = anvil.chain_id();

    let deployer: PrivateKeySigner = anvil.keys()[0].clone().into();
    let dealer: PrivateKeySigner = anvil.keys()[1].clone().into();
    let client: PrivateKeySigner = anvil.keys()[2].clone().into();

    let provider = ProviderBuilder::new()
        .wallet(deployer)
        .connect_http(url.clone());

    let usdc = MockUSDC::deploy(&provider).await?;
    let oracle = MockFixingOracle::deploy(&provider).await?;
    let settlement = RfqSettlement::deploy(&provider, *usdc.address(), *oracle.address()).await?;

    // Fund and approve both sides.
    for who in [dealer.address(), client.address()] {
        usdc.mint(who, U256::from(10_000_000_000_000u64)).send().await?.watch().await?;
    }
    for signer in [dealer.clone(), client.clone()] {
        let p = ProviderBuilder::new().wallet(signer).connect_http(url.clone());
        MockUSDC::new(*usdc.address(), &p)
            .approve(*settlement.address(), U256::MAX)
            .send()
            .await?
            .watch()
            .await?;
    }

    Ok(Fixture {
        url,
        chain_id,
        dealer,
        client,
        settlement: *settlement.address(),
        usdc: *usdc.address(),
        _anvil: anvil,
    })
}

fn params(f: &Fixture, nonce: u64) -> QuoteParams {
    QuoteParams {
        dealer: f.dealer.address(),
        client: f.client.address(),
        pair: PAIR.to_string(),
        side: 0,
        notional: 10_000_000.0,
        rate: 5.544602,
        tenor_days: 122,
        collateral: 500_000.0,
        collateral_decimals: COLLATERAL_DECIMALS,
        ttl_secs: 60,
        nonce,
    }
}

#[tokio::test]
async fn domain_separator_matches_the_contract() -> eyre::Result<()> {
    let f = deploy().await?;
    let provider = ProviderBuilder::new().connect_http(f.url.clone());
    let onchain = RfqSettlement::new(f.settlement, &provider).domainSeparator().call().await?;

    let local = quote::domain(f.chain_id, f.settlement).separator();
    assert_eq!(onchain, local, "Rust and Solidity disagree on the EIP-712 domain");
    Ok(())
}

#[tokio::test]
async fn digest_matches_the_contract() -> eyre::Result<()> {
    let f = deploy().await?;
    let q = quote::build(&params(&f, 1))?;

    let provider = ProviderBuilder::new().connect_http(f.url.clone());
    let onchain = RfqSettlement::new(f.settlement, &provider)
        .hashQuote(q.clone())
        .call()
        .await?;

    let local = quote::digest(&q, f.chain_id, f.settlement);
    assert_eq!(onchain, local, "Rust and Solidity disagree on the EIP-712 digest");
    Ok(())
}

#[tokio::test]
async fn a_signed_quote_opens_a_position() -> eyre::Result<()> {
    let f = deploy().await?;
    let q = quote::build(&params(&f, 1))?;
    let sig = quote::sign(&q, &f.dealer, f.chain_id, f.settlement).await?;

    let client_provider = ProviderBuilder::new()
        .wallet(f.client.clone())
        .connect_http(f.url.clone());
    let rfq = RfqSettlement::new(f.settlement, &client_provider);

    rfq.open(q.clone(), sig.clone().into()).send().await?.watch().await?;

    assert_eq!(rfq.positionCount().call().await?, U256::from(1));

    let p = rfq.positions(U256::ZERO).call().await?;
    assert_eq!(p.dealer, f.dealer.address());
    assert_eq!(p.client, f.client.address());
    assert_eq!(p.notional, q.notional);
    assert_eq!(p.rate, q.rate);
    assert!(!p.settled);

    // Collateral from both sides is now held by the contract.
    let usdc = MockUSDC::new(f.usdc, &client_provider);
    assert_eq!(usdc.balanceOf(f.settlement).call().await?, q.collateral * U256::from(2));
    Ok(())
}

#[tokio::test]
async fn the_same_signature_cannot_be_replayed() -> eyre::Result<()> {
    let f = deploy().await?;
    let q = quote::build(&params(&f, 7))?;
    let sig = quote::sign(&q, &f.dealer, f.chain_id, f.settlement).await?;

    let p = ProviderBuilder::new()
        .wallet(f.client.clone())
        .connect_http(f.url.clone());
    let rfq = RfqSettlement::new(f.settlement, &p);

    rfq.open(q.clone(), sig.clone().into()).send().await?.watch().await?;
    let second = rfq.open(q, sig.into()).send().await;
    assert!(second.is_err(), "a consumed nonce must not open a second position");
    Ok(())
}

#[tokio::test]
async fn a_quote_signed_by_someone_else_is_refused() -> eyre::Result<()> {
    let f = deploy().await?;
    let q = quote::build(&params(&f, 1))?;
    // The client signs its own quote and claims the dealer gave it.
    let sig = quote::sign(&q, &f.client, f.chain_id, f.settlement).await?;

    let p = ProviderBuilder::new()
        .wallet(f.client.clone())
        .connect_http(f.url.clone());
    let result = RfqSettlement::new(f.settlement, &p).open(q, sig.into()).send().await;
    assert!(result.is_err());
    Ok(())
}
