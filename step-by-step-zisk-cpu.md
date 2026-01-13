# Step by Step - CPU

0. Compile ethrex:

    ```bash
    COMPILE_CONTRACTS=true cargo build --release --bin ethrex --features l2,zisk
    ```

    This will generate the elf in `crates/l2/prover/src/guest_program/src/zisk/out/riscv64ima-zisk-elf`

1. clone zisk:

    ```bash
    git clone git@github.com:0xPolygonHermez/zisk.git
    ```

2. checkout to the custom branch:

    ```bash
    cd zisk && git checkout feature/bn128
    ```

3. Build ZisK:

    ```bash
    cargo build --release
    ```

4. Follow step from 3 to 7 in the [installation guide](https://github.com/0xPolygonHermez/zisk/blob/feature/bn128/book/getting_started/installation.md#option-2-building-from-source) from the building from source

5. Download the public keys:

    ```bash
    curl -LO https://storage.googleapis.com/zisk-setup/zisk-0.15.0-plonk.tar.gz
    ```

6. Extract the keys:

    ```bash
    tar -xvzf zisk-0.15.0-plonk.tar.gz -C zisk-pkey
    ```

7. copy the keys to the path:

    ```bash
    cd zisk-pkey
    cp -r provingKey ~/.zisk
    cp -r provingKeySnark ~/.zisk
    ```

    From now on, for the paths used in the commands do not add a `/` at the end.

8. Do the rom setup:

    ```bash
    cargo-zisk rom-setup -e <PATH_TO_ELF> -k ~/.zisk/provingKey
    ```

9. Check the setup:

    ```bash
    cargo-zisk check-setup -k ~/.zisk/provingKey -a
    ```

10. Export libs:

    ```bash
    export LD_LIBRARY_PATH=/home/admin/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/x86_64-unknown-linux-gnu/lib
    ```

11. Prove an input:

    ```bash
    cargo-zisk prove -e <PATH_TO_ELF> -i <PATH_TO_INPUT> -a -u -f -k ~/.zisk/provingKey -w <PATH_TO_ZISK_REPO>/target/release/libzisk_witness.so -o <OUTPUT_PATH>
    ```

12. Generate the VK:

    ```bash
    cargo-zisk rom-vkey -e <PATH_TO_ELF> -k ~/.zisk/provingKey -o <OUTPUT_PATH>/riscv64ima-zisk-vk
    ```

    This will also print the VK, example:

    ```
    ...
    INFO: Root hash: [3121973382251281428, 1947533496960916486, 15830689218699704550, 16339664693968653792]
    ...
    ```

13. Generate the snark proof:

    ```bash
    cargo-zisk prove-snark -k ~/.zisk/provingKeySnark -p <OUTPUT_PATH>/vadcop_final_proof.bin -o <OUTPUT_PATH>
    ```

    In here we have to already created a sub directory `proofs` inside `<OUTPUT_PATH>`:

    ```bash
    mkdir -p <OUTPUT_PATH>/proofs
    ```

    If you encounter an error like this:

    ```
    Caused by:
      0: Dynamic library error: /home/admin/.zisk/provingKeySnark/final/final.so: cannot enable executable stack as shared object requires: Invalid argument
      1: /home/admin/.zisk/provingKeySnark/final/final.so: cannot enable executable stack as shared object requires: Invalid argument
    ```

    Run:

    ```bash
    patchelf --clear-execstack ~/.zisk/provingKeySnark/final/final.so
    ```

    Then, generating the proof may lead to an error like this:

    ```
    Failed assert in template/function VerifyMerkleHash line 51. Followed trace of components: main.sV.VerifyMerkleHash_4302_268326[22]
    cargo-zisk: verifier.cpp:1688648: void VerifyMerkleHash_1708_run_parallel(uint, Circom_CalcWit*): Assertion `Fr_isTrue(&expaux[0])' failed.
    Aborted
    ```

    This will solve if you just run again the command

14. Compile the verifier contract:

    ```bash
    cd <PATH_TO_ZISK_REPO>

    solc --optimize --abi --bin \
        --base-path . --include-path zisk-contracts --allow-paths . \
        -o build/zisk-contracts --overwrite \
        zisk-contracts/ZiskVerifier.sol
    ```

15. Deploy the contract:

    ```bash
    rex deploy 0 0xe4f7dc8b199fdaac6693c9c412ea68aed9e1584d193e1c3478d30a6f01f26057 --bytecode `cat build/zisk-contracts/ZiskVerifier.bin` --rpc-url http://localhost:8545
    ```

    This will print something like this:

    ```bash
    Contract deployed in tx: 0x6e0f3ad8b0e837e835a9b1af83623c8865bef43a5cb111bb01889c8e2cc80d7a
    Contract address: 0xa0c79e7f98c9914c337d5b010af208b98f23f117
    ```

    This should not return an empty string:

    ```bash
    rex call 0xa0c79e7f98c9914c337d5b010af208b98f23f117 "VERSION()"
    ```

16. Verify the proof:

    ```bash
    cast call 0xa0c79e7f98c9914c337d5b010af208b98f23f117 \
        "verifySnarkProof(uint64[4],bytes,bytes)" \
        "[3121973382251281428,1947533496960916486,15830689218699704550,16339664693968653792]" \
        `cat zisk-output/proofs/final_snark_publics.hex` \
        `cat zisk-output/proofs/final_snark_proof.hex`
    ```

    This should not revert and return `0x`
