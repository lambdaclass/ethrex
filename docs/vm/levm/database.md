# LEVM State Representation

## Database

`Database` is a trait in LEVM. Any execution client that wants to use LEVM as its EVM should implement this trait on the struct they use for accessing the state trie. It has methods for interacting with it like `get_account_info(address)` and `get_storage_slot(address, key)`.\
Even though in LEVM we can abstract from the actual implementation, it’s useful to know that the Database is actually a [Merkle Patricia Trie](https://ethereum.org/en/developers/docs/data-structures-and-encoding/patricia-merkle-trie/).
The database can be either on-disk or in-memory. In a real case scenario it is usually the former one.

## CacheDB

LEVM exposes an `execute()` method just for executing transactions. Every time that we want to do this we first instantiate a new `VM` and execute a specific transaction on its own. However, execution clients frequently need to execute whole blocks, and we need to persist changes between transactions because the database is usually updated ONLY after having executed a whole block for performance reasons (or in some special cases, after having executed a batch of blocks). 

For example, imagine that in the first transaction of a block an account sends all its Ether to another account, and after that, there is another transaction in which that same account wants to call a contract. That last transaction should fail because the account has no Ether left for paying for the execution of that contract. The thing is, if we look at the Database we'll see that the account still has balance because it hasn't been updated yet!

The current solution to this is persisting the uncommitted storage in memory. In LEVM, this is done using the `CacheDB` struct, which is simply a `HashMap<Address, Account>`. Therefore, after executing any transaction we mutate this struct over and over again, storing all the accounts that have been gathered from the `Database` and that have potentially been updated! So that if we want to access information of an account or a storage slot we first check if it's in the `CacheDB`, and if it's not then we query the `Database` and insert the data into our cache. This is useful for tracking changes within and across transactions and it also reduces queries to the database, which impacts performance.

## Generalized Database

We have two structures that represent state. The first one, `Database`, represents access to the actual Merkle Trie. The second one, `CacheDB`, is an updateable storage in memory (that will be eventually committed to the actual database). These are wrapped into a `GeneralizedDatabase` mostly so that we can easily use it as an argument in various methods we have instead of passing an `Arc<dyn Database>` and a `CacheDB` separately.
