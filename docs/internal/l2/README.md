# L2 developer guides

## Chain IDs

In an attempt to standardize the deployed chains by the team, we decided to use the following convention for the chain IDs:

- All chains will have ID `65536XYY`.
- `X` is the stage of the chain:
  - `0`: Mainnet
  - `1`: Testnet
  - `2..9`: Staging 1..7
- `YY` is a number assigned to each deployed rollup. This could be a client, a project, etc. (e.g., Rogue could be 00).

Following this, the default chain ID set for local development is `65536999`.
