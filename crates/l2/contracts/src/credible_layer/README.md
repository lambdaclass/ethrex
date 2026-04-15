# Credible Layer Contracts

This directory contains example/demo contracts for the Credible Layer integration:

- `OwnableTarget.sol` — a simple Ownable contract used as a demonstration target
- `TestOwnershipAssertion.sol` — a test assertion that asserts ownership of `OwnableTarget` cannot change

## State Oracle

The **State Oracle** is the on-chain registry that maps protected contracts to their active
assertions. It is maintained by Phylax Systems and must be deployed separately using the
Phylax toolchain before starting the Credible Layer sidecar.

### Contract Source

The State Oracle and its dependencies live in the `credible-layer-contracts` repository:

> https://github.com/phylaxsystems/credible-layer-contracts

The relevant contracts are:

| Contract | Purpose |
|----------|---------|
| `StateOracle` | Core registry: maps protected contracts to assertions |
| `DAVerifierECDSA` | Verifies ECDSA-signed assertion DA payloads |
| `DAVerifierOnChain` | Verifies on-chain DA payloads |
| `AdminVerifierOwner` | Restricts assertion registration to contract owner |

The `StateOracle` constructor signature is:

```solidity
constructor(uint256 assertionTimelockBlocks) Ownable(msg.sender)
```

And initialization (called after proxy deployment):

```solidity
function initialize(
    address admin,
    IAdminVerifier[] calldata _adminVerifiers,
    IDAVerifier[] calldata _daVerifiers,
    uint16 _maxAssertionsPerAA
) external
```

### Deploying the State Oracle

Use the Phylax `pcl` CLI or the Foundry deployment scripts provided in
`credible-layer-contracts`:

```bash
# Install pcl
brew tap phylaxsystems/pcl
brew install pcl

# Or use Foundry scripts from the credible-layer-contracts repo
git clone https://github.com/phylaxsystems/credible-layer-contracts
cd credible-layer-contracts
forge script script/DeployStateOracle.s.sol --rpc-url <L2_RPC_URL> --broadcast
```

Once deployed, note the State Oracle address and pass it to ethrex via the
`--credible-layer-state-oracle` flag (see the [Credible Layer docs](../../../../docs/l2/credible_layer.md)).

### References

- [Credible Layer Introduction](https://docs.phylax.systems/credible/credible-introduction)
- [credible-layer-contracts](https://github.com/phylaxsystems/credible-layer-contracts)
- [credible-sdk (sidecar source)](https://github.com/phylaxsystems/credible-sdk)
