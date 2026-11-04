// SPDX-License-Identifier: Apache-2.0
// @openshell egress rpc.example.org:443 reason="sample JSON-RPC endpoint" protocol=json-rpc allow="POST:/"
pragma solidity ^0.8.0;

contract Oracle {
    function ping() external pure returns (uint256) {
        return 1;
    }
}
