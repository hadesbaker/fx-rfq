//! An off-chain RFQ auction, settled on chain.
//!
//! `auction` needs no network. Everything else talks to whichever chain
//! `--network` selects.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use alloy::signers::local::PrivateKeySigner;
use clap::{Parser, Subcommand};
use eyre::{eyre, Result};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use rfq_dapp::chain::{connect_read, connect_write};
use rfq_dapp::config::{env_var, Network, Settings};
use rfq_dapp::contracts::{MockFixingOracle, MockUSDC, RfqSettlement};
use rfq_dapp::quote::{self, pair_to_bytes32, QuoteParams};
use rfq_engine::dealer::{Dealer, MockDealer};
use rfq_engine::engine::{best_quote, request_quotes};
use rfq_engine::types::{DealerQuote, Market, QuoteRequest, Side, PIP};

const COLLATERAL_DECIMALS: u8 = 6;

#[derive(Parser)]
#[command(about = "Off-chain RFQ auction with on-chain settlement")]
struct Cli {
    /// local or sepolia. Defaults to config.toml.
    #[arg(long, short)]
    network: Option<String>,

    #[arg(long, default_value = "config.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Network, chain id, block height and deployed addresses.
    Info,
    /// Run the auction off chain. No network needed.
    Auction,
    /// Run the auction, then sign the winning price as the dealer.
    Quote,
    /// Run the auction, sign, and submit the signature on chain.
    Open,
    /// List positions recorded by the settlement contract.
    Positions,
    /// Mint mock collateral to the dealer and the client and approve the
    /// settlement contract.
    Fund,
    /// Read a corridor's published fixing from the oracle.
    Fixing {
        /// Defaults to the pair in config.toml.
        pair: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    let cli = Cli::parse();
    let settings = Settings::load(&cli.config)?;
    let network = Network::parse(
        cli.network.as_deref().unwrap_or(&settings.default_network),
    )?;

    match cli.command {
        Command::Info => info(&settings, network).await,
        Command::Auction => {
            auction(&settings).await?;
            Ok(())
        }
        Command::Quote => show_quote(&settings, network).await,
        Command::Open => open(&settings, network).await,
        Command::Positions => positions(&settings, network).await,
        Command::Fund => fund(&settings, network).await,
        Command::Fixing { pair } => fixing(&settings, network, pair).await,
    }
}

// ----------------------------------------------------------------- commands

async fn info(settings: &Settings, network: Network) -> Result<()> {
    let d = settings.deployment(network);
    println!("\nnetwork            {network}");
    println!("rpc from           {}", network.rpc_var());
    println!("settlement         {}", fmt_addr(d.settlement));
    println!("collateral token   {}", fmt_addr(d.collateral_token));
    println!("fixing oracle      {}", fmt_addr(d.oracle));

    match connect_read(network).await {
        Ok((provider, chain_id)) => {
            println!("chain id           {chain_id}");
            println!("block              {}", provider.get_block_number().await?);
        }
        Err(e) => println!("chain              unreachable: {e}"),
    }
    println!();
    Ok(())
}

/// Runs the off-chain auction and returns the winner.
async fn auction(settings: &Settings) -> Result<(DealerQuote, usize, usize)> {
    let m = &settings.market;
    let a = &settings.auction;

    let market = Market { spot: m.spot, r_base: m.r_base, r_quote: m.r_quote };
    let req = QuoteRequest {
        pair: m.pair.clone(),
        notional: m.notional,
        tenor_days: m.tenor_days,
        side: Side::Buy,
    };

    let mut rng = StdRng::seed_from_u64(a.seed);
    let specs: Vec<(String, f64, u64)> = (0..a.dealers)
        .map(|i| {
            (
                format!("dealer-{:02}", i + 1),
                (rng.gen_range(5..60) as f64) / 10.0,
                rng.gen_range(30..420),
            )
        })
        .collect();

    let dealers: Vec<Arc<dyn Dealer>> = specs
        .iter()
        .map(|(n, s, l)| Arc::new(MockDealer::new(n, market, *s, *l)) as Arc<dyn Dealer>)
        .collect();

    println!("\nAUCTION");
    println!("  buy {} {:.0}, {} days", m.pair, m.notional, m.tenor_days);
    println!("  {} dealers, {}ms window", dealers.len(), a.window_ms);

    let quotes = request_quotes(&dealers, &req, Duration::from_millis(a.window_ms)).await;

    for (name, spread, latency) in &specs {
        match quotes.iter().find(|q| &q.dealer == name) {
            Some(q) => println!("    {name:<11} {spread:>5.1}p {latency:>5}ms   {:.6}", q.rate),
            None => println!("    {name:<11} {spread:>5.1}p {latency:>5}ms   missed the window"),
        }
    }

    let best = best_quote(&quotes, req.side)
        .ok_or_else(|| eyre!("no dealer answered out of {} solicited", dealers.len()))?
        .clone();

    let t = fx_forward_pricer::fx::year_fraction(m.tenor_days, 360);
    let fair = fx_forward_pricer::fx::forward_rate(m.spot, m.r_base, m.r_quote, t);
    println!(
        "  winner {} at {:.6}, {:.1} pips over fair ({} of {} answered)",
        best.dealer,
        best.rate,
        (best.rate - fair) / PIP,
        quotes.len(),
        dealers.len()
    );

    Ok((best, dealers.len(), quotes.len()))
}

async fn show_quote(settings: &Settings, network: Network) -> Result<()> {
    let (best, _, _) = auction(settings).await?;
    let (provider, chain_id) = connect_read(network).await?;
    let settlement = settings.deployment(network).require_settlement(network)?;

    let dealer: PrivateKeySigner = env_var("DEALER_PRIVATE_KEY")?.parse()?;
    let client: PrivateKeySigner = env_var("CLIENT_PRIVATE_KEY")?.parse()?;

    let q = quote::build(&params(settings, &best, &dealer, &client)?)?;
    let local = quote::digest(&q, chain_id, settlement);
    let signature = quote::sign(&q, &dealer, chain_id, settlement).await?;

    // The contract computes the same digest from the same struct. If these
    // ever disagree, the bindings have drifted from the deployed code.
    let onchain = RfqSettlement::new(settlement, &provider).hashQuote(q.clone()).call().await?;

    println!("\nSIGNED QUOTE");
    println!("  dealer      {}", dealer.address());
    println!("  client      {}", client.address());
    println!("  rate        {}", q.rate);
    println!("  notional    {}", q.notional);
    println!("  collateral  {} per side", q.collateral);
    println!("  expiry      {}", q.expiry);
    println!("  nonce       {}", q.nonce);
    println!("  digest      {local}");
    println!("  on chain    {onchain}");
    println!("  match       {}", if local == onchain { "yes" } else { "NO, bindings have drifted" });
    println!("  signature   0x{}\n", hex(&signature));
    Ok(())
}

async fn open(settings: &Settings, network: Network) -> Result<()> {
    let (best, _, _) = auction(settings).await?;

    let dealer: PrivateKeySigner = env_var("DEALER_PRIVATE_KEY")?.parse()?;
    let client: PrivateKeySigner = env_var("CLIENT_PRIVATE_KEY")?.parse()?;

    // The client is the only address the contract will accept the quote from.
    let (provider, chain_id) = connect_write(network, client.clone()).await?;
    let settlement = settings.deployment(network).require_settlement(network)?;

    let q = quote::build(&params(settings, &best, &dealer, &client)?)?;
    let signature = quote::sign(&q, &dealer, chain_id, settlement).await?;

    println!("\nSUBMITTING");
    println!("  settlement  {settlement}");
    let rfq = RfqSettlement::new(settlement, &provider);
    let receipt = rfq
        .open(q.clone(), signature.into())
        .send()
        .await?
        .get_receipt()
        .await?;

    println!("  tx          {}", receipt.transaction_hash);
    println!("  gas used    {}", receipt.gas_used);

    let count = rfq.positionCount().call().await?;
    let p = rfq.positions(count - U256::from(1)).call().await?;
    println!("\nPOSITION {}", count - U256::from(1));
    println!("  dealer      {}", p.dealer);
    println!("  client      {}", p.client);
    println!("  rate        {}", p.rate);
    println!("  notional    {}", p.notional);
    println!("  collateral  {} per side", p.collateral);
    println!("  fixing date {}", p.fixingDate);
    println!("  settled     {}\n", p.settled);
    Ok(())
}

async fn positions(settings: &Settings, network: Network) -> Result<()> {
    let (provider, _) = connect_read(network).await?;
    let settlement = settings.deployment(network).require_settlement(network)?;
    let rfq = RfqSettlement::new(settlement, &provider);

    let count = rfq.positionCount().call().await?;
    println!("\n{count} position(s) at {settlement}");
    let mut i = U256::ZERO;
    while i < count {
        let p = rfq.positions(i).call().await?;
        let pair = String::from_utf8_lossy(p.pair.as_slice())
            .trim_end_matches('\0')
            .to_string();
        println!(
            "  [{i}] {pair}  side {}  rate {}  notional {}  fixing {}  settled {}",
            p.side, p.rate, p.notional, p.fixingDate, p.settled
        );
        i += U256::from(1);
    }
    println!();
    Ok(())
}

/// Mints collateral to both sides and approves the settlement contract.
///
/// The dealer approving in advance is the on-chain shape of a dealer posting
/// margin before it is allowed to quote: the contract pulls from both sides
/// when the position opens, because it is counterparty to both.
async fn fund(settings: &Settings, network: Network) -> Result<()> {
    let d = settings.deployment(network);
    let settlement = d.require_settlement(network)?;
    if d.collateral_token.is_zero() {
        return Err(eyre!("no collateral_token address for {network} in config.toml"));
    }

    let deployer: PrivateKeySigner = env_var("DEPLOYER_PRIVATE_KEY")?.parse()?;
    let dealer: PrivateKeySigner = env_var("DEALER_PRIVATE_KEY")?.parse()?;
    let client: PrivateKeySigner = env_var("CLIENT_PRIVATE_KEY")?.parse()?;

    // Ten million whole tokens each.
    let amount = U256::from(10_000_000u64) * U256::from(10u64).pow(U256::from(COLLATERAL_DECIMALS));

    let (minter, _) = connect_write(network, deployer).await?;
    let token = MockUSDC::new(d.collateral_token, &minter);
    for who in [dealer.address(), client.address()] {
        token.mint(who, amount).send().await?.watch().await?;
        println!("  minted {amount} to {who}");
    }

    for signer in [dealer, client] {
        let who = signer.address();
        let (p, _) = connect_write(network, signer).await?;
        MockUSDC::new(d.collateral_token, &p)
            .approve(settlement, U256::MAX)
            .send()
            .await?
            .watch()
            .await?;
        println!("  {who} approved the settlement contract");
    }
    println!();
    Ok(())
}

async fn fixing(settings: &Settings, network: Network, pair: Option<String>) -> Result<()> {
    let (provider, _) = connect_read(network).await?;
    let d = settings.deployment(network);
    if d.oracle.is_zero() {
        return Err(eyre!("no oracle address for {network} in config.toml"));
    }
    let pair = pair.unwrap_or_else(|| settings.market.pair.clone());
    let key = pair_to_bytes32(&pair)?;

    let (rate, published_at) = {
        let o = MockFixingOracle::new(d.oracle, &provider).latestFixing(key).call().await?;
        (o._0, o._1)
    };

    println!("\nFIXING {pair}");
    if published_at == 0 {
        println!("  nothing published for this corridor\n");
    } else {
        println!("  rate          {rate}");
        println!("  published at  {published_at}\n");
    }
    Ok(())
}

// ------------------------------------------------------------------ helpers

fn params(
    settings: &Settings,
    best: &DealerQuote,
    dealer: &PrivateKeySigner,
    client: &PrivateKeySigner,
) -> Result<QuoteParams> {
    let m = &settings.market;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;

    Ok(QuoteParams {
        dealer: dealer.address(),
        client: client.address(),
        pair: m.pair.clone(),
        side: 0,
        notional: m.notional,
        rate: best.rate,
        tenor_days: m.tenor_days,
        collateral: m.collateral,
        collateral_decimals: COLLATERAL_DECIMALS,
        ttl_secs: settings.auction.ttl_secs,
        nonce,
    })
}

fn fmt_addr(a: Address) -> String {
    if a.is_zero() { "not deployed".to_string() } else { a.to_string() }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
