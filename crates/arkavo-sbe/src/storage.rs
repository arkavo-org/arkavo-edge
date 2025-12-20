//! SQLite persistence for SBE state
//!
//! Provides durable storage for patchlets, contracts, and layer state.
//! Uses the same SqlitePool pattern as arkavo-memory's EventStore.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::sqlite::SqlitePool;
use uuid::Uuid;

use crate::error::{SbeError, SbeResult};
use crate::layer::GraphLayer;
use crate::patchlet::PatchOp;

/// Stored patchlet record
#[derive(Debug, Clone)]
pub struct StoredPatchlet {
    /// Unique patchlet ID
    pub id: Uuid,
    /// Target nodes
    pub target_nodes: Vec<u16>,
    /// Operation (serialized)
    pub operation: String,
    /// Ed25519 signature
    pub signature: Vec<u8>,
    /// When the patchlet was created
    pub created_at: DateTime<Utc>,
    /// When the patchlet was applied (if any)
    pub applied_at: Option<DateTime<Utc>>,
    /// When the patchlet was rolled back (if any)
    pub rolled_back_at: Option<DateTime<Utc>>,
}

/// Stored invariant contract record
#[derive(Debug, Clone)]
pub struct StoredContract {
    /// Unique contract ID
    pub id: Uuid,
    /// Contract name/description
    pub name: String,
    /// Graph predicate (serialized)
    pub predicate: String,
    /// Signer public key
    pub signer_pubkey: Vec<u8>,
    /// Ed25519 signature
    pub signature: Vec<u8>,
    /// When the contract was created
    pub created_at: DateTime<Utc>,
    /// Whether the contract is active
    pub active: bool,
}

/// Stored node layer assignment
#[derive(Debug, Clone)]
pub struct StoredNodeLayer {
    /// Node ID
    pub node_id: u16,
    /// Assigned layer
    pub layer: GraphLayer,
    /// When the assignment was made
    pub assigned_at: DateTime<Utc>,
}

/// Trait for SBE storage backends
#[async_trait]
pub trait SbeStore: Send + Sync {
    /// Store a patchlet
    async fn store_patchlet(&self, patchlet: &StoredPatchlet) -> SbeResult<()>;

    /// Get a patchlet by ID
    async fn get_patchlet(&self, id: Uuid) -> SbeResult<Option<StoredPatchlet>>;

    /// List all active (not rolled back) patchlets
    async fn list_active_patchlets(&self) -> SbeResult<Vec<StoredPatchlet>>;

    /// Mark a patchlet as applied
    async fn mark_patchlet_applied(&self, id: Uuid) -> SbeResult<()>;

    /// Mark a patchlet as rolled back
    async fn mark_patchlet_rolled_back(&self, id: Uuid) -> SbeResult<()>;

    /// Store a contract
    async fn store_contract(&self, contract: &StoredContract) -> SbeResult<()>;

    /// Get a contract by ID
    async fn get_contract(&self, id: Uuid) -> SbeResult<Option<StoredContract>>;

    /// List all active contracts
    async fn list_active_contracts(&self) -> SbeResult<Vec<StoredContract>>;

    /// Store a node layer assignment
    async fn store_node_layer(&self, node: &StoredNodeLayer) -> SbeResult<()>;

    /// Get layer for a node
    async fn get_node_layer(&self, node_id: u16) -> SbeResult<Option<GraphLayer>>;

    /// List all node layer assignments
    async fn list_node_layers(&self) -> SbeResult<Vec<StoredNodeLayer>>;
}

/// SQLite-backed SBE store
///
/// Uses the same pattern as arkavo-memory's EventStore - accepts an external
/// SqlitePool for integration with shared database connections.
pub struct SqliteSbeStore {
    pool: SqlitePool,
}

impl SqliteSbeStore {
    /// Create a new SQLite store with an existing pool
    ///
    /// This follows the same pattern as arkavo-memory's EventStore,
    /// allowing shared database connections across the application.
    pub async fn new(pool: SqlitePool) -> SbeResult<Self> {
        let store = Self { pool };
        store.initialize().await?;
        Ok(store)
    }

    /// Create an in-memory SQLite store for testing
    #[cfg(test)]
    pub async fn in_memory() -> SbeResult<Self> {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .map_err(|e| SbeError::Storage(e.to_string()))?;
        Self::new(pool).await
    }

