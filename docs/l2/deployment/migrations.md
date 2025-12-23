# Migrations

## From v7 to v8

The following table name change was made:
`messages` -> `l1_messages`

In order to perform a migration you would need to copy the contents of your `messages` table into `l1_messeges`

```sql
INSERT INTO l1_messages
SELECT *
FROM messages;
```

You can then safely delete the `messages` table

## From v8 to v9

The table `balance_diffs` schema was changed. We added a new column `value_per_token`.

In order to perform a migration tou would need to add the new column with type `BLOB`.
You can do this by executing:

```sql
ALTER TABLE balance_diffs 
ADD COLUMN value_per_token BLOB;
```
