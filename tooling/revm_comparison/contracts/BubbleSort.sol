// SPDX-License-Identifier: MIT
pragma solidity ^0.8.17;

contract BubbleSort {
    uint256[] public numbers;

    function Benchmark(uint256 amount) public returns (uint8 result) {
        // Fill array with random numbers
        for (uint256 i = 0; i < amount; i++) {
            numbers.push(uint256(keccak256(abi.encodePacked(block.timestamp, block.prevrandao, i))) % 100);
        }
        uint256 n = numbers.length;
        for (uint256 i = 0; i < n - 1; i++) {
            for (uint256 j = 0; j < n - i - 1; j++) {
                if (numbers[j] > numbers[j + 1]) {
                    (numbers[j], numbers[j + 1]) = (numbers[j + 1], numbers[j]);
                }
            }
        }
        // Ensure the array is sorted
        for (uint256 i = 0; i < n - 1; i++) {
            require(numbers[i] <= numbers[i + 1]);
        }
        return 0;
    }
}