    /// Initialize the database schema
    async fn initialize(&self) -> SbeResult<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS patchlets (
                id TEXT PRIMARY KEY,
                target_nodes TEXT NOT NULL,
                operation TEXT NOT NULL,
                signature BLOB NOT NULL,
                created_at TEXT NOT NULL,
                applied_at TEXT,
                rolled_back_at TEXT
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| SbeError::Storage(e.to_string()))?;

        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS contracts (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                predicate TEXT NOT NULL,
                signer_pubkey BLOB NOT NULL,
                signature BLOB NOT NULL,
                created_at TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| SbeError::Storage(e.to_string()))?;

        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS node_layers (
                node_id INTEGER PRIMARY KEY,
                layer TEXT NOT NULL,
                assigned_at TEXT NOT NULL
            )
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| SbeError::Storage(e.to_string()))?;

        Ok(())
    }
}

#[async_trait]
impl SbeStore for SqliteSbeStore {
    async fn store_patchlet(&self, patchlet: &StoredPatchlet) -> SbeResult<()> {
        let target_nodes_json = serde_json::to_string(&patchlet.target_nodes)
            .map_err(|e| SbeError::Storage(e.to_string()))?;

        sqlx::query(
            r"
            INSERT INTO patchlets (id, target_nodes, operation, signature, created_at, applied_at, rolled_back_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(patchlet.id.to_string())
        .bind(&target_nodes_json)
        .bind(&patchlet.operation)
        .bind(&patchlet.signature)
        .bind(patchlet.created_at.to_rfc3339())
        .bind(patchlet.applied_at.map(|t| t.to_rfc3339()))
        .bind(patchlet.rolled_back_at.map(|t| t.to_rfc3339()))
        .execute(&self.pool)
        .await
        .map_err(|e| SbeError::Storage(e.to_string()))?;

        Ok(())
    }

    async fn get_patchlet(&self, id: Uuid) -> SbeResult<Option<StoredPatchlet>> {
        let row: Option<(String, String, String, Vec<u8>, String, Option<String>, Option<String>)> =
            sqlx::query_as(
                r"
                SELECT id, target_nodes, operation, signature, created_at, applied_at, rolled_back_at
                FROM patchlets WHERE id = ?
                ",
            )
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| SbeError::Storage(e.to_string()))?;

        row.map(parse_patchlet_row).transpose()
    }

    async fn list_active_patchlets(&self) -> SbeResult<Vec<StoredPatchlet>> {
        let rows: Vec<(String, String, String, Vec<u8>, String, Option<String>, Option<String>)> =
            sqlx::query_as(
                r"
                SELECT id, target_nodes, operation, signature, created_at, applied_at, rolled_back_at
                FROM patchlets WHERE rolled_back_at IS NULL
                ORDER BY created_at ASC
                ",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| SbeError::Storage(e.to_string()))?;

        rows.into_iter().map(parse_patchlet_row).collect()
    }

    async fn mark_patchlet_applied(&self, id: Uuid) -> SbeResult<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE patchlets SET applied_at = ? WHERE id = ?")
            .bind(&now)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| SbeError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn mark_patchlet_rolled_back(&self, id: Uuid) -> SbeResult<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE patchlets SET rolled_back_at = ? WHERE id = ?")
            .bind(&now)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| SbeError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn store_contract(&self, contract: &StoredContract) -> SbeResult<()> {
        sqlx::query(
            r"
            INSERT INTO contracts (id, name, predicate, signer_pubkey, signature, created_at, active)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(contract.id.to_string())
        .bind(&contract.name)
        .bind(&contract.predicate)
        .bind(&contract.signer_pubkey)
        .bind(&contract.signature)
        .bind(contract.created_at.to_rfc3339())
        .bind(contract.active)
        .execute(&self.pool)
        .await
        .map_err(|e| SbeError::Storage(e.to_string()))?;

        Ok(())
    }

    async fn get_contract(&self, id: Uuid) -> SbeResult<Option<StoredContract>> {
        let row: Option<(String, String, String, Vec<u8>, Vec<u8>, String, bool)> = sqlx::query_as(
            r"
            SELECT id, name, predicate, signer_pubkey, signature, created_at, active
            FROM contracts WHERE id = ?
            ",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SbeError::Storage(e.to_string()))?;

        row.map(parse_contract_row).transpose()
    }

    async fn list_active_contracts(&self) -> SbeResult<Vec<StoredContract>> {
        let rows: Vec<(String, String, String, Vec<u8>, Vec<u8>, String, bool)> = sqlx::query_as(
            r"
            SELECT id, name, predicate, signer_pubkey, signature, created_at, active
            FROM contracts WHERE active = 1
            ORDER BY created_at ASC
            ",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| SbeError::Storage(e.to_string()))?;

        rows.into_iter().map(parse_contract_row).collect()
    }

    async fn store_node_layer(&self, node: &StoredNodeLayer) -> SbeResult<()> {
        let layer_str = match node.layer {
            GraphLayer::Invariant => "invariant",
            GraphLayer::Policy => "policy",
            GraphLayer::Adaptive => "adaptive",
        };

        sqlx::query(
            r"
            INSERT OR REPLACE INTO node_layers (node_id, layer, assigned_at)
            VALUES (?, ?, ?)
            ",
        )
        .bind(i64::from(node.node_id))
        .bind(layer_str)
        .bind(node.assigned_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| SbeError::Storage(e.to_string()))?;

        Ok(())
    }

    async fn get_node_layer(&self, node_id: u16) -> SbeResult<Option<GraphLayer>> {
        let row: Option<(String,)> = sqlx::query_as(
            r"
            SELECT layer FROM node_layers WHERE node_id = ?
            ",
        )
        .bind(i64::from(node_id))
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SbeError::Storage(e.to_string()))?;

        Ok(row.and_then(|(layer_str,)| parse_layer(&layer_str)))
    }

    async fn list_node_layers(&self) -> SbeResult<Vec<StoredNodeLayer>> {
        let rows: Vec<(i64, String, String)> = sqlx::query_as(
            r"
            SELECT node_id, layer, assigned_at FROM node_layers ORDER BY node_id
            ",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| SbeError::Storage(e.to_string()))?;

        rows.into_iter()
            .map(|(node_id, layer_str, assigned_at_str)| {
                let layer = parse_layer(&layer_str)
                    .ok_or_else(|| SbeError::Storage(format!("Invalid layer: {layer_str}")))?;
                let assigned_at = DateTime::parse_from_rfc3339(&assigned_at_str)
                    .map_err(|e| SbeError::Storage(e.to_string()))?
                    .with_timezone(&Utc);
                Ok(StoredNodeLayer {
                    node_id: node_id as u16,
                    layer,
                    assigned_at,
                })
            })
            .collect()
    }
}

fn parse_patchlet_row(
    row: (
        String,
        String,
        String,
        Vec<u8>,
        String,
        Option<String>,
        Option<String>,
    ),
) -> SbeResult<StoredPatchlet> {
    let (
        id_str,
        target_nodes_json,
        operation,
        signature,
        created_at_str,
        applied_at_str,
        rolled_back_at_str,
    ) = row;

    let id = Uuid::parse_str(&id_str).map_err(|e| SbeError::Storage(e.to_string()))?;
    let target_nodes: Vec<u16> =
        serde_json::from_str(&target_nodes_json).map_err(|e| SbeError::Storage(e.to_string()))?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map_err(|e| SbeError::Storage(e.to_string()))?
        .with_timezone(&Utc);
    let applied_at = applied_at_str
        .map(|s| DateTime::parse_from_rfc3339(&s).map(|dt| dt.with_timezone(&Utc)))
        .transpose()
        .map_err(|e| SbeError::Storage(e.to_string()))?;
    let rolled_back_at = rolled_back_at_str
        .map(|s| DateTime::parse_from_rfc3339(&s).map(|dt| dt.with_timezone(&Utc)))
        .transpose()
        .map_err(|e| SbeError::Storage(e.to_string()))?;

    Ok(StoredPatchlet {
        id,
        target_nodes,
        operation,
        signature,
        created_at,
        applied_at,
        rolled_back_at,
    })
}

fn parse_contract_row(
    row: (String, String, String, Vec<u8>, Vec<u8>, String, bool),
) -> SbeResult<StoredContract> {
    let (id_str, name, predicate, signer_pubkey, signature, created_at_str, active) = row;

    let id = Uuid::parse_str(&id_str).map_err(|e| SbeError::Storage(e.to_string()))?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map_err(|e| SbeError::Storage(e.to_string()))?
        .with_timezone(&Utc);

    Ok(StoredContract {
        id,
        name,
        predicate,
        signer_pubkey,
        signature,
        created_at,
        active,
    })
}

fn parse_layer(s: &str) -> Option<GraphLayer> {
    match s {
        "invariant" => Some(GraphLayer::Invariant),
        "policy" => Some(GraphLayer::Policy),
        "adaptive" => Some(GraphLayer::Adaptive),
        _ => None,
    }
}

/// Helper to serialize a PatchOp for storage
pub fn serialize_patch_op(op: &PatchOp) -> SbeResult<String> {
    serde_json::to_string(op).map_err(|e| SbeError::Storage(e.to_string()))
}

/// Helper to deserialize a PatchOp from storage
pub fn deserialize_patch_op(s: &str) -> SbeResult<PatchOp> {
    serde_json::from_str(s).map_err(|e| SbeError::Storage(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_store() {
        let store = SqliteSbeStore::in_memory().await.unwrap();

        // Test storing and retrieving a patchlet
        let patchlet = StoredPatchlet {
            id: Uuid::new_v4(),
            target_nodes: vec![1, 2, 3],
            operation: r#"{"AdjustThreshold":{"node_id":1,"new_threshold":0.5}}"#.to_string(),
            signature: vec![0u8; 64],
            created_at: Utc::now(),
            applied_at: None,
            rolled_back_at: None,
        };

        store.store_patchlet(&patchlet).await.unwrap();
        let retrieved = store.get_patchlet(patchlet.id).await.unwrap().unwrap();
        assert_eq!(retrieved.id, patchlet.id);
        assert_eq!(retrieved.target_nodes, patchlet.target_nodes);
    }

    #[tokio::test]
    async fn test_patchlet_lifecycle() {
        let store = SqliteSbeStore::in_memory().await.unwrap();

        let patchlet = StoredPatchlet {
            id: Uuid::new_v4(),
            target_nodes: vec![1],
            operation: "{}".to_string(),
            signature: vec![],
            created_at: Utc::now(),
            applied_at: None,
            rolled_back_at: None,
        };

        store.store_patchlet(&patchlet).await.unwrap();

        // Mark as applied
        store.mark_patchlet_applied(patchlet.id).await.unwrap();
        let retrieved = store.get_patchlet(patchlet.id).await.unwrap().unwrap();
        assert!(retrieved.applied_at.is_some());

        // Mark as rolled back
        store.mark_patchlet_rolled_back(patchlet.id).await.unwrap();
        let retrieved = store.get_patchlet(patchlet.id).await.unwrap().unwrap();
        assert!(retrieved.rolled_back_at.is_some());

        // Should not appear in active list
        let active = store.list_active_patchlets().await.unwrap();
        assert!(active.is_empty());
    }

    #[tokio::test]
    async fn test_contract_storage() {
        let store = SqliteSbeStore::in_memory().await.unwrap();

        let contract = StoredContract {
            id: Uuid::new_v4(),
            name: "safety_contract".to_string(),
            predicate: "{}".to_string(),
            signer_pubkey: vec![1, 2, 3],
            signature: vec![4, 5, 6],
            created_at: Utc::now(),
            active: true,
        };

        store.store_contract(&contract).await.unwrap();

        let retrieved = store.get_contract(contract.id).await.unwrap().unwrap();
        assert_eq!(retrieved.name, "safety_contract");
        assert!(retrieved.active);

        let active = store.list_active_contracts().await.unwrap();
        assert_eq!(active.len(), 1);
    }

    #[tokio::test]
    async fn test_node_layer_storage() {
        let store = SqliteSbeStore::in_memory().await.unwrap();

        let node = StoredNodeLayer {
            node_id: 42,
            layer: GraphLayer::Policy,
            assigned_at: Utc::now(),
        };

        store.store_node_layer(&node).await.unwrap();

        let layer = store.get_node_layer(42).await.unwrap().unwrap();
        assert_eq!(layer, GraphLayer::Policy);

        // Update layer
        let updated = StoredNodeLayer {
            node_id: 42,
            layer: GraphLayer::Adaptive,
            assigned_at: Utc::now(),
        };
        store.store_node_layer(&updated).await.unwrap();

        let layer = store.get_node_layer(42).await.unwrap().unwrap();
        assert_eq!(layer, GraphLayer::Adaptive);
    }

    #[tokio::test]
    async fn test_list_node_layers() {
        let store = SqliteSbeStore::in_memory().await.unwrap();

        for i in 0..5 {
            let node = StoredNodeLayer {
                node_id: i,
                layer: if i % 2 == 0 {
                    GraphLayer::Policy
                } else {
                    GraphLayer::Adaptive
                },
                assigned_at: Utc::now(),
            };
            store.store_node_layer(&node).await.unwrap();
        }

        let layers = store.list_node_layers().await.unwrap();
        assert_eq!(layers.len(), 5);
    }
}
