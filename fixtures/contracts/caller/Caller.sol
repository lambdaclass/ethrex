// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

contract Caller {
  function doCall(address to, bytes calldata data) public {
    (bool success, ) = to.call(data);
    require(success, "Caller: call reverted");
  }
}
