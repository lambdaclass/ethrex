# Why Use a Different Native Token for L2 Instead of ETH?

## Motivation and Rationale

Layer 2 (L2) solutions scale Ethereum by processing transactions off the main chain (L1) while relying on L1 for security and settlement. A key design choice for L2s is their native token—the asset for transaction fees, incentives, and protocol operations. While using ETH seems intuitive, many L2s opt for their own token.

The primary motivation is protocol independence. A distinct token allows an L2 to define its own monetary policy and fee structure, tailoring its economic model to its ecosystem’s needs. For instance, L2s can experiment with fixed or dynamic pricing, or subsidized transactions, which may not align with Ethereum’s fee dynamics.

Another factor is ecosystem growth. A native token can be distributed as rewards or grants to early users, developers, and validators, attracting activity and aligning incentives. Staking the token for consensus or security further ties participants to the L2’s success.

Governance is also key. Native tokens enable on-chain voting for protocol upgrades or treasury decisions, giving the L2 community direct control, which is harder to achieve with ETH alone.

## Description

In practice, using a different native token means that all core activities on the L2—such as paying transaction fees, staking, or participating in governance—are done with this token instead of ETH. To interact with the L2, users may need to bridge the native token from L1 or obtain it through other means, although ETH and other assets can still be used for regular transactions and applications on the L2.

To support this, the L1 bridge contract (`CommonBridge`) is set up with a `NATIVE_TOKEN_L1_ADDRESS` during initialization. This address determines which token can be deposited as the native asset on the L2. If you want to use ETH as the native token, you simply set this address to zero when initializing the bridge.

The bridge’s deposit function (`deposit(uint256 amount, address token, address L2Recipient)`) only accepts deposits for the token specified as the native token. Withdrawals are also restricted to this token. This setup does not prevent bridging or depositing other ERC20 tokens; it just means you cannot deposit the designated native token unless it matches the one set during initialization.

On the L2 side, the native token is treated like any other token in terms of functionality, so its use does not change the behavior of the protocol—it simply serves as the main asset for protocol-level operations.
