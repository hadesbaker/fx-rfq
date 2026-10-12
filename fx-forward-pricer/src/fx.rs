//! FX forward pricing.
//!
//! Prices outright forwards from covered interest parity, converts them to
//! the forward points dealers actually quote, and interpolates broken dates
//! off a two-point curve.
//!
//! # Conventions
//!
//! Pairs are `BASE/QUOTE` and a rate is the number of quote units per one
//! base unit. In `USD/BRL = 5.40`, USD is the base and one dollar buys 5.40
//! reais.
//!
//! Interest rates are decimals, so 12% is `0.12`. Tenors are year fractions,
//! produced by [`year_fraction`] rather than assumed: a 3M tenor is 92 actual
//! days on a 360-day basis, not 0.25.
//!
//! A pip is `0.0001` for most pairs and `0.01` for JPY pairs. Pips are
//! absolute increments in the quote currency, not basis points.
//!
//! # Scope
//!
//! Covered interest parity anchors forwards in **deliverable** currencies,
//! where the arbitrage that enforces it can actually be executed. In
//! non-deliverable corridors such as BRL, INR and KRW, capital controls block
//! one leg of that arbitrage, so parity does not bind: the traded points are
//! the market and an implied yield is the output rather than the input. See
//! the scope section of the README.

use std::error::Error;
use std::fmt;

/// Errors returned when a curve cannot support the requested tenor.
#[derive(Debug, PartialEq)]
pub enum FxError {
    /// Two curve points share the same tenor, so the slope between them is
    /// undefined.
    DegenerateTenor { t: f64 },
    /// The requested tenor lies outside the two quoted tenors. Extrapolating
    /// a forward curve past its quoted points produces a price with no market
    /// data behind it, so it is refused rather than returned.
    Extrapolation { t: f64, t1: f64, t2: f64 },
}

impl fmt::Display for FxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FxError::DegenerateTenor { t } => {
                write!(f, "two quoted tenors share t={t}, slope undefined")
            }
            FxError::Extrapolation { t, t1, t2 } => {
                write!(f, "t={t} is outside the quoted range [{t1}, {t2}]")
            }
        }
    }
}

impl Error for FxError {}

/// Converts a day count into a year fraction on the given basis.
///
/// USD and BRL accrue on ACT/360; GBP uses ACT/365. The distinction is not
/// cosmetic: pricing a 3M USD/BRL forward at `t = 0.25` instead of `92/360`
/// moves the quote by roughly 23.5 pips.
///
/// ```
/// use fx_forward_pricer::fx::year_fraction;
/// assert_eq!(year_fraction(360, 360), 1.0);
/// ```
pub fn year_fraction(days: u32, basis: u32) -> f64 {
    days as f64 / basis as f64
}

/// Prices an outright forward from covered interest parity.
///
/// ```text
/// F = S x (1 + r_quote x t) / (1 + r_base x t)
/// ```
///
/// The rate is the level at which converting now and investing in the quote
/// currency returns the same amount as investing in the base currency and
/// converting forward. Any other level is a riskless profit for whoever takes
/// the other side, which is why a forward is a carry calculation rather than
/// a forecast.
///
/// A quote-currency rate above the base-currency rate puts the forward above
/// spot. The higher-yielding currency trades at a forward discount.
///
/// ```
/// use fx_forward_pricer::fx::forward_rate;
/// // BRL at 12% against USD at 4%, one year, spot 5.40.
/// let f = forward_rate(5.40, 0.04, 0.12, 1.0);
/// assert!(f > 5.40);
/// ```
pub fn forward_rate(spot: f64, r_base: f64, r_quote: f64, t: f64) -> f64 {
    let quote = 1.0 + r_quote * t;
    let base = 1.0 + r_base * t;
    spot * quote / base
}

/// Expresses a forward as points: the difference from spot, counted in pips.
///
/// Dealers quote points rather than outrights because spot moves continuously
/// while the difference depends only on the rate differential and the tenor.
/// Points hold for hours; an outright is stale on arrival. At execution the
/// points are added to whatever spot prints at that moment.
///
/// Returns a negative number when the forward sits below spot, which is the
/// normal case for a quote currency that pays less than the base currency.
///
/// ```
/// use fx_forward_pricer::fx::forward_points;
/// assert_eq!(forward_points(5.40, 5.41, 0.0001).round(), 100.0);
/// ```
pub fn forward_points(spot: f64, forward: f64, pip: f64) -> f64 {
    let diff = forward - spot;
    diff / pip
}

/// Interpolates forward points for a broken date between two quoted tenors.
///
/// Standard tenors are 1W, 1M, 3M, 6M and 1Y. Anything else is a broken date
/// and has no published price, so it is constructed from the tenors either
/// side of it.
///
/// Interpolation is on **points**, never on outrights. An outright contains
/// spot, and spot is not a function of tenor, so interpolating outrights
/// averages spot against itself and biases the result. Interpolating points
/// varies only the carry, which is the component that depends on time.
///
/// Accepts the two curve points in either order. Returns
/// [`FxError::Extrapolation`] when the requested tenor falls outside them.
///
/// ```
/// use fx_forward_pricer::fx::interpolate_points;
/// // Midway between a 3M at 1000 points and a 6M at 2000 points.
/// let p = interpolate_points(0.25, 1000.0, 0.5, 2000.0, 0.375).unwrap();
/// assert_eq!(p, 1500.0);
/// ```
pub fn interpolate_points(
    t1: f64,
    pts1: f64,
    t2: f64,
    pts2: f64,
    t: f64,
) -> Result<f64, FxError> {
    if t2 == t1 {
        return Err(FxError::DegenerateTenor { t: t1 });
    }

    let (less, greater) = if t1 > t2 { (t2, t1) } else { (t1, t2) };
    if t > greater || t < less {
        return Err(FxError::Extrapolation { t, t1, t2 });
    }

    let w = (t - t1) / (t2 - t1);

    Ok(pts1 + w * (pts2 - pts1))
}
