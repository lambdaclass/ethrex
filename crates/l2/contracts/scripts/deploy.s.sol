// SPDX-License-Identifier: UNLICENSED
pragma solidity =0.8.29;

import {Script, console} from "forge-std/Script.sol";
import {console} from "forge-std/console.sol";
import {OnChainProposer} from "src/l1/OnChainProposer.sol";
import {Upgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";

contract Deployer is Script {
    OnChainProposer public onChainProposer;

    function setUp() public {}

    function run(address sequencer, bool validium) public {
        vm.startBroadcast();

        address[] memory sequencers = new address[](1);
        sequencers[0] = sequencer;

        address proxy = Upgrades.deployUUPSProxy(
            "OnChainProposer.sol",
            abi.encodeCall(
                OnChainProposer.initialize,
                (
                    validium,
                    0x7F8E01504e83F0422B7a3D2903292aE65918d065,
                    0x00000000000000000000000000000000000000AA,
                    0x00000000000000000000000000000000000000AA,
                    0x00000000000000000000000000000000000000AA,
                    sequencers
                )
            )
        );
        console.log("Proxy address: ", proxy);

        vm.stopBroadcast();
    }
}
