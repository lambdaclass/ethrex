use anyhow::Result;
use ethrex_storage_rollup::SQLStore;

#[tokio::test]
async fn test_schema_tables() -> Result<()> {
    let store = SQLStore::new(":memory:")?;
    let tables = [
        "blocks",
        "l1_messages",
        "privileged_transactions",
        "state_roots",
        "blob_bundles",
        "account_updates",
        "operation_count",
        "latest_sent",
        "batch_proofs",
        "block_signatures",
        "batch_signatures",
        "batch_prover_input",
    ];
    let mut attributes = Vec::new();
    for table in tables {
        let mut rows = store
            .query(format!("PRAGMA table_info({table})").as_str(), ())
            .await?;
        while let Some(row) = rows.next().await? {
            // (table, name, type)
            attributes.push((
                table.to_string(),
                row.get_str(1)?.to_string(),
                row.get_str(2)?.to_string(),
            ))
        }
    }
    for (table, name, given_type) in attributes {
        let expected_type = match (table.as_str(), name.as_str()) {
            ("blocks", "block_number") => "INT",
            ("blocks", "batch") => "INT",
            ("l1_messages", "batch") => "INT",
            ("l1_messages", "idx") => "INT",
            ("l1_messages", "message_hash") => "BLOB",
            ("privileged_transactions", "batch") => "INT",
            ("privileged_transactions", "transactions_hash") => "BLOB",
            ("state_roots", "batch") => "INT",
            ("state_roots", "state_root") => "BLOB",
            ("blob_bundles", "batch") => "INT",
            ("blob_bundles", "idx") => "INT",
            ("blob_bundles", "blob_bundle") => "BLOB",
            ("account_updates", "block_number") => "INT",
            ("account_updates", "updates") => "BLOB",
            ("operation_count", "_id") => "INT",
            ("operation_count", "transactions") => "INT",
            ("operation_count", "privileged_transactions") => "INT",
            ("operation_count", "messages") => "INT",
            ("latest_sent", "_id") => "INT",
            ("latest_sent", "batch") => "INT",
            ("batch_proofs", "batch") => "INT",
            ("batch_proofs", "prover_type") => "INT",
            ("batch_proofs", "proof") => "BLOB",
            ("block_signatures", "block_hash") => "BLOB",
            ("block_signatures", "signature") => "BLOB",
            ("batch_signatures", "batch") => "INT",
            ("batch_signatures", "signature") => "BLOB",
            ("batch_prover_input", "batch") => "INT",
            ("batch_prover_input", "prover_version") => "TEXT",
            ("batch_prover_input", "prover_input") => "BLOB",
            _ => {
                return Err(anyhow::Error::msg(
                    "unexpected attribute {name} in table {table}",
                ));
            }
        };
        assert_eq!(given_type, expected_type);
    }
    Ok(())
}
