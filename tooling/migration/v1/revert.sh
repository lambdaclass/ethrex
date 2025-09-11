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
if [ "$version" -ne 1 ]; then
    echo "Cannot revert migration. Current version: $version. Expected: 1"
    exit 2
fi

batches=$(sqlite3 $db_file 'SELECT MAX(number) from batches')
block_inserts=""
echo "Reverting $batches batches"

for batch in $(seq $batches); do
    first_block=$(sqlite3 $db_file "SELECT first_block FROM batches WHERE number = $batch")
    last_block=$(sqlite3 $db_file "SELECT last_block FROM batches WHERE number = $batch")
    for block in $(seq $first_block $last_block); do
        block_inserts="$block_inserts ($block, $batch),"
    done
done

block_inserts=$(echo "$block_inserts" | sed 's/,$//')

sqlite3 $db_file "
BEGIN TRANSACTION;


-- Restore individual tables
CREATE TABLE IF NOT EXISTS blocks (block_number INT PRIMARY KEY, batch INT);

INSERT INTO blocks (block_number, batch)
VALUES $block_inserts;

CREATE TABLE IF NOT EXISTS privileged_transactions (batch INT PRIMARY KEY, transactions_hash BLOB);

INSERT INTO privileged_transactions (batch, transactions_hash)
SELECT number, privileged_transactions_hash
FROM batches;

CREATE TABLE IF NOT EXISTS state_roots (batch INT PRIMARY KEY, state_root BLOB);

INSERT INTO state_roots (batch, state_root)
SELECT number, state_root
FROM batches;

CREATE TABLE IF NOT EXISTS commit_txs (batch INT PRIMARY KEY, commit_tx BLOB);

INSERT INTO commit_txs (batch, commit_tx)
SELECT number, commit_tx
FROM batches;

CREATE TABLE IF NOT EXISTS verify_txs (batch INT PRIMARY KEY, verify_tx BLOB);

INSERT INTO verify_txs (batch, verify_tx)
SELECT number, verify_tx
FROM batches;

CREATE TABLE IF NOT EXISTS batch_signatures (batch INT PRIMARY KEY, signature BLOB);

INSERT INTO batch_signatures (batch, signature)
SELECT number, signature
FROM batches;

CREATE TABLE IF NOT EXISTS latest_sent (_id INT PRIMARY KEY, batch INT);

INSERT INTO latest_sent (_id, batch)
VALUES (0, (
    SELECT number
    FROM batches
    WHERE verify_tx IS NOT NULL
));


-- Drop 'batches' table
DROP TABLE IF EXISTS batches;


-- Restore '_id' column from precommit_privileged
CREATE TABLE precommit_privileged_new (_id INT PRIMARY KEY, start INT, end INT);

INSERT INTO precommit_privileged_new (start, end)
SELECT start, end FROM precommit_privileged;

UPDATE precommit_privileged_new SET _id=0;

DROP TABLE precommit_privileged;
ALTER TABLE precommit_privileged_new RENAME TO precommit_privileged;


-- Restore '_id' column from operation_count
CREATE TABLE operation_count_new (
    _id INT PRIMARY KEY,
    transactions INT,
    privileged_transactions INT,
    messages INT
);

INSERT INTO operation_count_new (transactions, privileged_transactions, messages)
SELECT transactions, privileged_transactions, messages  FROM operation_count;

UPDATE operation_count_new SET _id=0;

DROP TABLE operation_count;
ALTER TABLE operation_count_new RENAME TO operation_count;

-- Remove migration version
DELETE FROM migrations WHERE version = 1;


COMMIT;
"

if [ $? -eq 0 ]; then
    new_batches=$(sqlite3 $db_file 'SELECT MAX(batch) FROM blocks')
    if [ "$new_batches" -ne "$batches" ]; then
        echo "ERROR: New batches count doesn't match old count!" > /dev/stderr
        exit 2
    fi
    echo "Revert migration completed successfully"
fi
