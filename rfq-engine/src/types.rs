use std::fmt;
use std::time::{Duration, Instant};

/// One pip, for a 4-decimal pair.
pub const PIP: f64 = 0.0001;

/// Which way the client is going, expressed on the base currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Client buys the base currency and pays the quote currency.
    Buy,
    /// Client sells the base currency and receives the quote currency.
    Sell,
}

/// What a dealer believes the market is. Each dealer holds its own, so two
/// dealers are free to disagree.
#[derive(Debug, Clone, Copy)]
pub struct Market {
    pub spot: f64,
    pub r_base: f64,
    pub r_quote: f64,
}

/// What the client sends. Deliberately carries no market data: a client asks
/// for a price, it does not supply one.
#[derive(Debug, Clone)]
pub struct QuoteRequest {
    pub pair: String,
    pub notional: f64,
    pub tenor_days: u32,
    pub side: Side,
}

/// One dealer's response.
#[derive(Debug, Clone, PartialEq)]
pub struct DealerQuote {
    pub dealer: String,
    pub rate: f64,
    pub latency: Duration,
}

/// What the engine hands back: the winning price, how thin the auction was,
/// and how long the price is good for.
#[derive(Debug, Clone)]
pub struct Quote {
    pub dealer: String,
    pub rate: f64,
    pub side: Side,
    pub solicited: usize,
    pub responded: usize,
    pub issued_at: Instant,
    pub ttl: Duration,
}

impl Quote {
    pub fn new(
        best: &DealerQuote,
        side: Side,
        solicited: usize,
        responded: usize,
        ttl: Duration,
    ) -> Self {
        Quote {
            dealer: best.dealer.clone(),
            rate: best.rate,
            side,
            solicited,
            responded,
            issued_at: Instant::now(),
            ttl,
        }
    }
}

/// A struck trade.
#[derive(Debug, Clone)]
pub struct Fill {
    pub dealer: String,
    pub rate: f64,
    pub notional: f64,
}

#[derive(Debug, PartialEq)]
pub enum RfqError {
    /// Nobody answered inside the window. Distinct from a thin auction: this
    /// is a failure, a partial response set is not.
    NoQuotes { solicited: usize },
    /// The client accepted after the price had expired.
    QuoteExpired { age: Duration, ttl: Duration },
}

impl fmt::Display for RfqError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RfqError::NoQuotes { solicited } => {
                write!(f, "no dealer answered out of {solicited} solicited")
            }
            RfqError::QuoteExpired { age, ttl } => {
                write!(f, "quote expired: {age:?} old, good for {ttl:?}")
            }
        }
    }
}

impl std::error::Error for RfqError {}
