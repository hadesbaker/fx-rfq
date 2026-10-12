# rfq-engine

A request-for-quote engine in Rust. Fans a quote request out to competing
dealers concurrently, collects the responses that arrive inside a window,
returns the best price for the side being traded, and expires it.

Prices come from [`fx-forward-pricer`](../fx-forward-pricer), which sits
beside this crate. That one is fair value; this one is distribution.

## Quick start

From the repository root:

```sh
cd rfq-engine
cargo build --release
cargo run --release
```

```sh
cargo test    # 14 tests
cargo clippy  # clean
```

## Why it is concurrent

An FX quote is good for seconds. Ten dealers at 200ms each is two seconds
asked one at a time, and two seconds is long enough that the spot rate the
price was computed against has already moved.

So every dealer is asked at the same moment, and the auction costs the slowest
responder inside the window rather than the sum of all of them. In the run
below, eight dealers carrying 2,027ms of latency between them were polled
inside a single 250ms window.

## What it does

```
CLIENT REQUEST
  Buy USD 10000000 against BRL
  tenor 122 days
  spot 5.4000   USD 4%   BRL 12%
  8 dealers solicited, 250ms window

RESPONSES
  dealer        edge   latency   quote      status
  dealer-a     1.7p     337ms   -          missed the window
  dealer-b     5.2p     104ms   5.544962   answered
  dealer-c     1.6p     181ms   5.544602   answered
  dealer-d     5.8p     387ms   -          missed the window
  dealer-e     4.6p     375ms   -          missed the window
  dealer-f     5.1p     125ms   5.544952   answered
  dealer-g     5.6p     394ms   -          missed the window
  dealer-h     4.2p     124ms   5.544862   answered

TIMING
  auction took             254ms
  asking in sequence      2027ms

QUOTE
  4 of 8 dealers answered
  winner      dealer-c at 5.544602
  fair value  5.544442
  client pays 1.6 pips over fair
  good for    3s

FILL
  struck with dealer-c at 5.544602
  client pays 55446020.08 BRL for 10000000 USD

THE SAME QUOTE, ACCEPTED LATE
  refused: quote expired: 4s old, good for 3s
```

Worth noting what that run shows. `dealer-a` had the second tightest edge in
the field at 1.7 pips and won nothing, because it answered in 337ms against a
250ms window. Half the field missed entirely, and the client is told so: four
of eight. A corridor where two dealers answer out of ten is thin, and that is
information the client should have before accepting a price.

The random seed is fixed, so the run reproduces. Change `SEED` in `main.rs`
and a different dealer wins.

## API

| item                                     | purpose                                                                      |
| ---------------------------------------- | ---------------------------------------------------------------------------- |
| `Dealer`                                 | the trait a counterparty implements: a name, and an async `quote`            |
| `MockDealer`                             | a dealer with a fixed market view, a fixed edge in pips, and a fixed latency |
| `request_quotes(dealers, req, window)`   | asks everyone concurrently, returns whoever answered in time                 |
| `best_quote(quotes, side)`               | the winner for that direction                                                |
| `Quote::accept(notional, now)`           | strikes the trade, or fails if the price expired                             |
| `run_auction(dealers, req, window, ttl)` | all of the above, end to end                                                 |

Four design decisions worth stating, since each one is a place this could
quietly be wrong:

**A short response set is not an error.** Nine dealers out of ten is a good
auction. A single hung dealer must not hold it open, so slow responses are
dropped and the result carries both how many were asked and how many answered.
Zero responses is a real failure and is reported separately, as
`RfqError::NoQuotes`.

**"Best" depends on the side.** A client buying the base currency pays the
rate, so the lowest wins. A client selling receives it, so the highest wins.
Using one comparison for both returns a plausible-looking number that is the
worst price in the book, which no amount of inspection reveals. There is a
test asserting the two sides pick different dealers.

**Quotes expire.** A quote is a firm commitment priced against spot at the
instant it was issued. Honoring it indefinitely hands the dealer a loss every
time the market moves. This is also where _last look_ comes from in real FX:
dealers wanted a veto after acceptance because clients were picking off stale
prices. An explicit expiry is the same protection under a rule both sides can
see, rather than a discretionary rejection after the fact.

**`accept` takes `now` as an argument.** Reading the clock inside the function
would make expiry testable only by waiting for it. Passing the instant in
means the test reaches two seconds into the future directly.

## The dealer boundary

`Dealer` is a trait so the engine never knows what it is talking to: a mock
with a fixed latency in a test, a randomized one in the demo, a real
counterparty over a network later.

That exists for testability more than for extensibility. **Timing and
randomness are the two things that make concurrent code unreproducible**, so
every test holds both fixed and only the demo binary randomizes. The
concurrency test asserts that five dealers at 100ms each complete in under
350ms, which fails loudly if the fan-out is ever made sequential.

`async_trait` is used because a trait with a native `async fn` is not
object-safe, and a heterogeneous `Vec<Arc<dyn Dealer>>` is the whole point.

## What this does not model

- **No last look.** Acceptance inside the expiry window always succeeds.
- **No credit or collateral.** Every dealer is assumed good for the trade. A
  matched principal venue carries credit risk to both sides, and the price
  depends on who is asking and what is posted.
- **No network.** Latency is a timer, not a measurement, and there is no
  reconnection, retry or session management.
- **No size tiering.** Real dealers quote differently at 1M and 100M and may
  decline a size outright. Here notional does not affect the price.
- **No partial fills.** All or nothing.
- **One request at a time.** No queueing, rate limiting or backpressure.
- **No information-leakage controls.** Dealers cannot see each other here, but
  a real venue has to guarantee that, and has to consider whether the request
  itself leaks a client's intent into the underlying market.
- **Parity pricing in non-deliverable corridors.** Dealers quote off covered
  interest parity, which does not bind where capital controls block the
  enforcing arbitrage. See the scope section of `fx-forward-pricer`.
- **No reporting.** A real swap is reported to a swap data repository.

## License

MIT. See `LICENSE`.
