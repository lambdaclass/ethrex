// Copyright 2024 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0
//
// NOTICE:
//  Modified from the original file. 
//  Making use of the IRiscZeroVerifier interface and nothing else.

pragma solidity ^0.8.9;

/// @notice A receipt attesting to the execution of a guest program.
/// @dev A receipt contains two parts: a seal and a claim. The seal is a zero-knowledge proof
/// attesting to knowledge of a zkVM execution resulting in the claim. The claim is a set of public
/// outputs for the execution. Crucially, the claim includes the journal and the image ID. The
/// image ID identifies the program that was executed, and the journal is the public data written
/// by the program. Note that this struct only contains the claim digest, as can be obtained with
/// the `digest()` function on `ReceiptClaimLib`.
struct Receipt {
    bytes seal;
    bytes32 claimDigest;
}

/// @notice Error raised when cryptographic verification of the zero-knowledge proof fails.
error VerificationFailed();

/// @notice Verifier interface for RISC Zero receipts of execution.
interface IRiscZeroVerifier {
    /// @notice Verify that the given seal is a valid RISC Zero proof of execution with the
    ///     given image ID and journal digest. Reverts on failure.
    /// @dev This method additionally ensures that the input hash is all-zeros (i.e. no
    /// committed input), the exit code is (Halted, 0), and there are no assumptions (i.e. the
    /// receipt is unconditional).
    /// @param seal The encoded cryptographic proof (i.e. SNARK).
    /// @param imageId The identifier for the guest program.
    /// @param journalDigest The SHA-256 digest of the journal bytes.
    function verify(bytes calldata seal, bytes32 imageId, bytes32 journalDigest) external view;
}
