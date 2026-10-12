// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {RfqSettlement, IERC20} from "../src/RfqSettlement.sol";
import {IFixingOracle} from "../src/interfaces/IFixingOracle.sol";
import {MockUSDC} from "../src/mocks/MockUSDC.sol";
import {MockFixingOracle} from "../src/mocks/MockFixingOracle.sol";

contract RfqSettlementTest is Test {
    RfqSettlement internal rfq;
    MockUSDC internal usdc;
    MockFixingOracle internal oracle;

    uint256 internal constant DEALER_KEY = 0xA11CE;
    uint256 internal constant OTHER_KEY = 0xB0B;
    address internal dealer;
    address internal client = address(0xC11E47);

    bytes32 internal constant PAIR = bytes32("USD/BRL");
    uint256 internal constant COLLATERAL = 500_000e6; // 500k mUSDC per side

    function setUp() public {
        dealer = vm.addr(DEALER_KEY);
        usdc = new MockUSDC();
        oracle = new MockFixingOracle();
        rfq = new RfqSettlement(IERC20(address(usdc)), IFixingOracle(address(oracle)));

        usdc.mint(client, 10_000_000e6);
        usdc.mint(dealer, 10_000_000e6);
        vm.prank(client);
        usdc.approve(address(rfq), type(uint256).max);
        vm.prank(dealer);
        usdc.approve(address(rfq), type(uint256).max);
    }

    function _quote() internal view returns (RfqSettlement.Quote memory) {
        return RfqSettlement.Quote({
            dealer: dealer,
            client: client,
            pair: PAIR,
            side: 0, // client buys the base currency
            notional: 10_000_000e18,
            rate: 5.544602e18,
            tenorDays: 122,
            collateral: COLLATERAL,
            expiry: uint64(block.timestamp + 3),
            nonce: 1
        });
    }

    /// @dev Always bind the result before a `vm.prank`. This reads
    ///      `rfq.hashQuote`, and that external call consumes a pending prank,
    ///      so signing inline inside `open(...)` leaves the prank spent and
    ///      the caller is the test contract.
    function _sign(RfqSettlement.Quote memory q, uint256 key) internal view returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, rfq.hashQuote(q));
        return abi.encodePacked(r, s, v);
    }

    // ---------------------------------------------------------- happy path

    function test_open_records_the_position() public {
        RfqSettlement.Quote memory q = _quote();
        bytes memory sig = _sign(q, DEALER_KEY);
        vm.prank(client);
        uint256 id = rfq.open(q, sig);

        assertEq(id, 0);
        assertEq(rfq.positionCount(), 1);

        (
            address d,
            address c,
            bytes32 pair,
            uint8 side,
            uint256 notional,
            uint256 rate,
            uint256 collateral,
            uint64 openedAt,
            uint64 fixingDate,
            bool settled
        ) = rfq.positions(0);

        assertEq(d, dealer);
        assertEq(c, client);
        assertEq(pair, PAIR);
        assertEq(side, 0);
        assertEq(notional, 10_000_000e18);
        assertEq(rate, 5.544602e18);
        assertEq(collateral, COLLATERAL);
        assertEq(openedAt, uint64(block.timestamp));
        assertEq(fixingDate, uint64(block.timestamp + 122 days));
        assertFalse(settled);
    }

    function test_open_takes_collateral_from_both_sides() public {
        uint256 clientBefore = usdc.balanceOf(client);
        uint256 dealerBefore = usdc.balanceOf(dealer);

        RfqSettlement.Quote memory q = _quote();
        bytes memory sig = _sign(q, DEALER_KEY);
        vm.prank(client);
        rfq.open(q, sig);

        assertEq(usdc.balanceOf(client), clientBefore - COLLATERAL);
        assertEq(usdc.balanceOf(dealer), dealerBefore - COLLATERAL);
        assertEq(usdc.balanceOf(address(rfq)), COLLATERAL * 2);
    }

    function test_two_dealers_may_reuse_a_nonce() public {
        RfqSettlement.Quote memory q = _quote();
        bytes memory sig = _sign(q, DEALER_KEY);
        vm.prank(client);
        rfq.open(q, sig);

        // Same nonce, different dealer. Nonces are scoped per dealer.
        address other = vm.addr(OTHER_KEY);
        usdc.mint(other, 10_000_000e6);
        vm.prank(other);
        usdc.approve(address(rfq), type(uint256).max);

        RfqSettlement.Quote memory q2 = _quote();
        q2.dealer = other;
        bytes memory sig2 = _sign(q2, OTHER_KEY);
        vm.prank(client);
        rfq.open(q2, sig2);

        assertEq(rfq.positionCount(), 2);
    }

    // -------------------------------------------------------------- guards

    function test_only_the_named_client_may_accept() public {
        RfqSettlement.Quote memory q = _quote();
        bytes memory sig = _sign(q, DEALER_KEY);

        vm.prank(address(0xDEAD));
        vm.expectRevert(
            abi.encodeWithSelector(RfqSettlement.NotTheClient.selector, client, address(0xDEAD))
        );
        rfq.open(q, sig);
    }

    function test_expired_price_is_refused() public {
        RfqSettlement.Quote memory q = _quote();
        bytes memory sig = _sign(q, DEALER_KEY);

        vm.warp(block.timestamp + 10);
        vm.prank(client);
        vm.expectRevert(
            abi.encodeWithSelector(RfqSettlement.PriceExpired.selector, q.expiry, block.timestamp)
        );
        rfq.open(q, sig);
    }

    function test_a_quote_cannot_be_replayed() public {
        RfqSettlement.Quote memory q = _quote();
        bytes memory sig = _sign(q, DEALER_KEY);

        vm.prank(client);
        rfq.open(q, sig);

        vm.prank(client);
        vm.expectRevert(
            abi.encodeWithSelector(RfqSettlement.NonceAlreadyUsed.selector, dealer, q.nonce)
        );
        rfq.open(q, sig);
    }

    function test_a_signature_from_anyone_else_is_refused() public {
        RfqSettlement.Quote memory q = _quote();
        bytes memory sig = _sign(q, OTHER_KEY);

        vm.prank(client);
        vm.expectRevert(
            abi.encodeWithSelector(
                RfqSettlement.SignerIsNotTheDealer.selector, vm.addr(OTHER_KEY), dealer
            )
        );
        rfq.open(q, sig);
    }

    function test_altering_the_rate_after_signing_is_refused() public {
        RfqSettlement.Quote memory q = _quote();
        bytes memory sig = _sign(q, DEALER_KEY);

        // The client shaves a pip off the price it was given.
        q.rate = 5.544502e18;

        vm.prank(client);
        vm.expectRevert();
        rfq.open(q, sig);
    }

    function test_malleable_signature_is_refused() public {
        RfqSettlement.Quote memory q = _quote();
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(DEALER_KEY, rfq.hashQuote(q));

        // The second valid encoding of the same signature: s' = n - s, v flipped.
        uint256 n = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141;
        bytes32 flippedS = bytes32(n - uint256(s));
        uint8 flippedV = v == 27 ? 28 : 27;

        vm.prank(client);
        vm.expectRevert(RfqSettlement.MalleableSignature.selector);
        rfq.open(q, abi.encodePacked(r, flippedS, flippedV));
    }

    function test_signature_length_is_checked() public {
        RfqSettlement.Quote memory q = _quote();
        vm.prank(client);
        vm.expectRevert(abi.encodeWithSelector(RfqSettlement.BadSignatureLength.selector, 10));
        rfq.open(q, hex"00112233445566778899");
    }

    function test_side_must_be_zero_or_one() public {
        RfqSettlement.Quote memory q = _quote();
        q.side = 2;
        bytes memory sig = _sign(q, DEALER_KEY);

        vm.prank(client);
        vm.expectRevert(abi.encodeWithSelector(RfqSettlement.InvalidSide.selector, 2));
        rfq.open(q, sig);
    }

    function test_zero_notional_is_refused() public {
        RfqSettlement.Quote memory q = _quote();
        q.notional = 0;
        bytes memory sig = _sign(q, DEALER_KEY);

        vm.prank(client);
        vm.expectRevert(RfqSettlement.ZeroNotional.selector);
        rfq.open(q, sig);
    }

    function test_zero_tenor_is_refused() public {
        RfqSettlement.Quote memory q = _quote();
        q.tenorDays = 0;
        bytes memory sig = _sign(q, DEALER_KEY);

        vm.prank(client);
        vm.expectRevert(RfqSettlement.ZeroTenor.selector);
        rfq.open(q, sig);
    }

    // ------------------------------------------------------------- EIP-712

    function test_domain_separator_is_chain_specific() public {
        bytes32 here = rfq.domainSeparator();
        vm.chainId(999);
        assertTrue(rfq.domainSeparator() != here, "a fork must not inherit signatures");
    }

    function test_signature_does_not_survive_a_fork() public {
        RfqSettlement.Quote memory q = _quote();
        bytes memory sig = _sign(q, DEALER_KEY);

        vm.chainId(999);
        vm.prank(client);
        vm.expectRevert();
        rfq.open(q, sig);
    }

    // -------------------------------------------------------------- oracle

    function test_fixing_is_read_through_the_oracle() public {
        oracle.setFixing(PAIR, 5.61e18);
        (uint256 rate, uint64 publishedAt) = rfq.fixingFor(PAIR);
        assertEq(rate, 5.61e18);
        assertEq(publishedAt, uint64(block.timestamp));
    }

    function test_an_unpublished_corridor_reads_zero() public view {
        (uint256 rate, uint64 publishedAt) = rfq.fixingFor(bytes32("USD/INR"));
        assertEq(rate, 0);
        assertEq(publishedAt, 0);
    }
}

