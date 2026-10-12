# crx

Rust projects covering FX forward pricing and quote distribution, written in
preparation for CRX.

Each directory is a self-contained project with its own README, tests and
license. The three build on each other: `fx-forward-pricer` is fair value,
`rfq-engine` distributes it through an auction, and `rfq-dapp` settles the
outcome on chain.

## Projects

### `fx-forward-pricer`

Prices outright FX forwards from covered interest parity, converts them into
the forward points dealers quote, and constructs broken dates by interpolating
between two quoted tenors.

The binary prices one client request end to end: a four month USD/BRL hedge on
a USD 10,000,000 notional, built from a 3M/6M curve, with the carry cost
separated from the spot cost.

```sh
cd fx-forward-pricer
cargo build --release
cargo run --release
cargo test
```

18 integration tests, 4 doc tests, clippy clean.

### `rfq-engine`

A request-for-quote engine. Fans a quote request out to competing dealers
concurrently, collects the responses that arrive inside a window, returns the
best price for the side being traded, and expires it.

Depends on `fx-forward-pricer` for fair value. Every dealer prices off covered
interest parity and adds its own edge, so the auction has something real to
compete on.

The demo polls eight dealers carrying 2,027ms of latency between them inside a
single 250ms window, drops the half that miss it, and reports four of eight so
the client knows how thin the auction was.

```sh
cd rfq-engine
cargo build --release
cargo run --release
cargo test
```

14 tests, clippy clean.

### `rfq-dapp`

The same auction, settled on chain. The winning dealer signs its price under
EIP-712; the client submits that signature to a Solidity contract that verifies
it, checks the expiry and a nonce, rejects malleable signatures, and records the
position with collateral from both sides.

The auction stays off chain deliberately, and not because of latency. A quote
is an option the dealer gives away for free: eight dealers quote, one wins, and
seven of those options expire worthless. Paying consensus costs to mint options
that mostly die is waste at any block time. Only the outcome is settled.

Runs against anvil or Sepolia. There is no mainnet target: the contracts are
unaudited and the mock collateral token lets anyone mint. Working against real
chain state needs no mainnet mode anyway, since `anvil --fork-url` inherits the
forked chain's id and is reached as the local network.

```sh
cd rfq-dapp
cp .env.example .env && chmod 600 .env
set -a; source .env; set +a          # the deploy needs the key in the shell

( cd contracts && forge install foundry-rs/forge-std )   # first time only

anvil &                              # or a second terminal
forge script Deploy --root contracts --rpc-url "$LOCAL_RPC_URL" \
  --broadcast --private-key "$DEPLOYER_PRIVATE_KEY"

cargo run -- fund
cargo run -- open
```

18 contract tests, 3 unit tests, and 5 integration tests against a spawned
anvil node. Clippy clean. Requires Foundry, and the toolchain pinned in
`rust-toolchain.toml`.

## Scope

These are working models of specific mechanics, not production systems, and
each README states what it leaves out.

The most important limitation is shared across the collection and worth
stating once here. **Covered interest parity anchors forwards only in
deliverable currencies.** In non-deliverable corridors such as BRL, INR and
KRW, capital controls prevent a foreign counterparty from borrowing the local
currency onshore and taking delivery, so one leg of the enforcing arbitrage is
unavailable and parity does not bind. In those corridors the traded points are
the market and an implied yield is backed out of them, rather than the points
being derived from onshore deposit rates.

The USD/BRL figures throughout are therefore illustrative of the mechanics
rather than of how a BRL non-deliverable forward is actually priced. Bid/ask
spreads, market-quoted curves, business day calendars, credit, collateral and
fixings are named in the individual READMEs as the things a production quote
carries that these do not.

## License

MIT, per project. See each directory's `LICENSE`.
