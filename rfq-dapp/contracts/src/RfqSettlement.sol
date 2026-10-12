// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IFixingOracle} from "./interfaces/IFixingOracle.sol";

interface IERC20 {
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}

/// @title  On-chain settlement for an off-chain RFQ auction
/// @notice Dealers compete off chain and the winner signs its price. The client
///         brings that signature here, where it is checked and the position is
///         recorded with collateral from both sides.
///
/// @dev Why the auction is not on chain, and it is not latency. On a chain
///      with sub-second finality, latency is not the binding constraint.
///
///      A quote is an option the dealer gives away for free: the client may
///      take it or leave it. Eight dealers quote and one wins, so seven of
///      those options expire worthless. Paying consensus costs to mint
///      options that mostly die is waste, and it does not get cheaper as the
///      chain gets faster.
///
///      A dealer also has to withdraw a price faster than the market moves,
///      and on chain the floor on that is block cadence plus finality. Off
///      chain there is no floor. Quoting on chain also means every dealer
///      runs a funded hot wallet and pays for quotes that mostly lose, which
///      changes who is willing to make markets at all.
///
///      So the auction runs off chain and only the outcome is settled. What
///      that costs is trust in a signature, and what it buys is a price that
///      was real when it was given. The signature is the whole boundary: a
///      dealer cannot deny a quote it signed, and cannot be bound to one it
///      did not.
///
///      Collateral is taken from both sides because the venue is counterparty
///      to both. A matched book carries no market risk and full credit risk,
///      so margin is the only defense against one side failing while the other
///      is still owed.
///
///      There is no exit. Nothing in this contract moves collateral out, so
///      what `open` takes in is locked permanently. Releasing it requires
///      settlement, and settlement requires the fixing rules this contract
///      deliberately does not encode: staleness limits, local holidays on
///      which nothing is published, and the ISDA fallback for a disrupted
///      fixing. Deploy this only where the collateral token is worthless.
contract RfqSettlement {
    // ---------------------------------------------------------------- types

    /// @param dealer     who signed the price and is bound by it
    /// @param client     the only address permitted to accept it
    /// @param pair       corridor, e.g. bytes32("USD/BRL")
    /// @param side       0 = client buys the base currency, 1 = client sells it
    /// @param notional   base currency units, scaled by 1e18
    /// @param rate       quote units per base unit, scaled by 1e18
    /// @param tenorDays  days from acceptance to the fixing date
    /// @param collateral collateral token units required from EACH side
    /// @param expiry     unix seconds after which the price is dead
    /// @param nonce      per-dealer, single use
    struct Quote {
        address dealer;
        address client;
        bytes32 pair;
        uint8 side;
        uint256 notional;
        uint256 rate;
        uint32 tenorDays;
        uint256 collateral;
        uint64 expiry;
        uint256 nonce;
    }

    struct Position {
        address dealer;
        address client;
        bytes32 pair;
        uint8 side;
        uint256 notional;
        uint256 rate;
        uint256 collateral;
        uint64 openedAt;
        uint64 fixingDate;
        bool settled;
    }

    // ------------------------------------------------------------ constants

    bytes32 public constant QUOTE_TYPEHASH = keccak256(
        "Quote(address dealer,address client,bytes32 pair,uint8 side,uint256 notional,uint256 rate,uint32 tenorDays,uint256 collateral,uint64 expiry,uint256 nonce)"
    );

    bytes32 private constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");

    bytes32 private constant NAME_HASH = keccak256("CRX RFQ Settlement");
    bytes32 private constant VERSION_HASH = keccak256("1");

    /// @dev Half the secp256k1 curve order. Signatures with a larger `s` are
    ///      the second valid encoding of the same signature, so they are
    ///      rejected rather than allowed to produce a second digest.
    uint256 private constant HALF_CURVE_ORDER =
        0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0;

    // -------------------------------------------------------------- storage

    IERC20 public immutable collateralToken;
    IFixingOracle public immutable fixingOracle;

    uint256 private immutable _cachedChainId;
    bytes32 private immutable _cachedDomainSeparator;

    Position[] public positions;

    /// @dev dealer => nonce => consumed. Scoped per dealer so two dealers
    ///      choosing the same nonce do not collide.
    mapping(address => mapping(uint256 => bool)) public nonceUsed;

    // --------------------------------------------------------------- events

    event PositionOpened(
        uint256 indexed positionId,
        address indexed dealer,
        address indexed client,
        bytes32 pair,
        uint8 side,
        uint256 notional,
        uint256 rate,
        uint256 collateral,
        uint64 fixingDate
    );

    // --------------------------------------------------------------- errors

    error NotTheClient(address expected, address caller);
    error PriceExpired(uint64 expiry, uint256 blockTimestamp);
    error NonceAlreadyUsed(address dealer, uint256 nonce);
    error SignerIsNotTheDealer(address recovered, address dealer);
    error MalleableSignature();
    error BadSignatureLength(uint256 length);
    error InvalidSide(uint8 side);
    error ZeroNotional();
    error ZeroTenor();
    error CollateralTransferFailed(address from);

    // ---------------------------------------------------------- constructor

    constructor(IERC20 collateralToken_, IFixingOracle fixingOracle_) {
        collateralToken = collateralToken_;
        fixingOracle = fixingOracle_;
        _cachedChainId = block.chainid;
        _cachedDomainSeparator = _buildDomainSeparator();
    }

    // ------------------------------------------------------------- external

    /// @notice Accept a dealer-signed price and open the position.
    /// @dev Only `q.client` may call. Collateral is pulled from the caller and
    ///      from the dealer, so the dealer must have approved this contract in
    ///      advance. That is the on-chain shape of a dealer posting margin
    ///      before it is allowed to quote.
    function open(Quote calldata q, bytes calldata signature) external returns (uint256 positionId) {
        if (msg.sender != q.client) revert NotTheClient(q.client, msg.sender);
        if (block.timestamp > q.expiry) revert PriceExpired(q.expiry, block.timestamp);
        if (nonceUsed[q.dealer][q.nonce]) revert NonceAlreadyUsed(q.dealer, q.nonce);
        if (q.side > 1) revert InvalidSide(q.side);
        if (q.notional == 0) revert ZeroNotional();
        if (q.tenorDays == 0) revert ZeroTenor();

        address recovered = _recover(hashQuote(q), signature);
        if (recovered != q.dealer) revert SignerIsNotTheDealer(recovered, q.dealer);

        // Consume the nonce before moving value, so a reentrant call cannot
        // replay this quote.
        nonceUsed[q.dealer][q.nonce] = true;

        uint64 fixingDate = uint64(block.timestamp + uint256(q.tenorDays) * 1 days);

        positionId = positions.length;
        positions.push(
            Position({
                dealer: q.dealer,
                client: q.client,
                pair: q.pair,
                side: q.side,
                notional: q.notional,
                rate: q.rate,
                collateral: q.collateral,
                openedAt: uint64(block.timestamp),
                fixingDate: fixingDate,
                settled: false
            })
        );

        // The return value is checked rather than discarded. A token that
        // signals failure by returning false instead of reverting would
        // otherwise leave a position recorded against collateral that never
        // arrived.
        if (q.collateral > 0) {
            if (!collateralToken.transferFrom(q.client, address(this), q.collateral)) {
                revert CollateralTransferFailed(q.client);
            }
            if (!collateralToken.transferFrom(q.dealer, address(this), q.collateral)) {
                revert CollateralTransferFailed(q.dealer);
            }
        }

        emit PositionOpened(
            positionId, q.dealer, q.client, q.pair, q.side, q.notional, q.rate, q.collateral, fixingDate
        );
    }

    // ----------------------------------------------------------------- view

    function positionCount() external view returns (uint256) {
        return positions.length;
    }

    /// @notice The current published fixing for a corridor, straight from the
    ///         oracle.
    /// @dev Settlement is deliberately not implemented. Paying out against a
    ///      fixing needs rules this contract does not yet encode: how stale a
    ///      fixing may be before it is unusable, what happens on a local
    ///      holiday when nothing is published, and which ISDA fallback applies
    ///      when a fixing is disrupted. Reading the number is the easy half.
    function fixingFor(bytes32 pair) external view returns (uint256 rate, uint64 publishedAt) {
        return fixingOracle.latestFixing(pair);
    }

    /// @notice EIP-712 domain separator, rebuilt if the chain id has changed
    ///         so a fork cannot replay signatures from the original chain.
    function domainSeparator() public view returns (bytes32) {
        if (block.chainid == _cachedChainId) return _cachedDomainSeparator;
        return _buildDomainSeparator();
    }

    /// @notice The EIP-712 digest a dealer signs.
    function hashQuote(Quote calldata q) public view returns (bytes32) {
        bytes32 structHash = keccak256(
            abi.encode(
                QUOTE_TYPEHASH,
                q.dealer,
                q.client,
                q.pair,
                q.side,
                q.notional,
                q.rate,
                q.tenorDays,
                q.collateral,
                q.expiry,
                q.nonce
            )
        );
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator(), structHash));
    }

    // -------------------------------------------------------------- private

    function _buildDomainSeparator() private view returns (bytes32) {
        return keccak256(abi.encode(DOMAIN_TYPEHASH, NAME_HASH, VERSION_HASH, block.chainid, address(this)));
    }

    function _recover(bytes32 digest, bytes calldata signature) private pure returns (address) {
        if (signature.length != 65) revert BadSignatureLength(signature.length);

        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly {
            r := calldataload(signature.offset)
            s := calldataload(add(signature.offset, 32))
            v := byte(0, calldataload(add(signature.offset, 64)))
        }

        if (uint256(s) > HALF_CURVE_ORDER) revert MalleableSignature();

        address recovered = ecrecover(digest, v, r, s);
        if (recovered == address(0)) revert SignerIsNotTheDealer(address(0), address(0));
        return recovered;
    }
}
