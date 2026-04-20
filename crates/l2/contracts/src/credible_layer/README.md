# Credible Layer Contracts

This directory contains example/demo contracts for the Credible Layer integration:

- `OwnableTarget.sol` — a simple Ownable contract used as a demonstration target
- `TestOwnershipAssertion.sol` — a test assertion that asserts ownership of `OwnableTarget` cannot change

## State Oracle

The **State Oracle** is the on-chain registry that maps protected contracts to their active
assertions. It is maintained by Phylax Systems and must be deployed separately using the
Phylax toolchain before starting the Credible Layer sidecar.

See the [Credible Layer docs](../../../../docs/l2/credible_layer.md) for deployment instructions.

### References

- [Credible Layer Introduction](https://docs.phylax.systems/credible/credible-introduction)
- [credible-layer-contracts](https://github.com/phylaxsystems/credible-layer-contracts)
