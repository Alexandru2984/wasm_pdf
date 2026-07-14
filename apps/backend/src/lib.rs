//! Native Axum API and platform-neutral service boundaries.

mod auth;
mod config;
mod database;
mod http;
mod metrics;
mod observability;

use std::time::Instant;

pub use auth::AuthService;
pub use config::{AuthConfig, Config, Environment};
pub use database::Database;
pub use metrics::Metrics;
pub use observability::init_tracing;

/// Shared, cheap-to-clone HTTP application state.
#[derive(Clone)]
pub struct AppState {
    pub metrics: Metrics,
    pub started_at: Instant,
    pub database: Option<Database>,
    pub auth: Option<AuthService>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            metrics: Metrics::new(),
            started_at: Instant::now(),
            database: None,
            auth: None,
        }
    }

    pub fn with_database(database: Database) -> Self {
        Self {
            database: Some(database),
            ..Self::new()
        }
    }

    pub fn with_services(database: Database, auth: AuthService) -> Self {
        Self {
            database: Some(database),
            auth: Some(auth),
            ..Self::new()
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Construct the complete HTTP router.
pub fn build_router(state: AppState) -> axum::Router {
    http::router(state)
}
