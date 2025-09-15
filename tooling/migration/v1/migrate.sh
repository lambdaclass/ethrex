#!/bin/bash

if [ $# -ne 1 ]; then
    echo "Usage: $0 <DB_FILE>"
    exit 1
fi

db_file="$1"

if [ ! -f "$db_file" ]; then
    echo "Database '$db_file' doesn't exist" > /dev/stderr
    exit 1
fi

version=$(sqlite3 $db_file 'SELECT MAX(version) from migrations' 2>/dev/null || echo 0)
if [ "$version" != "" ] && [ $version -ne 0 ]; then
    echo "Cannot apply migration. Current version: $version. Expected: 0"
    exit 2
fi

batches=$(sqlite3 $db_file 'SELECT MAX(batch) from blocks')
batch_inserts=""
echo "Migrating $batches batches"

for batch in $(seq $batches); do
    first_block=$(sqlite3 $db_file "SELECT MIN(block_number) FROM blocks WHERE batch = $batch")
    last_block=$(sqlite3 $db_file "SELECT MAX(block_number) FROM blocks WHERE batch = $batch")
    batch_inserts="$batch_inserts ($batch, $first_block, $last_block, \"\"),"
done

batch_inserts=$(echo "$batch_inserts" | sed 's/,$//')

sqlite3 $db_file "
BEGIN TRANSACTION;


-- Create rows in new 'batches' table
CREATE TABLE IF NOT EXISTS batches (
    number INT PRIMARY KEY,
    first_block INT NOT NULL,
    last_block INT NOT NULL,
    privileged_transactions_hash BLOB,
    state_root BLOB NOT NULL,
    commit_tx BLOB,
    verify_tx BLOB,
    signature BLOB
);

INSERT INTO batches (number, first_block, last_block, state_root)
VALUES $batch_inserts;

UPDATE batches
SET state_root = (
    SELECT sr.state_root
    FROM state_roots sr
    WHERE sr.batch = batches.number
);

UPDATE batches
SET privileged_transactions_hash = (
    SELECT pt.transactions_hash
    FROM privileged_transactions pt
    WHERE pt.batch = batches.number
)
WHERE EXISTS (
    SELECT 1
    FROM privileged_transactions pt
    WHERE pt.batch = batches.number
);

UPDATE batches
SET commit_tx = (
    SELECT pt.commit_tx
    FROM commit_txs pt
    WHERE pt.batch = batches.number
)
WHERE EXISTS (
    SELECT 1
    FROM commit_txs pt
    WHERE pt.batch = batches.number
);

UPDATE batches
SET verify_tx = (
    SELECT pt.verify_tx
    FROM verify_txs pt
    WHERE pt.batch = batches.number
)
WHERE EXISTS (
    SELECT 1
    FROM verify_txs pt
    WHERE pt.batch = batches.number
);

UPDATE batches
SET signature = (
    SELECT pt.signature
    FROM batch_signatures pt
    WHERE pt.batch = batches.number
)
WHERE EXISTS (
    SELECT 1
    FROM batch_signatures pt
    WHERE pt.batch = batches.number
);


-- Drop old tables
DROP TABLE IF EXISTS blocks;
DROP TABLE IF EXISTS privileged_transactions;
DROP TABLE IF EXISTS state_roots;
DROP TABLE IF EXISTS commit_txs;
DROP TABLE IF EXISTS verify_txs;
DROP TABLE IF EXISTS latest_sent;
DROP TABLE IF EXISTS batch_signatures;


-- Remove '_id' column from precommit_privileged
CREATE TABLE precommit_privileged_new (start INT, end INT);

INSERT INTO precommit_privileged_new (start, end)
SELECT start, end FROM precommit_privileged;

DROP TABLE precommit_privileged;
ALTER TABLE precommit_privileged_new RENAME TO precommit_privileged;


-- Remove '_id' column from operation_count
CREATE TABLE operation_count_new (
    transactions INT,
    privileged_transactions INT,
    messages INT
);

INSERT INTO operation_count_new (transactions, privileged_transactions, messages)
SELECT transactions, privileged_transactions, messages  FROM operation_count;

DROP TABLE operation_count;
ALTER TABLE operation_count_new RENAME TO operation_count;


-- Create 'migrations' table
CREATE TABLE IF NOT EXISTS migrations (version INT PRIMARY KEY);
INSERT OR REPLACE INTO migrations VALUES (0), (1);


COMMIT;
"

if [ $? -eq 0 ]; then
    new_batches=$(sqlite3 $db_file 'SELECT COUNT(*) FROM batches')
    if [ "$new_batches" -ne "$batches" ]; then
        echo "ERROR: New batches count doesn't match old count!" > /dev/stderr
        exit 2
    fi
    echo "Migration completed successfully"
fi
