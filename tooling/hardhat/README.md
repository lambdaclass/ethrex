# Hardhat (ethrex)

This folder wires Hardhat to the Solidity sources in `crates/l2/contracts/src`.

## Setup

```sh
cd tooling/hardhat
npm install
```

Install root dependencies so Hardhat can resolve `@openzeppelin/...` imports:

```sh
cd ../..
npm install
```

If you want to avoid Hardhat downloading solc, install solc 0.8.31 and set:

```sh
export HARDHAT_USE_NATIVE_SOLC=true
```

## Run

```sh
npm test
npm run test:l1
npm run test:l2
```

## Environment overrides

- `ETHREX_L1_RPC_URL` (default `http://127.0.0.1:8545`)
- `ETHREX_L2_RPC_URL` (default `http://127.0.0.1:1729`)
- `ETHREX_L1_CHAIN_ID` (default `9`)
- `ETHREX_L2_CHAIN_ID` (default `65536999`)
- `ETHREX_KEYS_FILE` or `ETHREX_L1_KEYS_FILE` / `ETHREX_L2_KEYS_FILE`

By default, the keys are loaded from `fixtures/keys/private_keys_tests.txt`.
