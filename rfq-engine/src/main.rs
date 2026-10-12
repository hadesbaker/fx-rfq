//! One auction, end to end.
//!
//! Eight dealers with different edges and different response times are asked
//! for the same four month USD/BRL price at the same moment. Some answer
//! inside the window and some do not. The best of those that did becomes the
//! client's quote, and it expires.
//!
//! The random seed is fixed so runs are reproducible. Change it and a
//! different dealer wins.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use rfq_engine::dealer::{Dealer, MockDealer};
use rfq_engine::engine::{best_quote, request_quotes};
use rfq_engine::types::{Market, Quote, QuoteRequest, Side, PIP};

const MARKET: Market = Market {
    spot: 5.40,
    r_base: 0.04,
    r_quote: 0.12,
};
const WINDOW: Duration = Duration::from_millis(250);
const TTL: Duration = Duration::from_secs(3);
const SEED: u64 = 11;

#[tokio::main]
async fn main() {
    let req = QuoteRequest {
        pair: "USD/BRL".to_string(),
        notional: 10_000_000.0,
        tenor_days: 122,
        side: Side::Buy,
    };

    // Build a roster. The specs are kept alongside so the report can show who
    // was asked, not only who answered.
    let mut rng = StdRng::seed_from_u64(SEED);
    let specs: Vec<(String, f64, u64)> = ["a", "b", "c", "d", "e", "f", "g", "h"]
        .iter()
        .map(|id| {
            (
                format!("dealer-{id}"),
                (rng.gen_range(5..60) as f64) / 10.0, // 0.5 to 6.0 pips of edge
                rng.gen_range(30..420),               // 30 to 420 ms to answer
            )
        })
        .collect();

    let dealers: Vec<Arc<dyn Dealer>> = specs
        .iter()
        .map(|(n, s, l)| Arc::new(MockDealer::new(n, MARKET, *s, *l)) as Arc<dyn Dealer>)
        .collect();

    println!("\nCLIENT REQUEST");
    println!("  {:?} USD {:.0} against BRL", req.side, req.notional);
    println!("  tenor {} days", req.tenor_days);
    println!(
        "  spot {:.4}   USD {}%   BRL {}%",
        MARKET.spot,
        MARKET.r_base * 100.0,
        MARKET.r_quote * 100.0
    );
    println!("  {} dealers solicited, {:?} window", dealers.len(), WINDOW);

    // Every dealer is asked at the same moment.
    let started = Instant::now();
    let quotes = request_quotes(&dealers, &req, WINDOW).await;
    let elapsed = started.elapsed();

    println!("\nRESPONSES");
    println!(
        "  {:<10} {:>7} {:>9}   {:<10} status",
        "dealer", "edge", "latency", "quote"
    );
    for (name, spread, latency) in &specs {
        match quotes.iter().find(|q| &q.dealer == name) {
            Some(q) => println!(
                "  {:<10} {:>5.1}p {:>7}ms   {:<10.6} answered",
                name, spread, latency, q.rate
            ),
            None => println!(
                "  {:<10} {:>5.1}p {:>7}ms   {:<10} missed the window",
                name, spread, latency, "-"
            ),
        }
    }

    let sequential: u64 = specs.iter().map(|(_, _, l)| l).sum();
    println!("\nTIMING");
    println!("  auction took          {:>6}ms", elapsed.as_millis());
    println!("  asking in sequence    {:>6}ms", sequential);
    println!("  the dealers were asked concurrently, so the auction costs the");
    println!("  slowest responder inside the window rather than the sum");

    // Pick the winner. Buying, so the lowest rate wins.
    let Some(best) = best_quote(&quotes, req.side) else {
        println!(
            "\nNO QUOTE: nobody answered out of {} solicited\n",
            dealers.len()
        );
        return;
    };
    let quote = Quote::new(best, req.side, dealers.len(), quotes.len(), TTL);

    println!("\nQUOTE");
    println!(
        "  {} of {} dealers answered",
        quote.responded, quote.solicited
    );
    println!("  winner      {} at {:.6}", quote.dealer, quote.rate);
    let fair = fair_value(&req);
    println!("  fair value  {fair:.6}");
    println!(
        "  client pays {:.1} pips over fair",
        (quote.rate - fair) / PIP
    );
    println!("  good for    {:?}", quote.ttl);

    println!("\nFILL");
    match quote.accept(req.notional, Instant::now()) {
        Ok(fill) => {
            println!("  struck with {} at {:.6}", fill.dealer, fill.rate);
            println!(
                "  client pays {:.2} BRL for {:.0} USD",
                fill.notional * fill.rate,
                fill.notional
            );
        }
        Err(e) => println!("  refused: {e}"),
    }

    // The same quote, accepted after it expired.
    println!("\nTHE SAME QUOTE, ACCEPTED LATE");
    let late = quote.issued_at + TTL + Duration::from_secs(1);
    match quote.accept(req.notional, late) {
        Ok(_) => println!("  struck, which should not happen"),
        Err(e) => println!("  refused: {e}"),
    }
    println!();
}

/// Covered interest parity with no dealer edge, for comparison only.
fn fair_value(req: &QuoteRequest) -> f64 {
    let t = fx_forward_pricer::fx::year_fraction(req.tenor_days, 360);
    fx_forward_pricer::fx::forward_rate(MARKET.spot, MARKET.r_base, MARKET.r_quote, t)
}
