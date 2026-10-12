//! Auction tests. Latencies are fixed, so results are reproducible.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rfq_engine::dealer::{Dealer, MockDealer};
use rfq_engine::engine::{best_quote, request_quotes, run_auction};
use rfq_engine::types::{DealerQuote, Market, Quote, QuoteRequest, RfqError, Side, PIP};

const MARKET: Market = Market { spot: 5.40, r_base: 0.04, r_quote: 0.12 };

/// Covered interest parity at 122 days on the market above.
const FAIR_122D: f64 = 5.544442008331506;

fn req(side: Side) -> QuoteRequest {
    QuoteRequest {
        pair: "USD/BRL".to_string(),
        notional: 10_000_000.0,
        tenor_days: 122,
        side,
    }
}

fn dealers(specs: &[(&str, f64, u64)]) -> Vec<Arc<dyn Dealer>> {
    specs
        .iter()
        .map(|(n, s, l)| Arc::new(MockDealer::new(n, MARKET, *s, *l)) as Arc<dyn Dealer>)
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// ---------- the dealer ----------

#[tokio::test]
async fn zero_spread_dealer_quotes_fair_value() {
    let d = MockDealer::new("flat", MARKET, 0.0, 0);
    assert!(close(d.quote(&req(Side::Buy)).await.rate, FAIR_122D));
    assert!(close(d.quote(&req(Side::Sell)).await.rate, FAIR_122D));
}

#[tokio::test]
async fn dealer_shades_against_the_client() {
    let d = MockDealer::new("edge", MARKET, 2.5, 0);

    let buy = d.quote(&req(Side::Buy)).await.rate;
    let sell = d.quote(&req(Side::Sell)).await.rate;

    // The client pays on a buy, so the dealer asks for more.
    assert!(buy > FAIR_122D, "buy {buy} should exceed fair {FAIR_122D}");
    assert!(close(buy, FAIR_122D + 2.5 * PIP), "buy was {buy}");

    // The client receives on a sell, so the dealer offers less.
    assert!(sell < FAIR_122D, "sell {sell} should be under fair {FAIR_122D}");
    assert!(close(sell, FAIR_122D - 2.5 * PIP), "sell was {sell}");
}

#[tokio::test]
async fn dealer_reports_its_own_name() {
    let d = MockDealer::new("alpha", MARKET, 1.0, 0);
    assert_eq!(d.quote(&req(Side::Buy)).await.dealer, "alpha");
}

// ---------- fanning out ----------

#[tokio::test]
async fn fan_out_runs_concurrently() {
    // Five dealers, 100ms each. In series that is 500ms.
    let ds = dealers(&[
        ("a", 1.0, 100),
        ("b", 2.0, 100),
        ("c", 3.0, 100),
        ("d", 4.0, 100),
        ("e", 5.0, 100),
    ]);

    let start = Instant::now();
    let quotes = request_quotes(&ds, &req(Side::Buy), Duration::from_millis(400)).await;
    let elapsed = start.elapsed();

    assert_eq!(quotes.len(), 5, "every dealer was inside the window");
    assert!(
        elapsed < Duration::from_millis(350),
        "took {elapsed:?}; sequential would be ~500ms, so this is not concurrent"
    );
}

#[tokio::test]
async fn slow_dealer_is_dropped() {
    let ds = dealers(&[("fast1", 1.0, 20), ("fast2", 2.0, 20), ("fast3", 3.0, 20), ("slow", 0.5, 500)]);

    let quotes = request_quotes(&ds, &req(Side::Buy), Duration::from_millis(200)).await;

    assert_eq!(quotes.len(), 3, "the 500ms dealer missed a 200ms window");
    assert!(!quotes.iter().any(|q| q.dealer == "slow"));
}

#[tokio::test]
async fn no_dealer_in_time_gives_an_empty_set() {
    let ds = dealers(&[("slow1", 1.0, 300), ("slow2", 2.0, 300)]);
    let quotes = request_quotes(&ds, &req(Side::Buy), Duration::from_millis(50)).await;
    assert!(quotes.is_empty());
}

// ---------- picking the winner ----------

fn q(name: &str, rate: f64) -> DealerQuote {
    DealerQuote { dealer: name.to_string(), rate, latency: Duration::from_millis(1) }
}

#[test]
fn buy_takes_the_lowest_rate() {
    let qs = [q("high", 5.5500), q("low", 5.5400), q("mid", 5.5450)];
    assert_eq!(best_quote(&qs, Side::Buy).unwrap().dealer, "low");
}

#[test]
fn sell_takes_the_highest_rate() {
    let qs = [q("high", 5.5500), q("low", 5.5400), q("mid", 5.5450)];
    assert_eq!(best_quote(&qs, Side::Sell).unwrap().dealer, "high");
}

#[test]
fn the_two_sides_disagree() {
    // The guard against picking one comparison for both cases.
    let qs = [q("x", 5.54), q("y", 5.56)];
    let buy = best_quote(&qs, Side::Buy).unwrap().dealer.clone();
    let sell = best_quote(&qs, Side::Sell).unwrap().dealer.clone();
    assert_ne!(buy, sell, "buy and sell cannot both win with the same dealer here");
}

#[test]
fn no_quotes_no_winner() {
    assert!(best_quote(&[], Side::Buy).is_none());
    assert!(best_quote(&[], Side::Sell).is_none());
}

// ---------- expiry ----------

#[test]
fn accept_inside_the_window_fills() {
    let quote = Quote::new(&q("alpha", 5.5442), Side::Buy, 5, 5, Duration::from_secs(10));
    let fill = quote.accept(10_000_000.0, Instant::now()).unwrap();
    assert_eq!(fill.dealer, "alpha");
    assert!(close(fill.rate, 5.5442));
    assert!(close(fill.notional, 10_000_000.0));
}

#[test]
fn accept_after_expiry_is_refused() {
    let ttl = Duration::from_millis(500);
    let quote = Quote::new(&q("alpha", 5.5442), Side::Buy, 5, 5, ttl);

    // No sleeping: `now` is an argument, so the future is reachable directly.
    let late = quote.issued_at + Duration::from_secs(2);
    match quote.accept(10_000_000.0, late) {
        Err(RfqError::QuoteExpired { ttl: t, .. }) => assert_eq!(t, ttl),
        other => panic!("expected QuoteExpired, got {other:?}"),
    }
}

// ---------- the whole auction ----------

#[tokio::test]
async fn auction_reports_how_thin_it_was() {
    let ds = dealers(&[("a", 3.0, 20), ("b", 1.0, 20), ("c", 2.0, 20), ("gone", 0.1, 900)]);

    let quote = run_auction(&ds, &req(Side::Buy), Duration::from_millis(200), Duration::from_secs(5))
        .await
        .unwrap();

    assert_eq!(quote.solicited, 4);
    assert_eq!(quote.responded, 3);
    // "gone" had the tightest spread but never answered, so b wins on 1 pip.
    assert_eq!(quote.dealer, "b");
    assert!(close(quote.rate, FAIR_122D + 1.0 * PIP));
}

#[tokio::test]
async fn auction_with_no_responses_errors() {
    let ds = dealers(&[("slow1", 1.0, 400), ("slow2", 2.0, 400)]);
    let err = run_auction(&ds, &req(Side::Buy), Duration::from_millis(50), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err, RfqError::NoQuotes { solicited: 2 });
}
