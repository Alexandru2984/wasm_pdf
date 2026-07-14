use std::time::Duration;

use anyhow::Context;
use sqlx::{Postgres, Transaction, query, query_scalar};
use tokio::task::JoinHandle;

use crate::{Database, MaintenanceConfig, Metrics};

const ADVISORY_LOCK_ID: i64 = 8_675_309_742_221_014;
const BATCH_SIZE: i64 = 1_000;
const MAX_BATCHES_PER_CATEGORY: usize = 10;

const CLEANUP_QUERIES: [CleanupQuery; 5] = [
    CleanupQuery {
        category: "sessions",
        sql: r"WITH doomed AS (
                   SELECT id FROM sessions
                   WHERE LEAST(expires_at, idle_expires_at) < now() - make_interval(days => $1::int)
                      OR revoked_at < now() - make_interval(days => $1::int)
                   ORDER BY COALESCE(revoked_at, LEAST(expires_at, idle_expires_at))
                   LIMIT $2
                   FOR UPDATE SKIP LOCKED
               )
               DELETE FROM sessions AS target
               USING doomed
               WHERE target.id = doomed.id",
        retention: Retention::Sessions,
    },
    CleanupQuery {
        category: "webauthn_ceremonies",
        sql: r"WITH doomed AS (
                   SELECT id FROM webauthn_ceremonies
                   WHERE expires_at < now() - make_interval(days => $1::int)
                   ORDER BY expires_at
                   LIMIT $2
                   FOR UPDATE SKIP LOCKED
               )
               DELETE FROM webauthn_ceremonies AS target
               USING doomed
               WHERE target.id = doomed.id",
        retention: Retention::Fixed(0),
    },
    CleanupQuery {
        category: "rate_limit_buckets",
        sql: r"WITH doomed AS (
                   SELECT scope_hash, category, window_start FROM rate_limit_buckets
                   WHERE expires_at < now() - make_interval(days => $1::int)
                   ORDER BY expires_at
                   LIMIT $2
                   FOR UPDATE SKIP LOCKED
               )
               DELETE FROM rate_limit_buckets AS target
               USING doomed
               WHERE target.scope_hash = doomed.scope_hash
                 AND target.category = doomed.category
                 AND target.window_start = doomed.window_start",
        retention: Retention::Fixed(0),
    },
    CleanupQuery {
        category: "account_tokens",
        sql: r"WITH doomed AS (
                   SELECT id FROM account_tokens
                   WHERE expires_at < now() - make_interval(days => $1::int)
                      OR consumed_at < now() - make_interval(days => $1::int)
                   ORDER BY COALESCE(consumed_at, expires_at)
                   LIMIT $2
                   FOR UPDATE SKIP LOCKED
               )
               DELETE FROM account_tokens AS target
               USING doomed
               WHERE target.id = doomed.id",
        retention: Retention::Fixed(7),
    },
    CleanupQuery {
        category: "audit_events",
        sql: r"WITH doomed AS (
                   SELECT id FROM audit_events
                   WHERE created_at < now() - make_interval(days => $1::int)
                   ORDER BY created_at
                   LIMIT $2
                   FOR UPDATE SKIP LOCKED
               )
               DELETE FROM audit_events AS target
               USING doomed
               WHERE target.id = doomed.id",
        retention: Retention::Audit,
    },
];

#[derive(Clone, Copy)]
enum Retention {
    Audit,
    Sessions,
    Fixed(i64),
}

struct CleanupQuery {
    category: &'static str,
    sql: &'static str,
    retention: Retention,
}

/// Periodically removes expired security records under a cluster-wide lock.
#[derive(Clone)]
pub struct MaintenanceService {
    database: Database,
    config: MaintenanceConfig,
    metrics: Metrics,
}

impl MaintenanceService {
    pub const fn new(database: Database, config: MaintenanceConfig, metrics: Metrics) -> Self {
        Self {
            database,
            config,
            metrics,
        }
    }

    pub fn spawn(&self) -> JoinHandle<()> {
        let service = self.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(service.config.interval_seconds));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(error) = service.run_once().await {
                    tracing::error!(%error, "database_maintenance_failed");
                }
            }
        })
    }

    /// Run one bounded cleanup pass.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot acquire or execute the
    /// maintenance transaction.
    pub async fn run_once(&self) -> anyhow::Result<u64> {
        match self.cleanup_transaction().await {
            Ok(Some(deleted)) => {
                self.metrics.observe_maintenance_run("success");
                tracing::info!(deleted, "database_maintenance_completed");
                Ok(deleted)
            }
            Ok(None) => {
                self.metrics.observe_maintenance_run("skipped");
                tracing::debug!("database_maintenance_lock_busy");
                Ok(0)
            }
            Err(error) => {
                self.metrics.observe_maintenance_run("error");
                Err(error)
            }
        }
    }

    async fn cleanup_transaction(&self) -> anyhow::Result<Option<u64>> {
        let mut transaction = self.database.pool().begin().await?;
        let acquired = query_scalar::<_, bool>("SELECT pg_try_advisory_xact_lock($1)")
            .bind(ADVISORY_LOCK_ID)
            .fetch_one(&mut *transaction)
            .await?;
        if !acquired {
            return Ok(None);
        }

        let mut total = 0;
        let mut deletion_counts = Vec::with_capacity(CLEANUP_QUERIES.len());
        for cleanup in CLEANUP_QUERIES {
            let retention_days = match cleanup.retention {
                Retention::Audit => self.config.audit_retention_days,
                Retention::Sessions => self.config.session_retention_days,
                Retention::Fixed(days) => days,
            };
            let deleted = delete_in_batches(&mut transaction, cleanup.sql, retention_days)
                .await
                .with_context(|| format!("could not clean {}", cleanup.category))?;
            deletion_counts.push((cleanup.category, deleted));
            total += deleted;
        }
        transaction.commit().await?;
        for (category, deleted) in deletion_counts {
            self.metrics
                .observe_maintenance_deletions(category, deleted);
        }
        Ok(Some(total))
    }
}

async fn delete_in_batches(
    transaction: &mut Transaction<'_, Postgres>,
    sql: &'static str,
    retention_days: i64,
) -> anyhow::Result<u64> {
    let mut total = 0;
    for _ in 0..MAX_BATCHES_PER_CATEGORY {
        let deleted = query(sql)
            .bind(retention_days)
            .bind(BATCH_SIZE)
            .execute(&mut **transaction)
            .await?
            .rows_affected();
        total += deleted;
        if deleted < BATCH_SIZE as u64 {
            break;
        }
    }
    Ok(total)
}
