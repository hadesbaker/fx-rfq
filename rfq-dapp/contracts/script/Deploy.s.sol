// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script} from "forge-std/Script.sol";
import {console} from "forge-std/console.sol";
import {RfqSettlement, IERC20} from "../src/RfqSettlement.sol";
import {IFixingOracle} from "../src/interfaces/IFixingOracle.sol";
import {MockUSDC} from "../src/mocks/MockUSDC.sol";
import {MockFixingOracle} from "../src/mocks/MockFixingOracle.sol";

/// @notice Deploys the mock collateral token, the mock fixing oracle and the
///         settlement contract.
///
/// @dev Local and Sepolia only. The mocks let anyone mint collateral and set
///      any fixing, so this must never be deployed to a network where either
///      would matter. The script refuses to run on mainnet.
contract Deploy is Script {
    error NotForMainnet();

    function run() external {
        if (block.chainid == 1) revert NotForMainnet();

        vm.startBroadcast();

        MockUSDC usdc = new MockUSDC();
        MockFixingOracle oracle = new MockFixingOracle();
        RfqSettlement rfq = new RfqSettlement(IERC20(address(usdc)), IFixingOracle(address(oracle)));

        vm.stopBroadcast();

        console.log("chain id        ", block.chainid);
        console.log("MockUSDC        ", address(usdc));
        console.log("MockFixingOracle", address(oracle));
        console.log("RfqSettlement   ", address(rfq));
        console.log("");
        console.log("Put these in .env:");
        console.log("  SETTLEMENT_ADDRESS=", address(rfq));
        console.log("  COLLATERAL_ADDRESS=", address(usdc));
        console.log("  ORACLE_ADDRESS=", address(oracle));
    }
}
