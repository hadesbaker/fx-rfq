//! The auction.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::dealer::Dealer;
use crate::types::{DealerQuote, Fill, Quote, QuoteRequest, RfqError, Side};

/// Asks every dealer at the same time and returns whoever answered inside
/// `window`.
///
/// Dealers are solicited concurrently, so this takes about as long as the
/// slowest responder rather than the sum of all of them. Ten dealers at 200ms
/// each is one 200ms call, not two seconds, and that difference decides
/// whether a quote is still valid when it reaches the client.
///
/// A dealer that does not answer in time is dropped rather than allowed to
/// hold the auction open, so **a short response set is normal and is not an
/// error**. Compare `dealers.len()` against the returned length to see how
/// thin the auction was.
pub async fn request_quotes(
    dealers: &[Arc<dyn Dealer>],
    req: &QuoteRequest,
    window: Duration,
) -> Vec<DealerQuote> {
    // Each task may outlive this function, so it cannot borrow the request.
    // An Arc gives every task its own owned handle to one shared copy.
    let request = Arc::new(req.clone());

    let mut handles = Vec::new();
    for d in dealers {
        let d = Arc::clone(d);
        let request = Arc::clone(&request);
        handles.push(tokio::spawn(async move {
            tokio::time::timeout(window, d.quote(&request)).await
        }));
    }

    let mut quotes = Vec::new();
    for h in handles {
        // The outer Result is whether the task ran; the inner is whether the
        // dealer answered in time. Anything else is simply not a quote.
        if let Ok(Ok(quote)) = h.await {
            quotes.push(quote);
        }
    }
    quotes
}

/// Picks the winning quote for the side the client is trading.
///
/// The comparison differs by direction, and getting it wrong returns a
/// plausible-looking price that is the worst one available. A client buying
/// the base currency pays the rate, so the lowest wins. A client selling
/// receives it, so the highest wins.
///
/// Returns `None` for an empty set.
pub fn best_quote(quotes: &[DealerQuote], side: Side) -> Option<&DealerQuote> {
    let by_rate = |a: &&DealerQuote, b: &&DealerQuote| {
        a.rate
            .partial_cmp(&b.rate)
            .unwrap_or(std::cmp::Ordering::Equal)
    };

    match side {
        Side::Buy => quotes.iter().min_by(by_rate),
        Side::Sell => quotes.iter().max_by(by_rate),
    }
}

impl Quote {
    /// Strikes the trade, if the price is still live.
    ///
    /// A quote is a firm commitment priced against spot as it stood when the
    /// quote was issued. Honoring it indefinitely would hand the dealer a
    /// loss every time the market moved, so it expires.
    ///
    /// `now` is a parameter rather than read internally, which makes expiry
    /// testable without waiting for it.
    pub fn accept(&self, notional: f64, now: Instant) -> Result<Fill, RfqError> {
        let age = now.duration_since(self.issued_at);
        if age > self.ttl {
            return Err(RfqError::QuoteExpired { age, ttl: self.ttl });
        }

        Ok(Fill {
            dealer: self.dealer.clone(),
            rate: self.rate,
            notional,
        })
    }
}

/// Runs an auction end to end: solicit every dealer, pick the winner, and
/// issue a quote that carries how thin the auction was and how long the price
/// is good for.
pub async fn run_auction(
    dealers: &[Arc<dyn Dealer>],
    req: &QuoteRequest,
    window: Duration,
    ttl: Duration,
) -> Result<Quote, RfqError> {
    let quotes = request_quotes(dealers, req, window).await;
    let best = best_quote(&quotes, req.side).ok_or(RfqError::NoQuotes {
        solicited: dealers.len(),
    })?;
    Ok(Quote::new(best, req.side, dealers.len(), quotes.len(), ttl))
}