/// @notice A token that signals failure by returning false instead of
///         reverting. Non-compliant but real, and the reason `open` checks
///         the return value.
contract FalseReturningToken {
    function transferFrom(address, address, uint256) external pure returns (bool) {
        return false;
    }
    function balanceOf(address) external pure returns (uint256) {
        return type(uint256).max;
    }
}

contract CollateralFailureTest is Test {
    RfqSettlement internal rfq;
    uint256 internal constant DEALER_KEY = 0xA11CE;
    address internal dealer;
    address internal client = address(0xC11E47);

    function setUp() public {
        dealer = vm.addr(DEALER_KEY);
        rfq = new RfqSettlement(
            IERC20(address(new FalseReturningToken())),
            IFixingOracle(address(new MockFixingOracle()))
        );
    }

    function test_a_silent_transfer_failure_reverts() public {
        RfqSettlement.Quote memory q = RfqSettlement.Quote({
            dealer: dealer,
            client: client,
            pair: bytes32("USD/BRL"),
            side: 0,
            notional: 10_000_000e18,
            rate: 5.5e18,
            tenorDays: 122,
            collateral: 500_000e6,
            expiry: uint64(block.timestamp + 3),
            nonce: 1
        });
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(DEALER_KEY, rfq.hashQuote(q));
        bytes memory sig = abi.encodePacked(r, s, v);

        vm.prank(client);
        vm.expectRevert(
            abi.encodeWithSelector(RfqSettlement.CollateralTransferFailed.selector, client)
        );
        rfq.open(q, sig);
    }
}
