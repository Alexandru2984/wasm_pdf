use std::time::Duration;

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, query, query_scalar};

use crate::{Config, DatabaseConfig, RuntimeDatabaseRole};

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
        Self::connect_with(&DatabaseConfig {
            url: config.database_url.clone(),
            max_connections: config.database_max_connections,
            run_migrations: config.run_migrations,
        })
        .await
    }

    /// Connect using the minimal settings accepted by one-shot maintenance
    /// commands.
    ///
    /// # Errors
    ///
    /// Returns an error when the pool cannot connect or a migration fails.
    pub async fn connect_with(config: &DatabaseConfig) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&config.url)
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

    /// Create or rotate the password of the unprivileged application role and
    /// grant only the DML privileges needed by the runtime backend.
    ///
    /// # Errors
    ///
    /// Returns an error when the connected migration owner cannot provision
    /// the role or apply grants.
    pub async fn provision_runtime_role(&self, role: &RuntimeDatabaseRole) -> anyhow::Result<()> {
        let mut transaction = self.pool.begin().await?;
        query("SELECT set_config('wasm_pdf.runtime_role', $1, true)")
            .bind(&role.name)
            .execute(&mut *transaction)
            .await?;
        query("SELECT set_config('wasm_pdf.runtime_password', $1, true)")
            .bind(&role.password)
            .execute(&mut *transaction)
            .await?;
        query(
            r"DO $provision$
               DECLARE
                   runtime_role text := current_setting('wasm_pdf.runtime_role');
                   runtime_password text := current_setting('wasm_pdf.runtime_password');
               BEGIN
                   IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = runtime_role) THEN
                       EXECUTE format(
                           'CREATE ROLE %I LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS CONNECTION LIMIT 50',
                           runtime_role
                       );
                   END IF;
                   EXECUTE format(
                       'ALTER ROLE %I WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS CONNECTION LIMIT 50 PASSWORD %L',
                       runtime_role,
                       runtime_password
                   );
                   EXECUTE format('GRANT CONNECT ON DATABASE %I TO %I', current_database(), runtime_role);
                   EXECUTE format('GRANT USAGE ON SCHEMA public TO %I', runtime_role);
                   EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO %I', runtime_role);
                   EXECUTE format('GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO %I', runtime_role);
                   EXECUTE format('ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO %I', runtime_role);
                   EXECUTE format('ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT ON SEQUENCES TO %I', runtime_role);
                   EXECUTE format('ALTER ROLE %I SET statement_timeout = %L', runtime_role, '30s');
                   EXECUTE format('ALTER ROLE %I SET idle_in_transaction_session_timeout = %L', runtime_role, '30s');
               END
               $provision$",
        )
        .execute(&mut *transaction)
        .await?;
        query("REVOKE CREATE ON SCHEMA public FROM PUBLIC")
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;

        let is_unprivileged = query_scalar::<_, bool>(
            r"SELECT NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole
                      AND NOT rolreplication AND NOT rolbypassrls
               FROM pg_roles WHERE rolname = $1",
        )
        .bind(&role.name)
        .fetch_one(&self.pool)
        .await?;
        anyhow::ensure!(
            is_unprivileged,
            "runtime database role is unexpectedly privileged"
        );
        Ok(())
    }
}
