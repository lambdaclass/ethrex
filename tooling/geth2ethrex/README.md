# Post-merge Sync

Post-merge sync can be achieved for Ethrex by full-syncing Geth to the right head, then using the `geth2ethrex` tool to update the Database.

## Step 1: Syncing Geth

The most efficient way to sync Geth to an arbitrary block is by full-syncing and telling it to stop afterwards:

```shell
# This syncs to post-merge mainnet.
# You'll need to fetch the right block's hash, this one is taken from Etherscan.
# Target for Sepolia: 0xa8958f18de8705a0c04dfc7b64c0a7dc38b19bd7517a3030fff7cf077f3f33bc
# Target for Mainnet: 0xe37e1a183a3d1c7234d090bfb7196081635919c26f2e65c67c106513158a7db4
geth --snapshot=false --state.scheme=path --history.chain=postmerge --history.logs.disable --history.state=1 --history.transactions=1 --syncmode=full --exitwhensynced=true --mainnet --synctarget=0xe37e1a183a3d1c7234d090bfb7196081635919c26f2e65c67c106513158a7db4
```

You don't need a consensus node for this step, as you'll be telling Geth exactly which block you need.


### Step 2: Run the `geth2ethrex` executable

Usage is:
```shell
./geth2ethrex --input_dir <geth_db_path> --output_dir <ethrex_db_path> --network <network> <block_number>
```
where:
* `<geth_db_path>` is the path to Geth's database (by default `.ethereum/geth/<chain>`). The DB is opened read-only.
* `<ethrex_db_path>` is the path to Ethrex' database.
* `<BLOCK_NUMBER>` is the target block, typically the first post-merge block of the chain.
  * For Sepolia: 1450410
  * For Mainnet: 15537395

If your goal is related to performance, you might want to add `--flatkeyvalue` to generate flatkeyvalues. Otherwise the measurements would likely not be representative of real life.
