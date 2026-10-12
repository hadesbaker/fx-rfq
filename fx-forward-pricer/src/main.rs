//! One client request priced end to end.
//!
//! A Brazilian importer owes a US supplier USD 10,000,000 in four months and
//! holds reais. They want the cost of those dollars fixed today so a currency
//! move cannot change the size of the bill.
//!
//! Four months is a broken date. The curve carries 3M and 6M, so the 4M price
//! is constructed from them.

use fx_forward_pricer::fx::{forward_points, forward_rate, interpolate_points, year_fraction};

const SPOT: f64 = 5.40; // USD/BRL: one dollar buys 5.40 reais
const R_USD: f64 = 0.04; // base currency
const R_BRL: f64 = 0.12; // quote currency
const PIP: f64 = 0.0001;
const NOTIONAL: f64 = 10_000_000.0; // USD required by the client

fn main() {
    println!("\nCLIENT REQUEST");
    println!("  buy USD {NOTIONAL:.0} against BRL, settling in 4 months");
    println!("  USD/BRL spot {SPOT:.4}   USD {}%   BRL {}%", R_USD * 100.0, R_BRL * 100.0);
    println!("  at today's spot the bill would be {:.2} BRL", NOTIONAL * SPOT);

    // Tenors are calendar dates; parity needs year fractions. 3M from the
    // spot date is 92 actual days, and USD and BRL both accrue on ACT/360.
    let t_3m = year_fraction(92, 360);
    let t_6m = year_fraction(183, 360);
    let t_4m = year_fraction(122, 360);
    println!("\n1. TENORS AS YEAR FRACTIONS   year_fraction()");
    println!("    3M   92 days -> t = {t_3m:.6}");
    println!("    6M  183 days -> t = {t_6m:.6}");
    println!("    4M  122 days -> t = {t_4m:.6}   <- requested");

    // Covered interest parity on the two tenors the curve carries.
    let f_3m = forward_rate(SPOT, R_USD, R_BRL, t_3m);
    let f_6m = forward_rate(SPOT, R_USD, R_BRL, t_6m);
    println!("\n2. THE CURVE   forward_rate()");
    println!("    3M outright {f_3m:.6}");
    println!("    6M outright {f_6m:.6}");
    println!("    both above spot: BRL pays 12%, USD pays 4%");

    // Dealers quote the difference from spot, not the outright, because the
    // difference is stable for hours while spot moves continuously.
    let p_3m = forward_points(SPOT, f_3m, PIP);
    let p_6m = forward_points(SPOT, f_6m, PIP);
    println!("\n3. AS FORWARD POINTS   forward_points()");
    println!("    3M {p_3m:.2} points");
    println!("    6M {p_6m:.2} points");

    // The broken date. Interpolating points rather than outrights keeps spot
    // out of the interpolation, since spot does not vary with tenor.
    let p_4m = match interpolate_points(t_3m, p_3m, t_6m, p_6m, t_4m) {
        Ok(points) => points,
        Err(e) => {
            eprintln!("cannot quote this tenor: {e}");
            return;
        }
    };
    println!("\n4. THE BROKEN DATE   interpolate_points()");
    println!("    4M {p_4m:.2} points, built from the 3M and 6M");

    // A fill applies the quoted points to spot at the moment of execution.
    let f_4m = SPOT + p_4m * PIP;
    let cost_forward = NOTIONAL * f_4m;
    let cost_spot = NOTIONAL * SPOT;
    println!("\n5. THE QUOTE");
    println!("    4M outright {f_4m:.6}   (spot {SPOT:.4} + {p_4m:.2} points)");
    println!("    client pays {cost_forward:.2} BRL in four months");
    println!("    versus     {cost_spot:.2} BRL at today's spot");
    println!("    difference {:.2} BRL", cost_forward - cost_spot);

    let pct = (f_4m / SPOT - 1.0) * 100.0;
    let differential = (R_BRL - R_USD) * t_4m * 100.0;
    println!("\n6. WHAT THE DIFFERENCE IS");
    println!("    the forward sits {pct:.3}% above spot");
    println!("    the rate differential over 122 days is {differential:.3}%");
    println!("    the forward is the smaller of the two because the");
    println!("    differential accrues on the original amount while");
    println!("    conversion applies to the grown amount");
    println!("\n    the difference is the cost of carry for 122 days, not a");
    println!("    view on the real. The client has exchanged an unknown");
    println!("    future bill for a known one.\n");
}
