# Hardhat (ethrex)

This folder wires Hardhat to the Solidity sources in `crates/l2/contracts/src`.

## Setup (matches CI)

From the repo root:

```sh
npm install
```

Then install the Hardhat workspace dependencies and compile:

```sh
cd tooling/hardhat
npm install
npm run compile
```

If you want to avoid Hardhat downloading solc, install solc 0.8.31 and set:

```sh
export HARDHAT_USE_NATIVE_SOLC=true
```

## Run locally

Run all Hardhat tests (currently the local upgradeability example):

```sh
npm test
```

Run only the upgradeability example:

```sh
npm run test:upgrade
```

This uses the dummy Box contracts and expects the incompatible upgrade to be rejected.

CI-style upgrade comparison against a reference build-info directory:

```sh
UPGRADE_REFERENCE_BUILD_INFO_DIR=/path/to/build-info-ref npm run validate:upgrades
```

## CI

The workflow in `.github/workflows/pr_upgradeability.yaml` runs on PRs and compares
upgradeable contracts against `main`. It compiles Hardhat in the PR and in a
`main` worktree, copies the `build-info` from `main`, and runs
`npm run validate:upgrades` to check storage layout compatibility.

## Environment overrides

- `HARDHAT_USE_NATIVE_SOLC=true` (skip solc downloads if you have solc 0.8.31 installed)
- `UPGRADE_REFERENCE_BUILD_INFO_DIR=/path/to/build-info-ref` (used by `npm run validate:upgrades`)
