use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// Stable engine errors suitable for mapping to a worker/API error code.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("at least one PDF document is required")]
    NoDocuments,

    #[error("a merge accepts at most {max} documents")]
    TooManyDocuments { max: usize },

    #[error("operation input exceeds the {max_bytes} byte limit")]
    InputTooLarge { max_bytes: usize },

    #[error("input PDF at index {input_index} is invalid: {reason}")]
    InvalidPdf { input_index: usize, reason: String },

    #[error("input PDF at index {input_index} is encrypted; decrypt it before processing")]
    EncryptedPdf { input_index: usize },

    #[error("input PDF at index {input_index} contains no pages")]
    EmptyPdf { input_index: usize },

    #[error("at least one page range is required")]
    NoPageRanges,

    #[error("page range {start}-{end} is outside the document's 1-{page_count} page bounds")]
    InvalidPageRange {
        start: u32,
        end: u32,
        page_count: u32,
    },

    #[error("rotation angle {angle_degrees} is invalid; use 90, 180, or 270 degrees")]
    InvalidRotation { angle_degrees: i16 },

    #[error("input PDF has an invalid page at number {page_number}: {reason}")]
    InvalidPage { page_number: u32, reason: String },

    #[error("page order is invalid: {reason}")]
    InvalidPageOrder { reason: String },

    #[error("crop rectangle is invalid: {reason}")]
    InvalidRectangle { reason: String },

    #[error("watermark is invalid: {reason}")]
    InvalidWatermark { reason: String },

    #[error("could not write the output PDF: {0}")]
    WriteFailed(String),
}

impl Error {
    /// Machine-readable code kept stable across Rust and JavaScript boundaries.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NoDocuments => "no_documents",
            Self::TooManyDocuments { .. } => "too_many_documents",
            Self::InputTooLarge { .. } => "input_too_large",
            Self::InvalidPdf { .. } => "invalid_pdf",
            Self::EncryptedPdf { .. } => "encrypted_pdf",
            Self::EmptyPdf { .. } => "empty_pdf",
            Self::NoPageRanges => "no_page_ranges",
            Self::InvalidPageRange { .. } => "invalid_page_range",
            Self::InvalidRotation { .. } => "invalid_rotation",
            Self::InvalidPage { .. } => "invalid_page",
            Self::InvalidPageOrder { .. } => "invalid_page_order",
            Self::InvalidRectangle { .. } => "invalid_rectangle",
            Self::InvalidWatermark { .. } => "invalid_watermark",
            Self::WriteFailed(_) => "write_failed",
        }
    }
}
