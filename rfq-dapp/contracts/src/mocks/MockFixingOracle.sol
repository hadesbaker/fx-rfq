// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IFixingOracle} from "../interfaces/IFixingOracle.sol";

/// @notice Stands in for a real fixing feed. Test and local use only: anyone
///         can set any rate, which is precisely the property a production
///         oracle must not have.
contract MockFixingOracle is IFixingOracle {
    mapping(bytes32 => uint256) private _rate;
    mapping(bytes32 => uint64) private _publishedAt;

    function setFixing(bytes32 pair, uint256 rate) external {
        _rate[pair] = rate;
        _publishedAt[pair] = uint64(block.timestamp);
    }

    function latestFixing(bytes32 pair) external view returns (uint256, uint64) {
        return (_rate[pair], _publishedAt[pair]);
    }
}
