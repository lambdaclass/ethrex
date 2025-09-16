use super::DBTable;
use crate::v2::backend::{StorageBackend, StorageError};
use std::collections::HashMap;
use std::sync::Arc;

/// Schema registry that manages the mapping between logical Ethereum data
/// and physical storage operations.
#[derive(Debug, Clone)]
pub struct SchemaRegistry {
    backend: Arc<dyn StorageBackend>,
    tables: HashMap<DBTable, TableDefinition>,
}

/// Definition of how a logical table is stored and serialized
#[derive(Debug, Clone)]
pub struct TableDefinition {
    pub table: DBTable,
    pub namespace: String,
    // We'll add serializers later when we need them
}

impl SchemaRegistry {
    /// Create a new schema registry with the given storage backend
    pub async fn new(backend: Arc<dyn StorageBackend>) -> Result<Self, StorageError> {
        let mut registry = Self {
            backend,
            tables: HashMap::new(),
        };

        // Initialize all Ethereum tables
        for &table in DBTable::all() {
            registry.register_table(table).await?;
        }

        Ok(registry)
    }

    /// Register a table with the storage backend
    async fn register_table(&mut self, table: DBTable) -> Result<(), StorageError> {
        let namespace = table.namespace().to_string();

        // Ensure the namespace exists in the backend
        self.backend.init_namespace(&namespace).await?;

        // Register the table definition
        let definition = TableDefinition {
            table,
            namespace: namespace.clone(),
        };

        self.tables.insert(table, definition);

        Ok(())
    }

    /// Get a value by key from a specific table
    pub fn get_sync(&self, table: DBTable, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        let table_def = self
            .tables
            .get(&table)
            .ok_or_else(|| StorageError::Custom(format!("Table {:?} not registered", table)))?;

        self.backend.get_sync(&table_def.namespace, key)
    }

    /// Get a value by key from a specific table
    pub async fn get_async(
        &self,
        table: DBTable,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, StorageError> {
        let table_def = self
            .tables
            .get(&table)
            .ok_or_else(|| StorageError::Custom(format!("Table {:?} not registered", table)))?;

        self.backend.get_async(&table_def.namespace, key).await
    }

    pub async fn get_async_batch(
        &self,
        table: DBTable,
        keys: Vec<Vec<u8>>,
    ) -> Result<Vec<Vec<u8>>, StorageError> {
        let table_def = self
            .tables
            .get(&table)
            .ok_or_else(|| StorageError::Custom(format!("Table {:?} not registered", table)))?;

        self.backend
            .get_async_batch(&table_def.namespace, keys)
            .await
    }

    /// Put a key-value pair in a specific table
    pub async fn put(&self, table: DBTable, key: &[u8], value: &[u8]) -> Result<(), StorageError> {
        let table_def = self
            .tables
            .get(&table)
            .ok_or_else(|| StorageError::Custom(format!("Table {:?} not registered", table)))?;

        self.backend.put(&table_def.namespace, key, value).await
    }

    /// Delete a key from a specific table
    pub async fn delete(&self, table: DBTable, key: &[u8]) -> Result<(), StorageError> {
        let table_def = self
            .tables
            .get(&table)
            .ok_or_else(|| StorageError::Custom(format!("Table {:?} not registered", table)))?;

        self.backend.delete(&table_def.namespace, key).await
    }

    /// Get a range of key-value pairs from a table
    pub async fn range(
        &self,
        table: DBTable,
        start_key: &[u8],
        end_key: Option<&[u8]>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StorageError> {
        let table_def = self
            .tables
            .get(&table)
            .ok_or_else(|| StorageError::Custom(format!("Table {:?} not registered", table)))?;

        self.backend
            .range(&table_def.namespace, start_key, end_key)
            .await
    }

    /// Execute a batch of operations across multiple tables
    pub async fn batch_write(&self, ops: Vec<TableBatchOp>) -> Result<(), StorageError> {
        let mut backend_ops = Vec::new();

        for op in ops {
            let table_def = self.tables.get(&op.table()).ok_or_else(|| {
                StorageError::Custom(format!("Table {:?} not registered", op.table()))
            })?;

            let backend_op = match op {
                TableBatchOp::Put {
                    table: _,
                    key,
                    value,
                } => crate::v2::backend::BatchOp::Put {
                    namespace: table_def.namespace.clone(),
                    key,
                    value,
                },
                TableBatchOp::Delete { table: _, key } => crate::v2::backend::BatchOp::Delete {
                    namespace: table_def.namespace.clone(),
                    key,
                },
            };

            backend_ops.push(backend_op);
        }

        self.backend.batch_write(backend_ops).await
    }
}

/// Batch operation at the table level (before translation to backend operations)
#[derive(Debug, Clone)]
pub enum TableBatchOp {
    Put {
        table: DBTable,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    Delete {
        table: DBTable,
        key: Vec<u8>,
    },
}

impl TableBatchOp {
    fn table(&self) -> DBTable {
        match self {
            Self::Put { table, .. } => *table,
            Self::Delete { table, .. } => *table,
        }
    }
}
