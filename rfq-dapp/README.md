# rfq-dapp

An off-chain RFQ auction with on-chain settlement. Dealers compete off chain
and the winner signs its price under EIP-712; the client submits that signature
to a contract that verifies it, checks the expiry and a nonce, and records the
position with collateral from both sides.

Runs against a local anvil node or Sepolia.

Builds on the two crates beside it: [`fx-forward-pricer`](../fx-forward-pricer)
for fair value and [`rfq-engine`](../rfq-engine) for the auction.

## Why the auction is not on chain in this demo

On a chain with sub-second finality, latency is not the binding constraint.

**A quote is an option the dealer gives away for free.** The client may take it
or leave it. Eight dealers quote and one wins, so seven of those options expire
worthless. Paying consensus costs to mint options that mostly die is waste, and
it does not get cheaper as the chain gets faster.

**A dealer has to withdraw a price faster than the market moves.** On chain the
floor on that is block cadence plus finality. Off chain there is no floor.

**Quoting on chain changes who will make markets.** Every dealer would need a
funded hot wallet and would pay for quotes that mostly lose.

So the auction runs off chain and only the outcome is settled. What that costs
is trust in a signature. What it buys is a price that was real when it was
given.

**The signature is the whole boundary.** A dealer cannot deny a price it
signed, and cannot be bound to one it did not. Everything the contract does is
check that boundary.

## Quick start

