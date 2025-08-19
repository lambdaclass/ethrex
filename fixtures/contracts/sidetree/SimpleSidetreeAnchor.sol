/*
 * Copyright 2020 - Transmute Industries Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *     http://www.apache.org/licenses/LICENSE-2.0
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

pragma solidity 0.8.29;

contract SimpleSidetreeAnchor {
    uint256 public transactionNumber = 0;

    event Anchor(
        bytes32 anchorFileHash,
        uint256 indexed transactionNumber,
        uint256 numberOfOperations,
        address writer
    );

    function anchorHash(bytes32 _anchorHash, uint256 _numberOfOperations)
        public
    {
        emit Anchor(_anchorHash, transactionNumber, _numberOfOperations , msg.sender);
        transactionNumber = transactionNumber + 1;
    }
}
