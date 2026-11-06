//! The merchant's durable state: immutable checkouts, idempotent operations, orders and
//! their payment state, and grant use.
//!
//! Business idempotency is separate from nonce replay protection: a retry carries a fresh
//! signature and nonce but the same idempotency key, scoped to the actor, the merchant and
//! the operation. The same key with different content is a conflict. Spend under a grant is
//! reserved in a write transaction, so concurrent orders cannot together exceed it.

use arkavo_tap::{CheckoutView, GrantLedger, GrantState};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row as _, SqlitePool};
use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

/// Where an idempotent operation stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// First use of the key.
    New,
    /// Finished earlier: replay the stored result.
    Done { status: u16, response: String },
    /// Started but not finished, for example after a crash. `order_id` once an order exists.
    InProgress { order_id: Option<String> },
    /// The key was used for different content.
    Conflict,
}

/// An order and its payment state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRecord {
    pub order_id: String,
    pub actor: String,
    pub checkout_id: String,
    pub grant_id: String,
    pub total_minor: i64,
    pub currency: String,
    pub state: String,
    pub provider_operation: String,
    pub decision: String,
    pub receipt: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Store {
    pool: SqlitePool,
}

const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS checkouts (checkout_id TEXT PRIMARY KEY NOT NULL, actor TEXT NOT NULL, view TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS operations (scope TEXT PRIMARY KEY NOT NULL, fingerprint TEXT NOT NULL, order_id TEXT, status INTEGER, response TEXT)",
    "CREATE TABLE IF NOT EXISTS orders (order_id TEXT PRIMARY KEY NOT NULL, actor TEXT NOT NULL, checkout_id TEXT NOT NULL, grant_id TEXT NOT NULL, total_minor INTEGER NOT NULL, currency TEXT NOT NULL, state TEXT NOT NULL, provider_operation TEXT NOT NULL UNIQUE, decision TEXT NOT NULL, receipt TEXT)",
    "CREATE TABLE IF NOT EXISTS revoked_grants (grant_id TEXT PRIMARY KEY NOT NULL)",
    "CREATE TABLE IF NOT EXISTS simulated_provider (provider_operation TEXT PRIMARY KEY NOT NULL, total_minor INTEGER NOT NULL, currency TEXT NOT NULL, result TEXT NOT NULL)",
];

impl Store {
    pub async fn open(path: &Path) -> Result<Self, sqlx::Error> {
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        for statement in SCHEMA {
            sqlx::query(statement).execute(&pool).await?;
        }
        Ok(Store { pool })
    }

    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn put_checkout(&self, actor: &str, view: &CheckoutView) -> Result<(), sqlx::Error> {
        let json = serde_json::to_string(view).map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        sqlx::query("INSERT INTO checkouts (checkout_id, actor, view) VALUES (?, ?, ?)")
            .bind(&view.checkout_id)
            .bind(actor)
            .bind(json)
            .execute(&self.pool)
            .await
            .map(|_| ())
    }

