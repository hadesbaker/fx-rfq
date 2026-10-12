//! Forward pricing tests.
//!
//! Expected values are pinned to 1e-12 on rates and 1e-6 on points. The
//! direction tests exist separately from the value tests because a
//! transposed numerator and denominator produces a plausible number, and
//! only the sign of `F - S` catches it.

use fx_forward_pricer::fx::{forward_points, forward_rate, interpolate_points, year_fraction, FxError};

// USD/BRL. USD is base, BRL is quote.
const SPOT: f64 = 5.40;
const R_USD: f64 = 0.04;
const R_BRL: f64 = 0.12;
const PIP: f64 = 0.0001;

fn close(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() < eps
}

// ---------- forward_rate: the values the plan asks you to check ----------

#[test]
fn three_month_outright() {
    let f = forward_rate(SPOT, R_USD, R_BRL, 0.25);
    assert!(close(f, 5.506930693069307, 1e-12), "3M came out {f}");
}

#[test]
fn six_month_outright() {
    let f = forward_rate(SPOT, R_USD, R_BRL, 0.5);
    assert!(close(f, 5.611764705882354, 1e-12), "6M came out {f}");
}

#[test]
fn one_year_outright() {
    let f = forward_rate(SPOT, R_USD, R_BRL, 1.0);
    assert!(close(f, 5.815384615384616, 1e-12), "1Y came out {f}");
}

#[test]
fn zero_tenor_is_spot() {
    // No time, no carry. Both growth factors are 1.0.
    let f = forward_rate(SPOT, R_USD, R_BRL, 0.0);
    assert!(close(f, SPOT, 1e-12), "t=0 gave {f}, should be spot");
}

// ---------- forward_rate: direction ----------

#[test]
fn high_quote_rate_sits_above_spot() {
    // BRL at 12% against USD at 4%. The high-yielding currency trades at a
    // forward discount, so it takes MORE reais per dollar forward.
    let f = forward_rate(SPOT, R_USD, R_BRL, 1.0);
    assert!(f > SPOT, "r_quote > r_base must give F > S, got {f}");
}

#[test]
fn low_quote_rate_sits_below_spot() {
    // USD/JPY. JPY at 0.5% against USD at 4%, so the forward is below spot.
    let f = forward_rate(150.0, 0.04, 0.005, 1.0);
    assert!(f < 150.0, "r_quote < r_base must give F < S, got {f}");
    assert!(close(f, 144.95192307692304, 1e-12), "USD/JPY 1Y came out {f}");
}

#[test]
fn carry_grows_with_tenor() {
    let short = forward_rate(SPOT, R_USD, R_BRL, 0.25);
    let long = forward_rate(SPOT, R_USD, R_BRL, 1.0);
    assert!(long > short, "more time means more carry");
}

// ---------- year_fraction and the day-count gap ----------

#[test]
fn act_360_and_365() {
    assert!(close(year_fraction(92, 360), 0.25555555555555554, 1e-15));
    assert!(close(year_fraction(92, 365), 92.0 / 365.0, 1e-15));
    assert!(close(year_fraction(360, 360), 1.0, 1e-15));
}

#[test]
fn day_count_is_worth_23_points() {
    // A 3M forward is 92 days on ACT/360, not 0.25 of a year. The gap is
    // 23.5 pips on a 1069 pip quote.
    let lazy = forward_rate(SPOT, R_USD, R_BRL, 0.25);
    let real = forward_rate(SPOT, R_USD, R_BRL, year_fraction(92, 360));
    let gap = forward_points(SPOT, real, PIP) - forward_points(SPOT, lazy, PIP);
    assert!(close(gap, 23.52192984366411, 1e-6), "day-count gap was {gap}");
}

// ---------- forward_points ----------

#[test]
fn points_for_each_tenor() {
    let p3 = forward_points(SPOT, forward_rate(SPOT, R_USD, R_BRL, 0.25), PIP);
    let p1y = forward_points(SPOT, forward_rate(SPOT, R_USD, R_BRL, 1.0), PIP);
    assert!(close(p3, 1069.3069306930702, 1e-6), "3M points were {p3}");
    assert!(close(p1y, 4153.846153846157, 1e-6), "1Y points were {p1y}");
}

#[test]
fn points_go_negative_below_spot() {
    // JPY pairs use a 0.01 pip, not 0.0001.
    let f = forward_rate(150.0, 0.04, 0.005, 1.0);
    let p = forward_points(150.0, f, 0.01);
    assert!(p < 0.0, "forward below spot must give negative points, got {p}");
    assert!(close(p, -504.80769230769624, 1e-6), "USD/JPY points were {p}");
}

#[test]
fn points_round_trip_to_the_outright() {
    // How a fill actually works: quote the points, add live spot.
    let f = forward_rate(SPOT, R_USD, R_BRL, 0.25);
    let p = forward_points(SPOT, f, PIP);
    assert!(close(SPOT + p * PIP, f, 1e-12));
}

// ---------- interpolate_points: the broken date ----------

#[test]
fn four_month_from_three_and_six() {
    let p3 = forward_points(SPOT, forward_rate(SPOT, R_USD, R_BRL, 0.25), PIP);
    let p6 = forward_points(SPOT, forward_rate(SPOT, R_USD, R_BRL, 0.5), PIP);
    let p4 = interpolate_points(0.25, p3, 0.5, p6, 1.0 / 3.0).unwrap();
    assert!(close(p4, 1418.7536400698932, 1e-6), "4M points were {p4}");
    assert!(p4 > p3 && p4 < p6, "4M must sit between 3M and 6M");
}

#[test]
fn endpoints_return_themselves() {
    assert!(close(interpolate_points(0.25, 100.0, 0.5, 200.0, 0.25).unwrap(), 100.0, 1e-12));
    assert!(close(interpolate_points(0.25, 100.0, 0.5, 200.0, 0.5).unwrap(), 200.0, 1e-12));
}

#[test]
fn midpoint_is_the_average() {
    let p = interpolate_points(0.0, 0.0, 1.0, 1000.0, 0.5).unwrap();
    assert!(close(p, 500.0, 1e-12), "midpoint was {p}");
}

#[test]
fn reversed_tenor_ordering_still_works() {
    // A curve supplied in either order must give the same answer.
    let a = interpolate_points(0.25, 100.0, 0.5, 200.0, 0.3).unwrap();
    let b = interpolate_points(0.5, 200.0, 0.25, 100.0, 0.3).unwrap();
    assert!(close(a, b, 1e-12), "ordering changed the answer: {a} vs {b}");
}

#[test]
fn degenerate_tenor_is_an_error() {
    let r = interpolate_points(0.25, 100.0, 0.25, 200.0, 0.25);
    assert_eq!(r, Err(FxError::DegenerateTenor { t: 0.25 }));
}

#[test]
fn extrapolation_is_an_error() {
    // 9M is not between 3M and 6M, so there is no market data behind it.
    let r = interpolate_points(0.25, 100.0, 0.5, 200.0, 0.75);
    assert_eq!(
        r,
        Err(FxError::Extrapolation { t: 0.75, t1: 0.25, t2: 0.5 })
    );
    let r = interpolate_points(0.25, 100.0, 0.5, 200.0, 0.1);
    assert_eq!(
        r,
        Err(FxError::Extrapolation { t: 0.1, t1: 0.25, t2: 0.5 })
    );
}
