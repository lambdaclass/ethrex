# Upgrading a node

## Prerequisites

This guide assumes that you already have an L2 node deployed and running as stated in the [Deploying a node](./running.md) instructions.

## Migrate the DB

If the upgrade implies a database schema change, you will need to migrate the database. You can do this by running the following command:

```sh
bash tooling/migration/<version>/migrate.sh <DATABASE_PATH>
```

Replace `<version>` with the version you are migrating to (generally the latest one), and `<DATABASE_PATH>` with the path to your database (should be `~/.local/share/ethrex/rollup_store` if using default `--datadir` flag).

> [!NOTE]
> A `revert.sh` script is also available for each migration version, in case you need to revert the migration.