    /// The checkout and the actor it belongs to.
    pub async fn checkout(&self, id: &str) -> Result<Option<(String, CheckoutView)>, sqlx::Error> {
        let row = sqlx::query("SELECT actor, view FROM checkouts WHERE checkout_id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|r| {
            let view: String = r.get("view");
            serde_json::from_str(&view)
                .ok()
                .map(|v| (r.get("actor"), v))
        }))
    }

    /// Claims `scope` for content with `fingerprint`, or reports what the key already did.
    pub async fn begin(&self, scope: &str, fingerprint: &str) -> Result<Operation, sqlx::Error> {
        let inserted =
            sqlx::query("INSERT OR IGNORE INTO operations (scope, fingerprint) VALUES (?, ?)")
                .bind(scope)
                .bind(fingerprint)
                .execute(&self.pool)
                .await?;
        if inserted.rows_affected() == 1 {
            return Ok(Operation::New);
        }
        let row = sqlx::query(
            "SELECT fingerprint, order_id, status, response FROM operations WHERE scope = ?",
        )
        .bind(scope)
        .fetch_one(&self.pool)
        .await?;
        if row.get::<String, _>("fingerprint") != fingerprint {
            return Ok(Operation::Conflict);
        }
        let status: Option<i64> = row.get("status");
        let response: Option<String> = row.get("response");
        Ok(match (status, response) {
            (Some(status), Some(response)) => Operation::Done {
                status: u16::try_from(status).unwrap_or(500),
                response,
            },
            _ => Operation::InProgress {
                order_id: row.get("order_id"),
            },
        })
    }

    pub async fn attach_order(&self, scope: &str, order_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE operations SET order_id = ? WHERE scope = ?")
            .bind(order_id)
            .bind(scope)
            .execute(&self.pool)
            .await
            .map(|_| ())
    }

    pub async fn finish(
        &self,
        scope: &str,
        status: u16,
        response: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE operations SET status = ?, response = ? WHERE scope = ?")
            .bind(i64::from(status))
            .bind(response)
            .bind(scope)
            .execute(&self.pool)
            .await
            .map(|_| ())
    }

    /// Records an authorized order if committed spend under its grant plus this order stays
    /// within `limit_minor`. Returns false, recording nothing, if it would not.
    pub async fn reserve(
        &self,
        order: &OrderRecord,
        limit_minor: i64,
    ) -> Result<bool, sqlx::Error> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
        let result = async {
            let spent: i64 = sqlx::query(
                "SELECT COALESCE(SUM(total_minor), 0) AS spent FROM orders WHERE grant_id = ? AND state != 'failed'",
            )
            .bind(&order.grant_id)
            .fetch_one(&mut *conn)
            .await?
            .get("spent");
            if spent.saturating_add(order.total_minor) > limit_minor {
                return Ok(false);
            }
            sqlx::query(
                "INSERT INTO orders (order_id, actor, checkout_id, grant_id, total_minor, currency, state, provider_operation, decision)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&order.order_id)
            .bind(&order.actor)
            .bind(&order.checkout_id)
            .bind(&order.grant_id)
            .bind(order.total_minor)
            .bind(&order.currency)
            .bind(&order.state)
            .bind(&order.provider_operation)
            .bind(&order.decision)
            .execute(&mut *conn)
            .await?;
            Ok::<bool, sqlx::Error>(true)
        }
        .await;
        let end = if matches!(result, Ok(true)) {
            "COMMIT"
        } else {
            "ROLLBACK"
        };
        sqlx::query(end).execute(&mut *conn).await?;
        result
    }

    pub async fn set_state(&self, order_id: &str, state: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE orders SET state = ? WHERE order_id = ?")
            .bind(state)
            .bind(order_id)
            .execute(&self.pool)
            .await
            .map(|_| ())
    }

    pub async fn set_receipt(&self, order_id: &str, receipt: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE orders SET receipt = ? WHERE order_id = ?")
            .bind(receipt)
            .bind(order_id)
            .execute(&self.pool)
            .await
            .map(|_| ())
    }

    pub async fn order(&self, order_id: &str) -> Result<Option<OrderRecord>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM orders WHERE order_id = ?")
            .bind(order_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| OrderRecord {
            order_id: r.get("order_id"),
            actor: r.get("actor"),
            checkout_id: r.get("checkout_id"),
            grant_id: r.get("grant_id"),
            total_minor: r.get("total_minor"),
            currency: r.get("currency"),
            state: r.get("state"),
            provider_operation: r.get("provider_operation"),
            decision: r.get("decision"),
            receipt: r.get("receipt"),
        }))
    }

    pub async fn revoke_grant(&self, grant_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT OR IGNORE INTO revoked_grants (grant_id) VALUES (?)")
            .bind(grant_id)
            .execute(&self.pool)
            .await
            .map(|_| ())
    }

    async fn state_of(&self, grant_id: &str) -> Result<GrantState, sqlx::Error> {
        let revoked = sqlx::query("SELECT 1 FROM revoked_grants WHERE grant_id = ?")
            .bind(grant_id)
            .fetch_optional(&self.pool)
            .await?
            .is_some();
        let spent: i64 = sqlx::query(
            "SELECT COALESCE(SUM(total_minor), 0) AS spent FROM orders WHERE grant_id = ? AND state != 'failed'",
        )
        .bind(grant_id)
        .fetch_one(&self.pool)
        .await?
        .get("spent");
        Ok(GrantState {
            revoked,
            spent_minor: spent,
        })
    }
}

impl GrantLedger for Store {
    fn grant_state(
        &self,
        grant_id: &str,
    ) -> impl Future<Output = Result<GrantState, String>> + Send {
        let grant_id = grant_id.to_owned();
        async move { self.state_of(&grant_id).await.map_err(|e| e.to_string()) }
    }
}
