//! Native Axum API and platform-neutral service boundaries.

mod config;
mod http;
mod metrics;
mod observability;

use std::time::Instant;

pub use config::Config;
pub use metrics::Metrics;
pub use observability::init_tracing;

/// Shared, cheap-to-clone HTTP application state.
#[derive(Clone)]
pub struct AppState {
    pub metrics: Metrics,
    pub started_at: Instant,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            metrics: Metrics::new(),
            started_at: Instant::now(),
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
