use pdf_engine::PageRange;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum WorkerRequest {
    Merge {
        protocol_version: u16,
        request_id: String,
        documents: Vec<serde_bytes::ByteBuf>,
    },
    Split {
        protocol_version: u16,
        request_id: String,
        document: serde_bytes::ByteBuf,
        ranges: Vec<PageRange>,
    },
}

impl WorkerRequest {
    pub const fn protocol_version(&self) -> u16 {
        match self {
            Self::Merge {
                protocol_version, ..
            }
            | Self::Split {
                protocol_version, ..
            } => *protocol_version,
        }
    }

    pub fn request_id(&self) -> &str {
        match self {
            Self::Merge { request_id, .. } | Self::Split { request_id, .. } => request_id,
        }
    }

    pub const fn operation(&self) -> Operation {
        match self {
            Self::Merge { .. } => Operation::Merge,
            Self::Split { .. } => Operation::Split,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Merge,
    Split,
    Unknown,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorkerResponse {
    Success {
        protocol_version: u16,
        request_id: String,
        operation: Operation,
        files: Vec<WorkerFile>,
        duration_ms: u64,
    },
    Error {
        protocol_version: u16,
        request_id: String,
        operation: Operation,
        code: String,
        message: String,
        duration_ms: u64,
    },
}

impl WorkerResponse {
    pub fn success(
        request_id: String,
        operation: Operation,
        files: Vec<Vec<u8>>,
        duration_ms: u64,
    ) -> Self {
        Self::Success {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            operation,
            files: files
                .into_iter()
                .enumerate()
                .map(|(index, bytes)| WorkerFile {
                    name: format!("{operation:?}-{}.pdf", index + 1).to_lowercase(),
                    bytes,
                })
                .collect(),
            duration_ms,
        }
    }

    pub fn error(
        request_id: String,
        operation: Operation,
        code: impl Into<String>,
        message: String,
        duration_ms: u64,
    ) -> Self {
        Self::Error {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            operation,
            code: code.into(),
            message,
            duration_ms,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct WorkerFile {
    pub name: String,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}
