# fx-forward-pricer

FX forward pricing in Rust. Prices outright forwards from covered interest
parity, converts them to the forward points dealers quote, and constructs
broken dates by interpolating between two quoted tenors.

No dependencies. Library plus a worked example binary.

## Quick start

From the repository root:

```sh
cd fx-forward-pricer
cargo build --release
cargo run --release
```

```sh
cargo test    # 18 integration tests, 4 doc tests
cargo clippy  # clean
```

## What it does

The binary prices one client request end to end. A Brazilian importer owes a
US supplier USD 10,000,000 in four months and holds reais. Four months is not
a standard tenor, so the price is built from the 3M and 6M points on the
curve.

```
client wants USD 10,000,000 in 4 months
  |
  1. year_fraction        122 days / 360 -> t = 0.338889
  2. forward_rate         3M = 5.509283    6M = 5.615224
  3. forward_points       3M = 1092.83     6M = 2152.24
  4. interpolate_points   4M = 1442.08 points
  5. spot + points x pip  4M outright = 5.544208
  |
  v
  client pays 55,442,084.57 BRL, fixed today
       versus 54,000,000.00 BRL at today's spot
```

That 1,442,084.57 BRL difference is the cost of carrying the position for 122
days. It is not a forecast. If the real is unchanged in four months the client
has still bought what they wanted, which is a known bill rather than an
unknown one.

## API

| function                                    | purpose                                                                     |
| ------------------------------------------- | --------------------------------------------------------------------------- |
| `year_fraction(days, basis)`                | day count to year fraction. ACT/360 for USD and BRL, ACT/365 for GBP.       |
| `forward_rate(spot, r_base, r_quote, t)`    | covered interest parity: `F = S x (1 + r_quote x t) / (1 + r_base x t)`     |
| `forward_points(spot, forward, pip)`        | the difference from spot, in pips. Negative when the forward is below spot. |
| `interpolate_points(t1, pts1, t2, pts2, t)` | broken dates. Returns `Err` on a degenerate tenor or on extrapolation.      |

Two design decisions worth stating explicitly:

**Interpolation is on points, not outrights.** An outright contains spot, and
spot is not a function of tenor, so interpolating outrights averages spot
against itself and biases the result. Interpolating points varies only the
carry, which is the component that depends on time.

**Extrapolation returns an error rather than a number.** Requesting 9M off a
3M/6M curve is not a mathematical failure, it is a request for a price with no
market data behind it. The caller is told, rather than handed something that
looks like a quote.

## Conventions

Pairs are `BASE/QUOTE`, and a rate is quote units per one base unit. In
`USD/BRL = 5.40`, USD is the base and one dollar buys 5.40 reais.

Rates are decimals: 12% is `0.12`. Tenors are year fractions produced by
`year_fraction` rather than assumed, because a 3M tenor is 92 actual days on a
360-day basis and not 0.25. Pricing at 0.25 moves a 3M USD/BRL quote by about
23.5 pips.

A pip is `0.0001` for most pairs and `0.01` for JPY pairs.

## Scope

Covered interest parity anchors forwards in **deliverable** currencies, where
the arbitrage enforcing it can actually be executed.

**It does not bind in non-deliverable corridors.** For BRL, INR, KRW and the
rest, capital controls block a foreign counterparty from borrowing the local
currency onshore and taking delivery, so one leg of the arbitrage is
unavailable. In those corridors the traded points are the market and an
implied yield is backed out of them, rather than the points being derived from
onshore deposit rates. The USD/BRL figures here are therefore illustrative of
the mechanics, not of how a BRL NDF is actually priced.

Also not modeled, and each one is a real part of a production quote:

- **Bid and ask.** One mid price is computed. A dealer quotes two sides.
- **A market curve.** Points are derived from flat deposit rates rather than
  taken from quoted market points at each standard tenor.
- **Holiday calendars and date rolls.** Day counts are supplied directly. Real
  tenors resolve against business day calendars in both centers, with
  modified-following and end-of-month rules.
- **Credit.** A forward is an unsecured promise until settlement, so the price
  depends on who is asking and on what collateral is posted.
- **Fixings.** A non-deliverable forward settles against a published reference
  such as PTAX, observed on a fixing date before settlement.

## License

MIT. See `LICENSE`.
