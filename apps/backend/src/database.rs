use std::time::Duration;

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, query};

use crate::Config;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    /// Connect to `PostgreSQL` and optionally apply embedded migrations.
    ///
    /// # Errors
    ///
    /// Returns an error when the pool cannot connect or a migration fails.
    pub async fn connect(config: &Config) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(config.database_max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&config.database_url)
            .await
            .context("could not connect to PostgreSQL")?;
        if config.run_migrations {
            MIGRATOR
                .run(&pool)
                .await
                .context("could not apply database migrations")?;
        }
        Ok(Self { pool })
    }

    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn is_ready(&self) -> bool {
        query("SELECT 1").execute(&self.pool).await.is_ok()
    }
}
