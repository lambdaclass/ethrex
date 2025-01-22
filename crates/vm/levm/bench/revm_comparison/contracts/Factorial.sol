pragma solidity ^0.8.4;

contract Factorial {
    function factorial(uint256 n) public pure returns (uint256 result) {
        if (n == 0 || n == 1) return 1;

        uint256 i = 2; // starting from 2
        while(i <= n){
            n *= i++;
        }

       result=n;
    }
    fallback() external payable {
    }
}
