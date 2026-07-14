use pdf_engine::{PageRange, PdfRect, WatermarkOptions};
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
    Rotate {
        protocol_version: u16,
        request_id: String,
        document: serde_bytes::ByteBuf,
        ranges: Vec<PageRange>,
        angle_degrees: i16,
    },
    Reorder {
        protocol_version: u16,
        request_id: String,
        document: serde_bytes::ByteBuf,
        order: Vec<u32>,
    },
    Crop {
        protocol_version: u16,
        request_id: String,
        document: serde_bytes::ByteBuf,
        ranges: Vec<PageRange>,
        rectangle: PdfRect,
    },
    Watermark {
        protocol_version: u16,
        request_id: String,
        document: serde_bytes::ByteBuf,
        ranges: Vec<PageRange>,
        options: WatermarkOptions,
    },
    ExtractText {
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
            }
            | Self::Rotate {
                protocol_version, ..
            }
            | Self::Reorder {
                protocol_version, ..
            }
            | Self::Crop {
                protocol_version, ..
            }
            | Self::Watermark {
                protocol_version, ..
            }
            | Self::ExtractText {
                protocol_version, ..
            } => *protocol_version,
        }
    }

    pub fn request_id(&self) -> &str {
        match self {
            Self::Merge { request_id, .. }
            | Self::Split { request_id, .. }
            | Self::Rotate { request_id, .. }
            | Self::Reorder { request_id, .. }
            | Self::Crop { request_id, .. }
            | Self::Watermark { request_id, .. }
            | Self::ExtractText { request_id, .. } => request_id,
        }
    }

    pub const fn operation(&self) -> Operation {
        match self {
            Self::Merge { .. } => Operation::Merge,
            Self::Split { .. } => Operation::Split,
            Self::Rotate { .. } => Operation::Rotate,
            Self::Reorder { .. } => Operation::Reorder,
            Self::Crop { .. } => Operation::Crop,
            Self::Watermark { .. } => Operation::Watermark,
            Self::ExtractText { .. } => Operation::ExtractText,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Merge,
    Split,
    Rotate,
    Reorder,
    Crop,
    Watermark,
    ExtractText,
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
        files: Vec<WorkerFile>,
        duration_ms: u64,
    ) -> Self {
        Self::Success {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            operation,
            files,
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
    pub mime_type: String,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

impl WorkerFile {
    pub fn pdf_files(operation: Operation, files: Vec<Vec<u8>>) -> Vec<Self> {
        files
            .into_iter()
            .enumerate()
            .map(|(index, bytes)| Self {
                name: format!("{operation:?}-{}.pdf", index + 1).to_lowercase(),
                mime_type: "application/pdf".to_owned(),
                bytes,
            })
            .collect()
    }

    pub fn extracted_text(text: String) -> Self {
        Self {
            name: "extract-text.txt".to_owned(),
            mime_type: "text/plain;charset=utf-8".to_owned(),
            bytes: text.into_bytes(),
        }
    }
}
