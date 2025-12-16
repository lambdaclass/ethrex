# Tradeoffs of using or not Optimistic Transaction DB in RocksDB

## Advantages of Optimistic Transactions
- Easier to keep consistency between multiple writers.
- Easier to keep consistency with readers.

## Advantages of Pessimistic Transactions
- Lower memory usage?
- No conflict resolution.

## Advantages of Non-Transactional DB
- `delete_range`: much faster key deletion in bulk, useful during healing.
- Lower memory usage (confirm).
- Secondary instances are either much easier or possible.

## Alternatives to DB Transactions
- We can use snapshots/secondary instances to provide a consistent view to block production and protocols.
- We can provide crash safety by adding checkpoints or explicit markers so we can go back to the last known consistent version.
- We would need to run a healing pass from that point on startup, to fix possible partial changes to tries.
- We would need to add explicit tests for crashing during writes to make sure we recover correctly.
- L2 can't recover from peers (at least until Based), so it should probably remain transactional.