Needs [Foundry](https://getfoundry.sh) and the toolchain pinned in
`rust-toolchain.toml`.

```sh
cp .env.example .env && chmod 600 .env
```

Terminal one:

```sh
anvil
```

Terminal two:

```sh
set -a; source .env; set +a          # the deploy reads the key from the shell

cd contracts
forge install foundry-rs/forge-std   # first time only
forge build && forge test
cd ..

forge script Deploy \
  --root contracts \
  --rpc-url "$LOCAL_RPC_URL" \
  --broadcast \
  --private-key "$DEPLOYER_PRIVATE_KEY"
```

The script prints three addresses. They are already in `config.toml` under
`[networks.local]`, because anvil deploys deterministically from account 0, so
a fresh node plus a fresh deploy reproduces them.

```sh
cargo run -- info        # network, chain id, block, addresses
cargo run -- fund        # mint mock collateral, approve the contract
cargo run -- open        # auction, sign, settle
cargo run -- positions
```

```sh
cargo test                     # 3 unit tests, 5 integration against a spawned anvil
cd contracts && forge test     # 18 contract tests
cargo clippy --all-targets     # clean
```

## What `open` does

```
AUCTION
  buy USD/BRL 10000000, 122 days
  8 dealers, 250ms window
    dealer-01     1.7p   337ms   missed the window
    dealer-02     5.2p   104ms   5.544962
    dealer-03     1.6p   181ms   5.544602
    dealer-04     5.8p   387ms   missed the window
    dealer-05     4.6p   375ms   missed the window
    dealer-06     5.1p   125ms   5.544952
    dealer-07     5.6p   394ms   missed the window
    dealer-08     4.2p   124ms   5.544862
  winner dealer-03 at 5.544602, 1.6 pips over fair (4 of 8 answered)

SUBMITTING
  settlement  0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0
  tx          0x0c230a66f5725a7f963e6d1ae48f286411d5e1449ba755f9b2ecbd962369e156
  gas used    285689

POSITION 0
  dealer      0x70997970C51812dc3A010C7d01b50e0d17dc79C8
  client      0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC
  rate        5544602008332000000
  notional    10000000000000000000000000
  collateral  500000000000 per side
  fixing date 1802022032
  settled     false
```

`dealer-01` had the second tightest edge in the field and won nothing, because
it answered in 337ms against a 250ms window. Four of eight missed entirely, and
the client is told so: a corridor where two dealers answer out of ten is thin,
and that is information worth having before accepting a price.

`cargo run -- quote` stops one step earlier and prints the digest and the
signature without sending anything.

## The contract

`contracts/src/RfqSettlement.sol`. One entry point.

```solidity
function open(Quote calldata q, bytes calldata signature) external returns (uint256 positionId)
```

In order, it checks:

| check                                 | why                                                                                                                                                   |
| ------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `msg.sender == q.client`              | a price quoted to one counterparty cannot be taken by another                                                                                         |
| `block.timestamp <= q.expiry`         | the dealer priced against spot at one instant                                                                                                         |
| nonce unused for that dealer          | a signature is replayable forever otherwise                                                                                                           |
| signature recovers to `q.dealer`      | the dealer is bound only to what it signed                                                                                                            |
| `s <= n/2`                            | rejects the second valid encoding of the same signature                                                                                               |
| side, notional and tenor are sane     | cheap, and catches a malformed client                                                                                                                 |
| both collateral transfers return true | a token that reports failure by returning false rather than reverting would otherwise leave a position recorded against collateral that never arrived |

Then it consumes the nonce **before** moving value, stores the position, and
pulls collateral from both sides.

**Collateral comes from both sides because the venue is counterparty to both.**
A matched book carries no market risk and full credit risk: if the client fails
at settlement, the dealer is still owed, and the difference comes out of the
venue. Margin is the only defense against that. The dealer approving the
contract in advance is the on-chain shape of a dealer posting margin before it
is allowed to quote.

The EIP-712 domain separator is rebuilt whenever the chain id differs from the
one cached at construction, so a signature from one chain cannot be replayed on
a fork of it. There is a test for exactly that.

## The Rust client

| module         | what it does                                             |
| -------------- | -------------------------------------------------------- |
| `config.rs`    | `config.toml` for public settings, `.env` for secrets    |
| `chain.rs`     | provider construction, chain id verification             |
| `quote.rs`     | builds the EIP-712 quote, scales decimals, signs         |
| `contracts.rs` | bindings generated from the compiled artifacts in `abi/` |
| `main.rs`      | the CLI                                                  |

**Bindings are generated from `forge build` output**, not from a hand-written
interface, so the `Quote` struct and every selector come from the contract that
was actually compiled. `scripts/sync-abi.sh` regenerates them.

That still leaves one way to drift, which is why
`tests/onchain.rs::digest_matches_the_contract` exists: it computes the EIP-712
digest in Rust and calls `hashQuote` on a deployed contract and asserts they
are equal. If the two struct definitions ever diverge, no signature verifies,
and that would otherwise show up as an unexplained revert rather than a failing
test.

**Floats are scaled through a decimal string**, not by multiplying. `5.544602 *
1e18` exceeds the range in which an `f64` holds integers exactly, so the
product is wrong in its low digits.

## Configuration

Public things in `config.toml`: the default network, the auction parameters,
the market, and the deployed addresses per network. Addresses are public and
belong in a committed file.

Secrets in `.env`, which is gitignored: RPC URLs, because they carry provider
API keys, and private keys.

The keys in `.env.example` are anvil's first three accounts. Those are
published in anvil's own documentation, known to everyone, and useless anywhere
but a local node. **Never put a key with real funds in that file.** For Sepolia,
use a wallet holding nothing but test ETH.

## Working against real chain state

There are two networks, `local` and `sepolia`, and no mainnet. These contracts
are unaudited and the collateral token lets anyone mint, so nothing here
belongs on a chain where a mistake costs money.

That costs nothing, because a fork is reached as `local`:

```sh
anvil --fork-url https://...
```

A fork inherits the forked chain's id and serves its real balances and real
contract state with fake money, so the client needs no special handling for it.
`Network::expected_chain_id` returns `None` for local precisely so that a fork
of any chain works.

Sepolia's chain id is pinned, which catches a `SEPOLIA_RPC_URL` pointed
somewhere else. Without that check a wrong URL surfaces as a signature that
will not verify, or a contract that is not there.

The deploy script still refuses chain id 1, since `forge script` takes an RPC
URL straight from the command line and a mistyped one should not be able to put
a public mint function on Ethereum.

## What this does not model

- **Any exit for collateral.** Nothing in the contract moves collateral out,
  so what `open` takes in is locked permanently. Releasing it requires
  settlement, which requires the rules below. Deploy only where the collateral
  token is worthless.
- **Settlement.** `fixingFor` reads the oracle; nothing pays out against it.
  Settling needs rules this contract does not encode: how stale a fixing may be
  before it is unusable, what happens on a local holiday when nothing is
  published, and which ISDA fallback applies when a fixing is disrupted.
  Reading the number is the easy half.
- **A real oracle.** `MockFixingOracle` lets anyone set any rate, which is
  precisely the property a production fixing feed must not have. A real one
  faces staleness, a single source, non-publication, and manipulation of the
  observation window.
- **Variation margin.** Collateral is taken once at open and never adjusted. A
  real position is marked daily and margin moves with it.
- **Liquidation.** No cure window, no auction of a defaulted position, no
  close-out at an oracle mark.
- **Netting.** Each position stands alone. A real book nets exposures per
  counterparty under an ISDA master agreement.
- **Eligibility.** `open` can only be called by the client named in the signed
  quote, but nothing restricts who that client may be. A real venue faces only
  eligible contract participants, which is a documentary check performed before
  trading and would need an allowlist here. Transferable positions would need
  one too, or an eligible holder could pass a position to an ineligible one.
- **Information leakage in the auction.** Dealers cannot see each other, but a
  real venue has to guarantee that, and has to consider whether the request
  itself leaks client intent into the underlying market.
- **Parity pricing in non-deliverable corridors.** Dealers quote off covered
  interest parity, which does not bind where capital controls block the
  enforcing arbitrage. See the scope section of `fx-forward-pricer`.
- **Reporting.** A real swap is reported to a swap data repository.

## License

MIT. See `LICENSE`.
