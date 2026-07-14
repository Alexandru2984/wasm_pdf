mod crypto;
mod error;
mod http;
mod lifecycle;
mod model;
mod passkey;
mod rate_limit;
mod recovery;
mod service;

pub(crate) use error::AuthError;
pub use http::router;
pub(crate) use rate_limit::RateLimitCategory;
pub use service::AuthService;
