## Can you delete accounts in Ethereum? Yes

### How it happens

Ethereum accounts are broadly divided into two categories:
- Externally Owned Accounts (EOA): accounts for general users to transfer eth and call contracts.
- Contracts: which execute code and store data.

Creating EOA is done through sending ETH into a new address, at which point the account is created and added into the state trie.

Creating a contract can be done through the CREATE and [CREATE2](https://eips.ethereum.org/EIPS/eip-1014) opcode. Notably, those opcodes check that the account is created at an address where the code is empty and the nonce is zero, but **it doesn't check balance**. As such, a contract can be created through taking over an existing account.

During the creation of a contract, the `init_code` is run which can include the [self destruct opcode](https://eips.ethereum.org/EIPS/eip-6780) that deletes the contract in the same transaction it was created. Normally, this deletes an account that was created in the same transaction (because contracts are usually created over empty accounts) but in this case the account already existed because it already had some balance. This is the only edge case in which an account can go from existing to non-existing from one block to another after the Cancun fork.

### How we found it

Snap-sync is broadly divided into two stages:
- Downloading the leaves of the state (account states) and storage tries (storage slots)
- Healing (reconciling the state). 

Healing is needed because the leaves can be downloaded from disparate blocks, and to "fix" only the nodes of the trie that changed between blocks. [In depth explanation](./healing.md).

We were working under the assumption that accounts were never deleted, so we adopted some specific optimizations. During the state healing stage every account that was "healed" was added into a list of accounts that needed to be checked for storage healing. When healing the storage of those accounts the algorithm requested their account states and expected them to be there to see if they had any storage that needed healing. This led to the storage healing threads panicking when they failed to find the account that was deleted.

During the test of snapsync mainnet, we started seeing that storage healing was panicking, so we added some logs to see what account hashes were being accessed and when were they healed vs accessed. Exploring the database we saw that the offending account was present in a previous state and missing in the next one, with the corresponding merkle proof matching the block state root. Originally we suspected a reorg, but searching the blocks we saw they were finalized in the chain. 

The account state present indicated an account with nonce 0, no code and no storage but with balance. We didn't have access to the account address, as the state trie only stores the hash of the account address so we turned to another strategy to find it. Using [etherscan's API](https://docs.etherscan.io/api-endpoints/accounts#get-internal-transactions-by-block-range) allowing to search internal transactions from a block range, we explored the range where we knew the account existed in the state trie. Hashing all of the `to` and `from` of the transactions [we found the transaction](https://etherscan.io/tx/0xf23b2c233410141cda0c6d24f21f0074c494565bfd54ce008c5ce1b30b23b0da) that deleted the account with a self destruct. Despite the account becoming a contract just during that transaction, we saw that 900 blocks before it was [created with a transfer](https://etherscan.io/tx/0xbc9f52ba45a6915878318be944cb20bd3bb1bbf36b2ce8ff5e6575ce1689f1b6). The result of the self destruct was the transfer of 0.044 ETH from one account to another.

The specific transaction that created the contract: https://etherscan.io/tx/0xf23b2c233410141cda0c6d24f21f0074c494565bfd54ce008c5ce1b30b23b0da
