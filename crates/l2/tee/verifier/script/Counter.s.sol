// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script, console} from "forge-std/Script.sol";
import {Counter} from "../src/Counter.sol";

contract CounterScript is Script {
    Counter public counter;
    address dcap = vm.envAddress("DCAP_ADDRESS");

    function setUp() public {}

    function run() public {
        vm.startBroadcast();

        counter = new Counter(dcap);

        vm.stopBroadcast();
    }
}
