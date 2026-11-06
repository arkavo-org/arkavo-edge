//! Replay protection: each nonce is claimed once within its scope, durably.
//!
//! The scope is the trust authority, the verified key's thumbprint, the merchant audience and
//! the nonce, hashed so the table holds no raw identifiers. A claim is made only after the
//! signature verifies: claiming first would let an unauthenticated caller burn another agent's
//! nonce. Claims are kept until the signature's expiry plus a cleanup margin, in SQLite, so
//! they survive a restart and every verifier sharing the database sees them. When the store
//! cannot answer, protected operations fail closed.

use crate::clock::Clock;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

/// What a nonce is unique within.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayScope {
    pub authority: String,
    pub key_thumbprint: String,
    pub audience: String,
    pub nonce: String,
}

impl ReplayScope {
    /// A fixed-length digest of the scope, with each field length-prefixed so no two scopes
    /// share an encoding.
    pub fn digest(&self) -> String {
        let mut h = Sha256::new();
        for part in [
            &self.authority,
            &self.key_thumbprint,
            &self.audience,
            &self.nonce,
        ] {
            h.update((part.len() as u64).to_be_bytes());
            h.update(part.as_bytes());
        }
        crate::keys::hex(&h.finalize())
    }
}

/// The result of a claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    Claimed,
    AlreadyUsed,
}

/// A durable, shared set of claimed nonces.
pub trait ReplayStore: Send + Sync {
    /// Claims `scope` until `expires_at`, atomically. `Err` means the store could not answer.
    fn claim(
        &self,
        scope: &ReplayScope,
        expires_at: i64,
        decision_id: &str,
    ) -> impl Future<Output = Result<ClaimOutcome, String>> + Send;
}

/// Replay claims in a SQLite database file.
#[derive(Debug, Clone)]
pub struct SqliteReplayStore<C> {
    pool: SqlitePool,
    clock: C,
    cleanup_margin: i64,
}

impl<C: Clock> SqliteReplayStore<C> {
    /// Opens (or creates) the store at `path`. Several stores may open the same file.
    pub async fn open(path: &Path, clock: C, cleanup_margin: i64) -> Result<Self, String> {
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
            .map_err(|e| format!("replay store path: {e}"))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .map_err(|e| format!("opening the replay store: {e}"))?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS replay_claims (
                scope TEXT PRIMARY KEY NOT NULL,
                expires_at INTEGER NOT NULL,
                decision_id TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .map_err(|e| format!("creating the replay table: {e}"))?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS replay_claims_expiry ON replay_claims (expires_at)",
        )
        .execute(&pool)
        .await
        .map_err(|e| format!("creating the replay index: {e}"))?;
        Ok(SqliteReplayStore {
            pool,
            clock,
            cleanup_margin,
        })
    }

    /// Removes claims whose signatures expired more than the margin ago. Their signatures
    /// fail the time check, so their nonces no longer need remembering.
    pub async fn purge_expired(&self) -> Result<u64, String> {
        let cutoff = self.clock.now() - self.cleanup_margin;
        sqlx::query("DELETE FROM replay_claims WHERE expires_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map(|r| r.rows_affected())
            .map_err(|e| format!("purging replay claims: {e}"))
    }

    /// Closes the pool, as a process exit would.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

impl<C: Clock> ReplayStore for SqliteReplayStore<C> {
    async fn claim(
        &self,
        scope: &ReplayScope,
        expires_at: i64,
        decision_id: &str,
    ) -> Result<ClaimOutcome, String> {
        self.purge_expired().await?;
        let inserted = sqlx::query(
            "INSERT INTO replay_claims (scope, expires_at, decision_id) VALUES (?, ?, ?)",
        )
        .bind(scope.digest())
        .bind(expires_at)
        .bind(decision_id)
        .execute(&self.pool)
        .await;
        match inserted {
            Ok(_) => Ok(ClaimOutcome::Claimed),
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Ok(ClaimOutcome::AlreadyUsed)
            }
            Err(e) => Err(format!("claiming a nonce: {e}")),
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // Tokio test entrypoints own their runtime.
mod tests {
    use super::*;
    use crate::clock::ManualClock;

    fn scope(nonce: &str) -> ReplayScope {
        ReplayScope {
            authority: "arkavo-local-demo/local".into(),
            key_thumbprint: "tp".into(),
            audience: "http://127.0.0.1:3001".into(),
            nonce: nonce.into(),
        }
    }

    #[test]
    fn scope_digests_are_unambiguous() {
        let a = ReplayScope {
            authority: "ab".into(),
            key_thumbprint: "c".into(),
            ..scope("n")
        };
        let b = ReplayScope {
            authority: "a".into(),
            key_thumbprint: "bc".into(),
            ..scope("n")
        };
        assert_ne!(a.digest(), b.digest());
        assert_eq!(a.digest().len(), 64);
    }

    #[tokio::test]
    async fn a_nonce_is_claimed_once_and_the_claim_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replay.sqlite");
        let clock = ManualClock::new(1_000);
        let store = SqliteReplayStore::open(&path, clock.clone(), 30)
            .await
            .unwrap();
        assert_eq!(
            store.claim(&scope("n1"), 1_100, "d1").await,
            Ok(ClaimOutcome::Claimed)
        );
        assert_eq!(
            store.claim(&scope("n1"), 1_100, "d2").await,
            Ok(ClaimOutcome::AlreadyUsed)
        );
        store.close().await;
        let reopened = SqliteReplayStore::open(&path, clock.clone(), 30)
            .await
            .unwrap();
        assert_eq!(
            reopened.claim(&scope("n1"), 1_100, "d3").await,
            Ok(ClaimOutcome::AlreadyUsed)
        );
        // Kept through expiry plus the margin, then purged.
        clock.set(1_129);
        assert_eq!(reopened.purge_expired().await, Ok(0));
        clock.set(1_131);
        assert_eq!(reopened.purge_expired().await, Ok(1));
    }

    #[tokio::test]
    async fn concurrent_verifiers_on_one_file_admit_a_nonce_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replay.sqlite");
        let clock = ManualClock::new(1_000);
        let first = SqliteReplayStore::open(&path, clock.clone(), 30)
            .await
            .unwrap();
        let second = SqliteReplayStore::open(&path, clock, 30).await.unwrap();
        let race = scope("race");
        let (one, two) = tokio::join!(
            first.claim(&race, 1_100, "first"),
            second.claim(&race, 1_100, "second")
        );
        let claimed = [one.unwrap(), two.unwrap()]
            .iter()
            .filter(|o| **o == ClaimOutcome::Claimed)
            .count();
        assert_eq!(claimed, 1);
    }

    #[tokio::test]
    async fn an_unreachable_store_fails() {
        let dir = tempfile::tempdir().unwrap();
        let clock = ManualClock::new(0);
        let store = SqliteReplayStore::open(&dir.path().join("r.sqlite"), clock, 30)
            .await
            .unwrap();
        store.close().await;
        assert!(store.claim(&scope("n"), 10, "d").await.is_err());
        assert!(
            SqliteReplayStore::open(
                &dir.path().join("missing/dir/r.sqlite"),
                ManualClock::new(0),
                30
            )
            .await
            .is_err()
        );
    }
}
