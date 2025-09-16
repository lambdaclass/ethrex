# Connect a Wallet

You can connect your L2 network to MetaMask to interact with your rollup using a familiar wallet interface.

## Add Your L2 Network to MetaMask

1. Open MetaMask and click the network dropdown.
2. Select "Add custom network".
3. Enter your L2 network details:
   - **Network Name:** (choose any name, e.g. "My L2 Rollup")
   - **RPC URL:** `http://localhost:1729` (or your L2 node's RPC endpoint)
   - **Chain ID:** (use the chain ID from your L2 genesis config)
   - **Currency Symbol:** (e.g. ETH)
   - **Block Explorer URL:** (optional, can be left blank)
4. Save the network.

You can now use MetaMask to send transactions and interact with contracts on your L2.

> **Tip:** If you are running the L2 node on a remote server, replace `localhost` with the server's IP or domain.
