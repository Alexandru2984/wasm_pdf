use std::{fmt, sync::Arc};

use prometheus_client::encoding::{EncodeLabelSet, text::encode};
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::histogram::Histogram;
use prometheus_client::registry::Registry;

#[derive(Clone, Debug, EncodeLabelSet, Eq, Hash, PartialEq)]
struct HttpRequestLabels {
    method: String,
    route: String,
    status: String,
}

#[derive(Clone, Debug, EncodeLabelSet, Eq, Hash, PartialEq)]
struct HttpDurationLabels {
    method: String,
    route: String,
}

#[derive(Clone, Debug, EncodeLabelSet, Eq, Hash, PartialEq)]
struct PdfOperationLabels {
    operation: String,
    status: String,
}

#[derive(Clone, Debug, EncodeLabelSet, Eq, Hash, PartialEq)]
struct PdfDurationLabels {
    operation: String,
}

#[derive(Clone, Debug, EncodeLabelSet, Eq, Hash, PartialEq)]
struct EmailDeliveryLabels {
    status: String,
}

/// Application metrics and their immutable `OpenMetrics` registry.
#[derive(Clone)]
pub struct Metrics {
    registry: Arc<Registry>,
    http_requests: Family<HttpRequestLabels, Counter>,
    http_duration: Family<HttpDurationLabels, Histogram>,
    pdf_operations: Family<PdfOperationLabels, Counter>,
    pdf_duration: Family<PdfDurationLabels, Histogram>,
    email_deliveries: Family<EmailDeliveryLabels, Counter>,
}

impl Metrics {
    pub fn new() -> Self {
        let http_requests = Family::<HttpRequestLabels, Counter>::default();
        let http_duration = Family::<HttpDurationLabels, Histogram>::new_with_constructor(|| {
            Histogram::new([0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0])
        });
        let pdf_operations = Family::<PdfOperationLabels, Counter>::default();
        let pdf_duration = Family::<PdfDurationLabels, Histogram>::new_with_constructor(|| {
            Histogram::new([0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0])
        });
        let email_deliveries = Family::<EmailDeliveryLabels, Counter>::default();

        let mut registry = Registry::default();
        registry.register(
            "pdf_editor_http_requests",
            "Total HTTP requests handled by route, method and status",
            http_requests.clone(),
        );
        registry.register(
            "pdf_editor_http_request_duration_seconds",
            "HTTP request latency in seconds",
            http_duration.clone(),
        );
        registry.register(
            "pdf_editor_operations",
            "Client-side PDF operations by operation and outcome",
            pdf_operations.clone(),
        );
        registry.register(
            "pdf_editor_operation_duration_seconds",
            "Client-side PDF operation processing time in seconds",
            pdf_duration.clone(),
        );
        registry.register(
            "pdf_editor_email_deliveries",
            "Email outbox delivery attempts by bounded outcome",
            email_deliveries.clone(),
        );

        Self {
            registry: Arc::new(registry),
            http_requests,
            http_duration,
            pdf_operations,
            pdf_duration,
            email_deliveries,
        }
    }

    pub fn observe_http(&self, method: &str, route: &str, status: u16, duration_seconds: f64) {
        self.http_requests
            .get_or_create(&HttpRequestLabels {
                method: method.to_owned(),
                route: route.to_owned(),
                status: status.to_string(),
            })
            .inc();
        self.http_duration
            .get_or_create(&HttpDurationLabels {
                method: method.to_owned(),
                route: route.to_owned(),
            })
            .observe(duration_seconds);
    }

    pub fn observe_pdf_operation(&self, operation: &str, status: &str, duration_seconds: f64) {
        self.pdf_operations
            .get_or_create(&PdfOperationLabels {
                operation: operation.to_owned(),
                status: status.to_owned(),
            })
            .inc();
        self.pdf_duration
            .get_or_create(&PdfDurationLabels {
                operation: operation.to_owned(),
            })
            .observe(duration_seconds);
    }

    pub fn observe_email_delivery(&self, status: &str) {
        debug_assert!(matches!(status, "sent" | "retry" | "dead"));
        self.email_deliveries
            .get_or_create(&EmailDeliveryLabels {
                status: status.to_owned(),
            })
            .inc();
    }

    /// Encode all metrics using the `OpenMetrics` text format.
    ///
    /// # Errors
    ///
    /// Returns an encoding error if a registered collector cannot be rendered.
    pub fn encode(&self) -> Result<String, fmt::Error> {
        let mut body = String::new();
        encode(&mut body, &self.registry)?;
        Ok(body)
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}
