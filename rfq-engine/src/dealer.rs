//! The dealer boundary.
//!
//! Anything that can be asked for a price is a [`Dealer`]: a mock with a fixed
//! latency in a test, a randomized one in the demo, a real counterparty over a
//! network. The engine does not change.
//!
//! The boundary exists for testability. Timing and randomness are what make
//! concurrent code unreproducible, so tests hold both fixed and only the demo
//! randomizes.

use async_trait::async_trait;
use std::time::Duration;

use crate::types::{DealerQuote, Market, QuoteRequest, Side, PIP};

#[async_trait]
pub trait Dealer: Send + Sync {
    fn name(&self) -> &str;

    /// Price the request. Taking as long as it takes: the engine, not the
    /// dealer, decides how long it is prepared to wait.
    async fn quote(&self, req: &QuoteRequest) -> DealerQuote;
}

/// A dealer with a fixed view of the market, a fixed edge, and a fixed delay.
pub struct MockDealer {
    pub name: String,
    pub market: Market,
    /// Pips of edge this dealer takes, always in its own favor.
    pub spread_pips: f64,
    /// How long it takes to answer. Set this above the engine's window to
    /// simulate a dealer that never responds.
    pub latency: Duration,
}

impl MockDealer {
    pub fn new(name: &str, market: Market, spread_pips: f64, latency_ms: u64) -> Self {
        MockDealer {
            name: name.to_string(),
            market,
            spread_pips,
            latency: Duration::from_millis(latency_ms),
        }
    }
}

#[async_trait]
impl Dealer for MockDealer {
    fn name(&self) -> &str {
        &self.name
    }

    /// Prices the request off covered interest parity and takes its edge.
    ///
    /// The spread always moves against the client: a buyer pays more than fair
    /// value, a seller receives less. Fair value comes from
    /// [`fx_forward_pricer`], so this dealer and the engine agree on the
    /// arithmetic and differ only on the edge.
    async fn quote(&self, req: &QuoteRequest) -> DealerQuote {
        tokio::time::sleep(self.latency).await;
        let tenor = fx_forward_pricer::fx::year_fraction(req.tenor_days, 360);

        let fair = fx_forward_pricer::fx::forward_rate(
            self.market.spot,
            self.market.r_base,
            self.market.r_quote,
            tenor,
        );

        let edge = self.spread_pips * PIP;
        let rate = match req.side {
            Side::Buy => fair + edge,
            Side::Sell => fair - edge,
        };

        DealerQuote {
            dealer: self.name.clone(),
            rate,
            latency: self.latency,
        }
    }
}
