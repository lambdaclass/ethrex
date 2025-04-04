# LEVM State Representation

## Database

`Database` is a trait in LEVM. Any execution client that wants to use LEVM as its EVM should implement this in the struct they use for accessing the state trie. It has methods for interacting with it like `get_account_info(address)` and `get_storage_slot(address, key)` . \
Even though in LEVM we can abstract from the actual implementation, it’s useful to know that the Database is actually a [Merkle Patricia Trie](https://ethereum.org/en/developers/docs/data-structures-and-encoding/patricia-merkle-trie/).

## CacheDB

LEVM exposes an `execute()` method just for executing transactions. Every time that we want to do this we instantiate a new `VM` and execute a specific transaction on its own. However, execution clients frequently need to execute whole blocks, and we need to persist changes between transactions because the database is usually updated ONLY after having executed a whole block for performance reasons (or in some special cases, after having executed a batch of blocks). 

For example, imagine that in the first transaction of a block an account sends all its Ether to another account, and after that, there is another transaction in which that same account wants to call a contract. That last transaction should fail because the account has no Ether left for paying for the execution of that contract. The thing is that, if we look at the Merkle Trie, we’ll see that account still has balance left! 

The solution to this is persisting that storage that hasn’t yet been committed to the database in memory. In LEVM, the struct that holds this is the `CacheDB`, which is simply a `HashMap<Address, Account>` . Therefore, after executing any transaction we mutate this struct over and over again, storing all the accounts that have been gathered from the `Database` and that have potentially been updated! This struct is both useful for keeping track of the changes during a transaction and between transactions. It also helps reduce queries to the database, which impacts performance.

## Generalized Database

So, we have two structures that represent state, the first one, `Database`, represents access to the actual Merkle Trie, and the second one, `CacheDB`, is an updateable storage in memory (that will be eventually committed to the actual database). These are wrapped into a `GeneralizedDatabase` mostly so that we can easily use it as an argument in various methods we have instead of passing an `Arc<dyn Database>` and a `CacheDB` separately.
