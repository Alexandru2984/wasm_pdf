use anyhow::Context;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Install newline-delimited JSON logging for collection by Promtail/Loki.
///
/// # Errors
///
/// Returns an error for an invalid filter or when a global subscriber was
/// already installed by the host process.
pub fn init_tracing(filter: &str) -> anyhow::Result<()> {
    let env_filter = EnvFilter::try_new(filter).context("RUST_LOG contains an invalid filter")?;
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(true),
        )
        .try_init()
        .context("could not install tracing subscriber")?;
    Ok(())
}
