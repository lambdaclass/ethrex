## Forks

In LEVM we won't implement forks previous to Paris.
The main reasons are:
- Ethrex client will only support post-Merge forks 
- There are some aspects of previous forks that add complexity to the EVM, something we try to avoid whenever possible.

For example, thanks to [EIP-158](https://eips.ethereum.org/EIPS/eip-158) from Tangerine Whistle onwards we don't need to differentiate existing empty accounts from non-existing empty accounts because the definition of empty account changed. Now empty accounts don't exist in the [trie](https://ethereum.org/en/developers/docs/data-structures-and-encoding/patricia-merkle-trie/). So the EVM can easily check if an account exists or not just by checking that it doesn't have nonce, balance nor code! Implementing forks previous to this change would definitely add more complexity to the EVM.
We also avoid filling LEVM with `if` statements of old forks, this way the code is cleaner and easier to understand.


Note: EFTests from older forks are also tested for more recent forks, so we aren't missing any tests. The tests remain the same but we just check the results for the forks we are interested in.
