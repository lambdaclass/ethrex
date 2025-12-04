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
